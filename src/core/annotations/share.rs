//! Self-contained shares contain only approved PNGs and reading text.
use super::{media::ApprovedCapture, note, notebook::Document};
use crate::core::{file_reads, store_io};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn text(s: &str) -> String {
    let url = regex::Regex::new(r#"(?i)\bhttps?://[^\s<>\"']+"#).unwrap();
    let path =
        regex::Regex::new(r#"(?i)(?:[a-z]:[\\/]|\\\\)[^\r\n<>\"']+|(?:^|\s)/[^\s<>\"']+"#).unwrap();
    escape(&path.replace_all(
        &url.replace_all(s, "[link omitted]"),
        "[local file omitted]",
    ))
}
pub fn default_name(title: &str) -> String {
    let name: String = title
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .take(80)
        .collect();
    let name = name.trim().trim_end_matches(['.', ' ']);
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty()
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    {
        return "annotation-note.html".into();
    }
    format!("{name}.html")
}
const CSS: &str = r#"
:root{color-scheme:light dark;--paper:#fcfbf8;--ink:#262522;--soft:#66645e;--line:#deddd7;--accent:#315f89;--card:#fff}*{box-sizing:border-box}body{overflow-wrap:anywhere;margin:0;background:var(--paper);color:var(--ink);font:16px/1.65 'Segoe UI',system-ui,sans-serif}main{max-width:1180px;margin:auto;padding:48px 24px 32px}header{max-width:820px;margin-bottom:36px}h1{font-size:clamp(28px,4vw,40px);line-height:1.18;letter-spacing:-.025em;margin:0 0 20px}h2{font-size:24px;line-height:1.3;margin:36px 0 18px}h3{font-size:17px;margin:0}p{margin:0 0 16px;white-space:pre-line}section{margin:24px 0 40px}.figure{display:grid;grid-template-columns:minmax(0,2fr) minmax(250px,1fr);gap:24px;align-items:start;margin:24px 0}figure{margin:0;min-width:0}.image-wrap{position:relative}.image-wrap svg{position:absolute;inset:0;width:100%;height:100%;pointer-events:none}.reuse{color:var(--soft);font-size:14px}img{display:block;width:100%;height:auto;border-radius:6px}figcaption{color:var(--soft);font-size:13px;margin-top:10px}.callout{border-top:1px solid var(--line);padding:14px 0}.callout:first-child{padding-top:0;border:0}.callout h3{display:flex;align-items:center;gap:10px;margin-bottom:8px}.number{display:inline-flex;min-width:26px;height:26px;padding:0 6px;align-items:center;justify-content:center;background:var(--accent);color:var(--card);border-radius:50%;font-size:14px}.callout p{margin:0;font-size:15px}li{margin:10px 0}footer{border-top:1px solid var(--line);padding:20px 0;color:var(--soft);font-size:12px}@media(prefers-color-scheme:dark){:root{--paper:#1c1d1b;--ink:#eeeee9;--soft:#bbbcb3;--line:#42443d;--accent:#99bfdf;--card:#1c1d1b}}@media(max-width:700px){main{padding:28px 18px}.figure{grid-template-columns:1fr;gap:18px}h2{font-size:22px}}@media print{:root{color-scheme:light;--paper:#fff;--ink:#111;--soft:#555;--line:#bbb;--accent:#315f89;--card:#fff}main{max-width:none;padding:0}.figure{break-inside:avoid}h2,h3{break-after:avoid}img{print-color-adjust:exact}footer{font-size:10px}@page{margin:14mm}}
"#;
fn header(title: &str, summary: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>{}</title><style>{CSS}</style></head><body><main><header><h1>{}</h1><p>{}</p></header>",
        text(title),
        text(title),
        text(summary)
    )
}
fn end(out: &mut String, value: &Value) {
    out.push_str("<section><h2>Open questions</h2><ul>");
    for q in super::notebook::merged_questions(value) {
        out.push_str(&format!("<li>{}</li>", text(q.as_str().unwrap())));
    }
    out.push_str("</ul><h2>To-dos</h2><ul>");
    for t in value["todos"].as_array().unwrap() {
        out.push_str(&format!("<li>{}", text(t["action"].as_str().unwrap())));
        for k in ["owner", "due_date"] {
            if let Some(s) = t[k].as_str() {
                out.push_str(&format!(" ({})", text(s)));
            }
        }
        out.push_str("</li>");
    }
    out.push_str("</ul></section><footer>Made with Sidenote</footer></main></body></html>");
}
fn figures(figures: &Value, approved: &ApprovedCapture) -> Result<String, String> {
    approved.verify().map_err(|e| e.to_string())?;
    let frames = figures.as_array().ok_or("Invalid note figures")?;
    let mut paths = Vec::new();
    for f in frames {
        if f["same_screen"].is_string() {
            continue;
        }
        let frame = approved
            .approval
            .frames
            .iter()
            .find(|frame| Some(frame.index) == f["keyframe_index"].as_u64())
            .ok_or("A shared frame was not approved.")?;
        paths.push(approved.folder.join(&frame.marked));
    }
    let _pins = approved.pin_images(&paths).map_err(|e| e.to_string())?;
    let mut out = String::new();
    let mut paths = paths.into_iter();
    for f in frames {
        if let Some(reference) = f["same_screen"].as_str() {
            out.push_str(&format!("<p class=\"reuse\">{}</p>", text(reference)));
            for explanation in f["reuse_explanations"].as_array().unwrap() {
                out.push_str(&format!("<p>{}</p>", text(explanation.as_str().unwrap())));
            }
            continue;
        }
        let path = paths.next().unwrap();
        let bytes = file_reads::read(&path, super::IMAGE_MAX_BYTES).map_err(|e| e.to_string())?;
        let expected = &approved.approval.file_hashes[path.file_name().unwrap().to_str().unwrap()];
        if blake3::hash(&bytes).to_hex().as_str() != expected {
            return Err("A shared image differs from its approval.".into());
        }
        out.push_str(&format!("<div class=\"figure\"><figure><div class=\"image-wrap\"><img alt=\"{}\" src=\"data:image/png;base64,{}\">{}</div><figcaption>{}</figcaption></figure><div>",text(f["caption"].as_str().unwrap()),STANDARD.encode(&bytes), overlays(f, &bytes)?, text(f["caption"].as_str().unwrap())));
        for c in f["callouts"].as_array().unwrap() {
            out.push_str(&format!("<article class=\"callout\"><h3><span class=\"number\">{}</span>{}</h3><p>{}</p></article>",c["mark_number"].as_u64().unwrap(),text(c["label"].as_str().unwrap()),text(c["explanation"].as_str().unwrap())));
        }
        out.push_str("</div></div>");
    }
    Ok(out)
}
fn overlays(figure: &Value, bytes: &[u8]) -> Result<String, String> {
    let image = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = image.into_dimensions().map_err(|e| e.to_string())?;
    let mut svg = String::new();
    for c in figure["callouts"].as_array().unwrap() {
        let Some(old) = c["original_mark_number"].as_u64() else {
            continue;
        };
        let n = c["mark_number"].as_u64().unwrap();
        if old == n {
            continue;
        }
        if let Some(b) = c["boxNorm"].as_array() {
            let bw = (old.to_string().len().max(n.to_string().len()) as f64 * 12.0 + 6.0).max(22.0);
            let x = (b[0].as_f64().unwrap() * w as f64)
                .floor()
                .clamp(0.0, (w as f64 - bw).max(0.0));
            let y = (b[1].as_f64().unwrap() * h as f64)
                .floor()
                .clamp(0.0, (h as f64 - 22.0).max(0.0));
            svg.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"{bw}\" height=\"22\" fill=\"#1e4a74\"/><text x=\"{}\" y=\"{}\" fill=\"white\" font-size=\"16\" font-family=\"sans-serif\" text-anchor=\"middle\">{n}</text>",x+bw/2.0,y+17.0));
        }
    }
    Ok(if svg.is_empty() {
        svg
    } else {
        format!("<svg viewBox=\"0 0 {w} {h}\" aria-hidden=\"true\">{svg}</svg>")
    })
}
pub fn capture(value: &Value, approved: &ApprovedCapture) -> Result<String, String> {
    note::validate(value, approved)?;
    let mut out = header(
        value["title"].as_str().unwrap(),
        value["summary"].as_str().unwrap(),
    );
    for section in value["sections"].as_array().unwrap() {
        out.push_str(&format!(
            "<section><h2>{}</h2>",
            text(section["heading"].as_str().unwrap())
        ));
        for p in section["paragraphs"].as_array().unwrap() {
            out.push_str(&format!("<p>{}</p>", text(p.as_str().unwrap())));
        }
        let fs = section["figure_refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| {
                value["figures"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|f| &f["figure_id"] == id)
                    .unwrap()
                    .clone()
            })
            .collect::<Vec<_>>();
        out.push_str(&figures(&Value::Array(fs), approved)?);
        out.push_str("</section>");
    }
    end(&mut out, value);
    Ok(out)
}
pub fn notebook(doc: &Document) -> Result<String, String> {
    super::notebook::validate(doc)?;
    let value = super::notebook::reading_value(doc);
    let v = &value;
    let mut out = header(v["title"].as_str().unwrap(), v["summary"].as_str().unwrap());
    for (section, asset) in v["sections"].as_array().unwrap().iter().zip(&doc.assets) {
        out.push_str(&format!(
            "<section><h2>{}</h2>",
            text(section["heading"].as_str().unwrap())
        ));
        for p in section["paragraphs"].as_array().unwrap() {
            out.push_str(&format!("<p>{}</p>", text(p.as_str().unwrap())));
        }
        out.push_str(&figures(&section["figures"], &asset.approved)?);
        out.push_str("</section>");
    }
    end(&mut out, v);
    Ok(out)
}
pub fn save(path: &Path, html: &str) -> Result<(), String> {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("html"))
    {
        return Err("Choose an .html file name.".into());
    }
    if path.ancestors().any(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("scrawl"))
    }) {
        return Err("Choose a folder outside the Hex capture bundles.".into());
    }
    store_io::atomic_write(path, html.as_bytes())
        .map_err(|_| "Could not save the shared page. Choose another location.".into())
}
pub fn image_hashes(html: &str) -> Result<BTreeMap<String, usize>, String> {
    let re = regex::Regex::new(r#"src=\"data:image/png;base64,([^\"]+)\""#).unwrap();
    let mut hashes = BTreeMap::new();
    for caps in re.captures_iter(html) {
        let bytes = STANDARD.decode(&caps[1]).map_err(|e| e.to_string())?;
        *hashes
            .entry(blake3::hash(&bytes).to_hex().to_string())
            .or_default() += 1;
    }
    Ok(hashes)
}
