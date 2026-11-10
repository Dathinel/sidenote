//! Local capture browsing. No agent or model is called from this page.
use super::{
    Event, State, hex, nav, roles,
    theme::{self, Palette},
    widgets::{link, pressable, scroll},
};
use rune::{El, Image, ImageCache, Key, col, el, row};
use sidenote::{
    core::{
        annotations::{self, Capture, Keyframe, Mark, Scan, Word},
        file_reads,
    },
    hex_engine::HexEngine,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Default)]
pub struct Annotations {
    pub open: bool,
    captures: Vec<Capture>,
    errors: Vec<String>,
    selected: Option<PathBuf>,
    transcript: bool,
    loading: bool,
    images: Vec<Result<Image, String>>,
    shortcut: Option<String>,
    disabled: bool,
}
pub enum Msg {
    Scanned {
        scan: Scan,
        shortcut: Option<String>,
        disabled: bool,
    },
    Images {
        folder: PathBuf,
        images: Vec<Result<Image, String>>,
    },
}

fn images(capture: &Capture, wake: rune::Waker, wait: bool) -> Vec<Result<Image, String>> {
    let cache = ImageCache::new(wake);
    capture
        .data
        .keyframes
        .iter()
        .map(|frame| {
            let path = capture.frame_path(frame).map_err(|e| e.to_string())?;
            let bytes = file_reads::read(&path, annotations::IMAGE_MAX_BYTES)
                .map_err(|_| "Could not read this frame image.".to_owned())?;
            let image = cache.load(bytes);
            if wait {
                image
                    .wait()
                    .map_err(|_| "Could not decode this frame image.".to_owned())?;
            }
            Ok(image)
        })
        .collect()
}
fn hex_hints(settings: &[sidenote::hex_engine::Setting]) -> (Option<String>, bool) {
    let shortcut = settings.iter().filter(|s| !s.secret).find_map(|s| {
        let key = s.key.to_lowercase();
        (key.contains("annotate") && (key.contains("shortcut") || key.contains("hotkey")))
            .then(|| {
                s.value
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .map(str::to_owned)
            })
            .flatten()
    });
    let disabled = settings
        .iter()
        .any(|s| s.key == "labs.annotate" && s.value == false);
    (shortcut, disabled)
}
impl State {
    pub fn open_annotations(&mut self) {
        self.go_home();
        self.annotations.open = true;
        self.annotations.selected = None;
        self.annotations.images.clear();
        self.refresh_annotations();
        self.nav_done();
    }
    pub fn refresh_annotations(&mut self) {
        if self.annotations.loading || rune::tooling::headless_run().is_some() {
            return;
        }
        self.annotations.loading = true;
        let sender = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let scan = annotations::captures_dir()
                .map(|p| annotations::scan(&p))
                .unwrap_or_else(|| Scan {
                    captures: Vec::new(),
                    errors: vec!["Could not find the Hex captures folder.".into()],
                });
            let engine = HexEngine::new();
            let settings = engine.settings_list().unwrap_or_default();
            let (shortcut, disabled) = hex_hints(&settings);
            let _ = sender.send(Event::Annotations(Msg::Scanned {
                scan,
                shortcut,
                disabled,
            }));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
    fn select_annotation(&mut self, index: usize) {
        let Some(capture) = self.annotations.captures.get(index).cloned() else {
            return;
        };
        self.annotations.selected = Some(capture.folder.clone());
        self.annotations.transcript = false;
        self.annotations.images.clear();
        let sender = self.events_tx.clone();
        let wake = self.waker.clone().unwrap_or_else(|| Arc::new(|| {}));
        std::thread::spawn(move || {
            let images = images(&capture, wake.clone(), false);
            let _ = sender.send(Event::Annotations(Msg::Images {
                folder: capture.folder,
                images,
            }));
            wake();
        });
    }
    pub fn apply_annotations(&mut self, msg: Msg) {
        match msg {
            Msg::Scanned {
                scan,
                shortcut,
                disabled,
            } => {
                self.annotations.captures = scan.captures;
                self.annotations.errors = scan.errors;
                self.annotations.shortcut = shortcut;
                self.annotations.disabled = disabled;
                self.annotations.loading = false;
                if let Some(folder) = self.annotations.selected.clone() {
                    if let Some(index) = self
                        .annotations
                        .captures
                        .iter()
                        .position(|c| c.folder == folder)
                    {
                        let transcript = self.annotations.transcript;
                        self.select_annotation(index);
                        self.annotations.transcript = transcript;
                    } else {
                        self.annotations.selected = None;
                        self.annotations.images.clear();
                    }
                }
            }
            Msg::Images { folder, images }
                if self.annotations.selected.as_ref() == Some(&folder) =>
            {
                self.annotations.images = images
            }
            Msg::Images { .. } => {}
        }
    }
}

pub fn on_key(s: &mut State, e: &rune::KeyEvent) -> bool {
    if e.key == Key::Escape {
        if s.annotations.selected.take().is_none() {
            s.go_home();
        }
        s.annotations.images.clear();
        return true;
    }
    false
}
fn date(c: &Capture) -> String {
    chrono::DateTime::from_timestamp_millis(c.data.created_at_ms)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Date unavailable".into())
}
fn time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn facts(c: &Capture, p: Palette) -> El {
    roles::meta_line(
        &[
            date(c),
            time(c.data.duration_ms as f64 / 1000.0),
            format!("{} words", c.word_count()),
            format!("{} marks", c.data.marks.len()),
        ],
        p,
    )
}
fn number(n: usize, p: Palette) -> El {
    col()
        .w(24.0)
        .h(24.0)
        .center()
        .radius(12.0)
        .fill(p.accent)
        .child(roles::CAPTION.line(n.to_string(), p.card))
}
fn words_text(words: &[&Word]) -> String {
    words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
fn callout(c: &Capture, n: usize, mark: &Mark, p: Palette) -> El {
    let words = c.words_for_mark(mark);
    let description = mark
        .description
        .as_deref()
        .filter(|d| !d.is_empty())
        .unwrap_or("Mark without a description");
    let mut content = col().w_full().min_w(0.0).gap(7.0).child(
        row()
            .gap(8.0)
            .items_center()
            .child(number(n, p))
            .child(roles::NAME.wrap(description, p.ink).grow(1.0).min_w(0.0)),
    );
    if let (Some(first), Some(last)) = (words.first(), words.last()) {
        content = content
            .child(roles::META.wrap(
                format!("Words nearby, {} to {}", time(first.t0), time(last.t1)),
                p.faint,
            ))
            .child(
                roles::SMALL
                    .wrap(words_text(&words), p.ink)
                    .selectable(true),
            );
    } else {
        content = content.child(roles::SMALL.wrap("No timed words near this mark.", p.soft));
    }
    content
        .key(format!("annotation-callout-{n}"))
        .padding(12.0)
        .radius(8.0)
        .border(1.0, p.line)
        .fill(p.card)
}
fn frame_image(
    s: &State,
    c: &Capture,
    index: usize,
    frame: &Keyframe,
    width: f32,
    p: Palette,
) -> El {
    let image = s.annotations.images.get(index);
    let image = match image {
        Some(Ok(image)) if image.error().is_none() => image,
        Some(Err(error)) => {
            return roles::SMALL
                .wrap(error, p.warn)
                .padding(20.0)
                .w(width)
                .role("status");
        }
        Some(Ok(_)) => {
            return roles::SMALL
                .wrap("Could not decode this frame image.", p.warn)
                .padding(20.0)
                .w(width)
                .role("status");
        }
        None => {
            return roles::SMALL
                .wrap("Loading frame image...", p.soft)
                .padding(20.0)
                .w(width)
                .role("status");
        }
    };
    let Some([w, h]) = image.size() else {
        return roles::SMALL
            .wrap("Loading frame image...", p.soft)
            .padding(20.0)
            .w(width);
    };
    let height = width * h as f32 / w as f32;
    let mut canvas = el()
        .key(format!("annotation-image-{}", frame.index))
        .w(width)
        .h(height)
        .pixels(image.pixels())
        .role("img")
        .label(format!(
            "{}: {}",
            c.app(),
            frame.reason.as_deref().unwrap_or("Capture frame")
        ));
    for (i, mark) in c.data.marks.iter().enumerate() {
        if !c
            .frame_for_mark(mark)
            .is_some_and(|f| f.index == frame.index)
        {
            continue;
        }
        if let Some([x, y, w, h]) = mark.box_norm.filter(|b| b.iter().all(|n| n.is_finite())) {
            let x = x.clamp(0.0, 1.0);
            let y = y.clamp(0.0, 1.0);
            let bw = w.clamp(0.0, 1.0 - x) * width;
            let bh = h.clamp(0.0, 1.0 - y) * height;
            canvas = canvas.child(
                el().key(format!("annotation-box-{}", i + 1))
                    .absolute(x * width, y * height)
                    .w(bw)
                    .h(bh)
                    .border(1.25, p.accent)
                    .radius(3.0),
            );
            canvas = canvas.child(
                number(i + 1, p)
                    .key(format!("annotation-badge-{}", i + 1))
                    .absolute(
                        (x * width).min((width - 24.0).max(0.0)),
                        (y * height).min((height - 24.0).max(0.0)),
                    )
                    .label(format!("Mark {}", i + 1)),
            );
        }
    }
    canvas
}
fn capture_page(s: &State, c: &Capture, width: f32, p: Palette) -> El {
    let mut page = col()
        .w_full()
        .gap(14.0)
        .child(
            link("\u{2039}  All annotations", p)
                .key("annotations-back")
                .on_press(|s: &mut State| {
                    s.annotations.selected = None;
                    s.annotations.images.clear();
                }),
        )
        .child(roles::GROUP.wrap(c.app(), p.soft))
        .child(roles::SECTION.wrap(c.title(), p.ink).max_lines(2))
        .child(facts(c, p));
    let folder = c.folder.clone();
    page = page.child(link("Show files", p).key("annotation-show-files").on_press(
        move |s: &mut State| {
            if rune::tooling::headless_run().is_some() {
                return;
            }
            if sidenote::platform::open_folder(&folder).is_err() {
                s.annotations
                    .errors
                    .push("Could not open the capture folder.".into());
            }
        },
    ));
    let mut tabs = row().gap(8.0).role("tablist").label("Capture views");
    for (transcript, label, key) in [
        (false, "Frames", "annotation-frames-tab"),
        (true, "Transcript", "annotation-transcript-tab"),
    ] {
        let active = s.annotations.transcript == transcript;
        tabs = tabs.child(
            pressable(roles::CONTROL.wrap(label, if active { p.ink } else { p.soft }))
                .key(key)
                .role("tab")
                .label(label)
                .checked(active)
                .px(12.0)
                .py(8.0)
                .radius(6.0)
                .fill(if active { p.sel } else { p.bg })
                .on_press(move |s: &mut State| s.annotations.transcript = transcript),
        );
    }
    page = page.child(tabs);
    if s.annotations.transcript {
        let transcript = &c.data.transcript;
        let mut text = col()
            .w_full()
            .gap(16.0)
            .key("annotation-transcript")
            .role("tabpanel");
        if !transcript.words.is_empty() {
            for (i, chunk) in transcript.words.chunks(28).enumerate() {
                text = text.child(
                    col()
                        .w_full()
                        .gap(4.0)
                        .child(roles::META.wrap(time(chunk[0].t0), p.faint))
                        .child(
                            roles::READ
                                .wrap(
                                    chunk
                                        .iter()
                                        .map(|w| w.text.as_str())
                                        .collect::<Vec<_>>()
                                        .join(" "),
                                    p.ink,
                                )
                                .key(format!("annotation-transcript-words-{i}"))
                                .selectable(true),
                        ),
                );
            }
        } else if !transcript.text.trim().is_empty() {
            text = text.child(roles::READ.wrap(&transcript.text, p.ink).selectable(true));
        } else if !transcript.segments.is_empty() {
            for segment in &transcript.segments {
                text = text.child(roles::READ.wrap(&segment.text, p.ink).selectable(true));
            }
        } else {
            text = text.child(roles::SMALL.wrap("This capture has no transcript yet.", p.soft));
        }
        return page.child(hex::settings::panel(p).child(text.padding(18.0)));
    }
    let side_by_side = width >= 860.0;
    let image_width = if side_by_side {
        width - 300.0 - 20.0
    } else {
        width
    };
    for (index, frame) in c.data.keyframes.iter().enumerate() {
        let reason = frame.reason.as_deref().unwrap_or("Captured frame");
        let mut callouts = col()
            .w(if side_by_side { 300.0 } else { width })
            .min_w(0.0)
            .gap(10.0);
        for (i, mark) in c.data.marks.iter().enumerate() {
            if c.frame_for_mark(mark)
                .is_some_and(|f| f.index == frame.index)
            {
                callouts = callouts.child(callout(c, i + 1, mark, p));
            }
        }
        let visual = frame_image(s, c, index, frame, image_width, p);
        let body = if side_by_side {
            row().w_full().gap(20.0).child(visual).child(callouts)
        } else {
            col().w_full().gap(12.0).child(visual).child(callouts)
        };
        page = page.child(
            col()
                .w_full()
                .gap(10.0)
                .mt(8.0)
                .key(format!("annotation-frame-{}", frame.index))
                .child(roles::GROUP.wrap(
                    format!("Frame {}  {}  {}", index + 1, roles::DOT, reason),
                    p.soft,
                ))
                .child(body),
        );
    }
    if c.data.keyframes.is_empty() {
        page = page.child(roles::SMALL.wrap("This capture has no keyframes yet.", p.soft));
    }
    let missing: Vec<_> = c
        .data
        .marks
        .iter()
        .enumerate()
        .filter(|(_, m)| c.frame_for_mark(m).is_none())
        .collect();
    if !missing.is_empty() {
        page = page.child(roles::GROUP.wrap("Marks without a matching frame", p.soft));
        for (i, mark) in missing {
            page = page.child(callout(c, i + 1, mark, p));
        }
    }
    page
}
pub fn pane(s: &State, p: Palette, narrow: bool) -> El {
    let width = (s.app_w.get() - nav::width(s) - 1.0 - if narrow { 32.0 } else { 64.0 })
        .clamp(240.0, 1040.0);
    let mut content = if let Some(c) = s
        .annotations
        .captures
        .iter()
        .find(|c| Some(&c.folder) == s.annotations.selected.as_ref())
    {
        capture_page(s, c, width, p)
    } else {
        let mut list = col()
            .w_full()
            .gap(10.0)
            .child(
                row()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        roles::SECTION
                            .wrap("Annotations", p.ink)
                            .font(theme::display(28.0)),
                    )
                    .child(
                        link(
                            if s.annotations.loading {
                                "Refreshing..."
                            } else {
                                "Refresh"
                            },
                            p,
                        )
                        .key("annotations-refresh")
                        .on_press(|s: &mut State| s.refresh_annotations()),
                    ),
            )
            .child(roles::SMALL.wrap("Screen captures and the words around each mark.", p.soft))
            .child(roles::META.wrap("Local on this PC", p.faint));
        if s.annotations.captures.is_empty() {
            let title = if s.annotations.loading {
                "Looking for captures..."
            } else if s.annotations.disabled {
                "Hex Annotate is turned off"
            } else {
                "No annotations yet"
            };
            let hint = if let Some(shortcut) = &s.annotations.shortcut {
                format!("In Hex, turn on Annotate and press {shortcut} to draw and talk.")
            } else {
                "In Hex, turn on Annotate, then record your screen while you draw and talk.".into()
            };
            list = list.child(hex::settings::card(
                title,
                col()
                    .gap(12.0)
                    .padding(22.0)
                    .child(super::icons::draw(&super::icons::ANNOTATIONS, 28.0, p.soft))
                    .child(roles::SMALL.wrap(hint, p.soft)),
                p,
            ));
        } else {
            let mut rows = col().w_full();
            for (index, c) in s.annotations.captures.iter().enumerate() {
                rows = rows.child(
                    pressable(
                        col()
                            .w_full()
                            .min_w(0.0)
                            .gap(5.0)
                            .px(16.0)
                            .py(14.0)
                            .child(roles::META.wrap(c.app(), p.soft))
                            .child(roles::TITLE.wrap(c.title(), p.ink).max_lines(2))
                            .child(facts(c, p)),
                    )
                    .key(format!("annotation-row-{index}"))
                    .label(format!("{}: {}", c.app(), c.title()))
                    .radius(9.0)
                    .hover(p.chip)
                    .on_press(move |s: &mut State| s.select_annotation(index)),
                );
            }
            list = list.child(hex::settings::card("Captures, newest first", rows, p));
        }
        list
    };
    for error in &s.annotations.errors {
        content = content.child(roles::SMALL.wrap(error, p.warn).role("status"));
    }
    scroll(
        col()
            .w_full()
            .items_center()
            .py(20.0)
            .child(content.w(width).pb(40.0)),
        "annotations-scroll",
    )
    .key("annotations-pane")
}

pub fn scene(s: &mut State, name: &str) {
    hex::scene(s, false);
    s.go_home();
    s.annotations.open = true;
    s.annotations.loading = false;
    s.annotations.errors.clear();
    s.annotations.captures = if name == "annotations-empty" {
        Vec::new()
    } else {
        annotations::scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations"),
        )
        .captures
    };
    super::scene_sidebar(s, true);
    if (name.starts_with("annotation-capture") || name == "annotation-transcript")
        && let Some(c) = s.annotations.captures.first()
    {
        s.annotations.selected = Some(c.folder.clone());
        s.annotations.images = images(c, Arc::new(|| {}), true);
        s.annotations.transcript = name == "annotation-transcript";
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hints_use_only_reported_non_secret_annotation_settings() {
        let settings =
            serde_json::from_value::<Vec<sidenote::hex_engine::Setting>>(serde_json::json!([
                {"key":"labs.annotate","value":false,"kind":"boolean","writable":true},
                {"key":"shortcuts.annotate","value":"Ctrl+Alt+A","kind":"string","writable":true},
                {"key":"dictation.shortcut","value":"Ctrl+D","kind":"string","writable":true}
            ]))
            .unwrap();
        assert_eq!(hex_hints(&settings), (Some("Ctrl+Alt+A".into()), true));
        assert_eq!(hex_hints(&[]), (None, false));
        let mut secret = settings[1].clone();
        secret.secret = true;
        assert_eq!(hex_hints(&[secret]), (None, false));
    }
}
