"""Renders one meeting's english-summary.json three ways: the Markdown file, the plain text Hex's
Summary box shows, and a styled HTML page, plus an index page for that meeting only.

Usage: python hex-meeting-render.py <meeting folder> [...]   (writes english-summary.md and the pages)
       python hex-meeting-render.py --all                       (every Hex meeting with a summary)
Pages go to <Documents>/Hex Meetings/Viewer/ (index.html lists them); open them in Helium.
"""
import datetime as dt
import html
import json
import os
import re
import stat
import sys
from pathlib import Path

HEX_MEETINGS = Path(os.environ["APPDATA"]) / "hex" / "meetings"
LINE = re.compile(r"^\[(\d+):(\d+):(\d+)(?:\.(\d+))?\] (\S+) \(([^)]*)\): ?(.*)$")


def no_links(path):
    path = Path(path).expanduser()
    if ".." in path.parts or any(":" in part for part in path.parts[1:]):
        raise ValueError("Unsafe export path")
    path = path.absolute()
    for item in [path, *path.parents]:
        try:
            info = item.lstat()
        except FileNotFoundError:
            continue
        if item.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("Refusing linked or reparse export path")
        if not stat.S_ISDIR(info.st_mode) and info.st_nlink > 1:
            raise ValueError("Refusing hard-linked export file")
    return path.resolve()


def documents_dir():
    # Windows may move Documents (for example into OneDrive); ask the shell instead of guessing.
    try:
        import winreg

        key = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders"
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key) as handle:
            return Path(winreg.QueryValueEx(handle, "Personal")[0])
    except (ImportError, OSError):
        return Path.home() / "Documents"


def export_roots():
    configured = os.environ.get("HEX_EXPORT_ROOTS")
    roots = configured.split(os.pathsep) if configured else [str(documents_dir() / "Hex Meetings")]
    if not roots or any(not root.strip() or not Path(root).is_absolute() for root in roots):
        raise ValueError("Export roots must be absolute paths")
    return [no_links(root) for root in roots]


def safe_export(path):
    if not Path(path).is_absolute():
        raise ValueError("Export path must be absolute")
    target = no_links(path)
    if not any(target != root and target.is_relative_to(root) for root in export_roots()):
        raise ValueError("Export path is outside HEX_EXPORT_ROOTS")
    return target


def viewer_dir():
    # A manifest cannot select another meeting's export directory.
    return safe_export(export_roots()[0] / "Viewer")


def short(stamp):
    """00:14:22 -> 14:22; 01:02:03 stays."""
    return re.sub(r"^00:", "", stamp)


def nice_date(iso):
    try:
        day = dt.date.fromisoformat(iso)
        return f"{day.day} {day:%b}"
    except (TypeError, ValueError):
        return iso or ""


def load(meeting):
    meeting = Path(meeting)
    data = json.loads((meeting / "english-summary.json").read_text(encoding="utf-8"))
    meta = {"id": meeting.name, "title": data.get("title") or meeting.name, "started": None, "minutes": None}
    manifest = meeting / "manifest.json"
    if manifest.exists():
        m = json.loads(manifest.read_text(encoding="utf-8"))
        if m.get("title_source") in ("call", "calendar", "user") and m.get("title"):
            meta["title"] = m["title"]
        if m.get("created_at_ms"):
            meta["started"] = dt.datetime.fromtimestamp(m["created_at_ms"] / 1000)
        if m.get("duration_ms"):
            meta["minutes"] = round(m["duration_ms"] / 60000)
        meta["export"] = m.get("export_path")
    lines = []
    transcript = meeting / "english-transcript.md"
    if transcript.exists():
        for raw in transcript.read_text(encoding="utf-8").splitlines():
            found = LINE.match(raw.strip())
            if found:
                h, mi, s, _, who, lang, text = found.groups()
                lines.append({"at": f"{int(h):02d}:{int(mi):02d}:{int(s):02d}", "who": who, "lang": lang, "text": text.strip()})
            elif raw.strip() and lines and not raw.startswith("#"):
                lines[-1]["text"] += " " + raw.strip()
    # Speech ends long before the recording when a meeting was left running.
    if lines:
        h, mi, s = map(int, lines[-1]["at"].split(":"))
        spoken = round((h * 3600 + mi * 60 + s) / 60) + 1
        if meta["minutes"] is None or meta["minutes"] > spoken + 30:
            meta["minutes"] = spoken
    return data, meta, lines


def points(data):
    return [(topic, p) for topic in data.get("topics", []) for p in topic.get("points", [])]


# ---------- Markdown ----------

def md_stamps(at):
    # The Markdown summary has no transcript to link to, so it shows no timestamps; the meeting page links them.
    return ""


def markdown(data, meta, with_title=True):
    out = []
    if with_title:
        out += [f"# {meta['title']}", ""]
        facts = [x for x in (meta["started"] and meta["started"].strftime("%a %d %b %Y, %H:%M"),
                             meta["minutes"] and f"{meta['minutes']} min", data.get("kind"),
                             ", ".join(data.get("people") or [])) if x]
        if facts:
            out += ["**" + "** · **".join(facts) + "**", ""]
    out += ["## Context", ""] + [f"- {c}" for c in data.get("context", [])] + [""]
    decided = [(t, p) for t, p in points(data) if p.get("type") == "decided"]
    if decided:
        out += ["## Decisions", "", "| # | Decision | Who |", "|---|---|---|"]
        out += [f"| {i} | {cell(p['text'])} | {cell(p.get('who') or '')} |" for i, (_, p) in enumerate(decided, 1)] + [""]
    if data.get("todo"):
        out += ["## To do", "", "| | Task | Owner | By |", "|---|---|---|---|"]
        for item in data["todo"]:
            due = nice_date(item.get("due")) + (" (default)" if item.get("due_default") else "")
            out.append(f"| ☐ | {cell(item['task'])} | {cell(item.get('owner') or 'to confirm')} | {due} |")
        out.append("")
    if data.get("to_settle"):
        out += ["## Still to settle", ""]
        out += [f"- {q['question']}" + (f" *(waits on {q['waits_on']})*" if q.get("waits_on") else "") for q in data["to_settle"]] + [""]
    out += ["## Discussion", ""]
    for topic in data.get("topics", []):
        out += [f"### {topic['title']}", ""]
        for p in topic.get("points", []):
            who = f" *({p['who']})*" if p.get("who") else ""
            kind = p.get("type")
            if kind == "decided":
                out.append(f"- ✓ **Decided:** {p['text']}{who}{md_stamps(p.get('at'))}")
            elif kind == "concern":
                out.append(f"- ⚠ **Concern:** {p['text']}{who}{md_stamps(p.get('at'))}")
            elif kind == "response":
                out.append(f"  - ↳ **Response:** {p['text']}{who}{md_stamps(p.get('at'))}")
            elif kind == "open":
                out.append(f"- ? **Not settled:** {p['text']}{md_stamps(p.get('at'))}")
            else:
                out.append(f"- {p['text']}{md_stamps(p.get('at'))}")
        out.append("")
    if data.get("verify"):
        out += ["## Check before acting", ""] + [f"- {v}" for v in data["verify"]] + [""]
    return "\n".join(out).rstrip() + "\n"


def cell(text):
    return str(text).replace("|", "\\|")


# ---------- Hex Summary box (plain text only) ----------

def hex_text(data):
    def block(title, lines):
        return f"{title.upper()}\n{'━' * min(len(title), 28)}\n\n" + "\n\n".join(lines)
    parts = [block("Context", [f"• {c}" for c in data.get("context", [])])]
    decided = [p for _, p in points(data) if p.get("type") == "decided"]
    if decided:
        parts.append(block("Decisions", [f"✓ {p['text']}" for p in decided]))
    steps = [f"→ {t['task']}\n   {t.get('owner') or 'Owner to confirm'}, by {nice_date(t.get('due'))}" for t in data.get("todo", [])]
    steps += [f"? {q['question']}" + (f"\n   Waits on {q['waits_on']}" if q.get("waits_on") else "") for q in data.get("to_settle", [])]
    if steps:
        parts.append(block("Next steps", steps))
    story = []
    for topic in data.get("topics", []):
        story.append(f"◆ {topic['title']}")
        for p in topic.get("points", []):
            mark = {"decided": "✓ ", "concern": "! ", "response": "   ↳ ", "open": "? "}.get(p.get("type"), "• ")
            story.append(mark + p["text"])
    parts.append(block("Discussion", story))
    if data.get("verify"):
        parts.append(block("Check before acting", [f"• {v}" for v in data["verify"]]))
    return "\n\n\n".join(parts)


# ---------- HTML page ----------

CSS = """
:root{--bg:#f7f6f2;--card:#ffffff;--ink:#1e1e1c;--soft:#5d5b55;--faint:#8f8c84;--line:#e8e5dd;--accent:#2f5d8a;
--ok:#1d7a4c;--okbg:#e4f3eb;--warn:#a35d00;--warnbg:#fcf0dc;--open:#6a4ea3;--openbg:#efe9fa;--chip:#efede7;--hit:#fff3c4}
@media (prefers-color-scheme:dark){:root{--bg:#141413;--card:#1c1c1a;--ink:#e9e7e1;--soft:#b0ada4;--faint:#7f7c75;--line:#2e2d2a;
--accent:#8db5dc;--ok:#69c995;--okbg:#1b2c22;--warn:#e3aa55;--warnbg:#33281a;--open:#b49ee3;--openbg:#272138;--chip:#262624;--hit:#3a3317}}
*{box-sizing:border-box}html{-webkit-text-size-adjust:100%;scroll-behavior:smooth}
body{margin:0;background:var(--bg);color:var(--ink);font:15.5px/1.65 "Segoe UI Variable Text","Segoe UI",system-ui,sans-serif}
a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}
.shell{display:grid;grid-template-columns:minmax(0,740px);justify-content:center;gap:56px;padding:40px 24px 96px}
nav.toc,aside.side{display:none}
@media (min-width:1120px){.shell{grid-template-columns:minmax(0,720px) 310px;gap:40px}
aside.side{display:block;position:sticky;top:32px;align-self:start;max-height:calc(100vh - 64px);overflow:auto}
.todo-inline{display:none}}
@media (min-width:1500px){.shell{grid-template-columns:200px minmax(0,720px) 310px}
nav.toc{display:block;position:sticky;top:32px;align-self:start;font-size:13.5px;max-height:calc(100vh - 64px);overflow:auto}
nav.toc a{display:block;color:var(--soft);padding:5px 10px;border-left:2px solid var(--line);line-height:1.35}
nav.toc a:hover{color:var(--ink);border-color:var(--accent);text-decoration:none}nav.toc .sub{padding-left:22px;font-size:13px;color:var(--faint)}
nav.toc .label{font-size:11px;letter-spacing:.09em;text-transform:uppercase;color:var(--faint);margin:0 0 8px 12px}}
.back{font-size:13.5px;color:var(--faint)}
h1{font:650 28px/1.25 "Segoe UI Variable Display","Segoe UI",system-ui,sans-serif;margin:10px 0 12px;letter-spacing:-.01em}
.meta{display:flex;flex-wrap:wrap;gap:6px;margin:0 0 26px}
.chip{background:var(--chip);color:var(--soft);border-radius:999px;padding:2px 11px;font-size:13px}
.glance{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin:0 0 8px}
.glance a{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:12px 14px;color:var(--ink)}
.glance a:hover{text-decoration:none;border-color:var(--accent)}
.glance b{display:block;font-size:24px;line-height:1.1;font-weight:650}.glance span{color:var(--soft);font-size:13px}
.glance .g-ok b{color:var(--ok)}.glance .g-todo b{color:var(--accent)}.glance .g-open b{color:var(--open)}
h2{font-size:12px;letter-spacing:.09em;text-transform:uppercase;color:var(--faint);margin:38px 0 10px;font-weight:650;scroll-margin-top:24px}
.card{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:8px 22px}
ul{margin:0;padding:0;list-style:none}
.rows>li{padding:10px 0;border-bottom:1px solid var(--line)}.rows>li:last-child{border-bottom:0}
.row{display:grid;grid-template-columns:22px minmax(0,1fr);gap:12px;align-items:start}
.ic{width:20px;height:20px;border-radius:50%;display:grid;place-items:center;font-size:11.5px;font-weight:800;margin-top:2px;line-height:1}
.ic.fact:before{content:"";width:6px;height:6px;border-radius:50%;background:var(--faint)}
.ic.decided{background:var(--okbg);color:var(--ok)}.ic.concern{background:var(--warnbg);color:var(--warn)}
.ic.response{color:var(--warn);font-size:14px;font-weight:600}.ic.open{background:var(--openbg);color:var(--open)}
.ic.num{background:var(--okbg);color:var(--ok);font-size:12px}
.who{color:var(--faint);font-size:13.5px;white-space:nowrap}.who:before{content:"— ";content:"· "}
.at{font:12px/1 ui-monospace,"Cascadia Mono",Consolas,monospace;color:var(--faint);margin-left:8px;white-space:nowrap;padding:2px 4px;border-radius:4px}
a.at:hover{color:var(--accent);background:var(--chip);text-decoration:none}
table{width:100%;border-collapse:collapse}
td,th{text-align:left;padding:11px 6px;border-bottom:1px solid var(--line);vertical-align:top}
th{font-size:11.5px;color:var(--faint);font-weight:650;text-transform:uppercase;letter-spacing:.06em}
tr:last-child td{border-bottom:0}td.box{width:26px}td.own{white-space:nowrap;color:var(--soft)}td.due{white-space:nowrap;color:var(--soft)}
.def{display:block;color:var(--faint);font-size:11.5px}
input[type=checkbox]{width:16px;height:16px;accent-color:var(--ok);margin-top:4px}
tr.done td:not(.box){text-decoration:line-through;color:var(--faint)}
.topic{margin:0 0 14px;padding:4px 22px 10px;scroll-margin-top:24px}.topic h3{font-size:17px;font-weight:650;margin:0;padding:14px 0 6px;line-height:1.35}
.pts .row{padding:7px 0}.pts .row.response{margin-left:34px;padding-top:0}
.pts .row.decided .tx{font-weight:600}
.pts .row.open .tx{color:var(--soft)}
.verify .ic{background:var(--warnbg);color:var(--warn)}
details summary{cursor:pointer;color:var(--accent);font-weight:600;padding:6px 0;list-style:none}
details summary::-webkit-details-marker{display:none}details summary:before{content:"▸ "}details[open] summary:before{content:"▾ "}
.filter{width:100%;padding:9px 12px;border:1px solid var(--line);border-radius:8px;background:var(--card);color:var(--ink);font:inherit;margin:8px 0 10px}
.line{display:grid;grid-template-columns:52px 84px minmax(0,1fr);gap:10px;padding:7px 8px;border-radius:6px;font-size:15px;scroll-margin-top:35vh;transition:background .5s}
.line .t{font:12px ui-monospace,"Cascadia Mono",Consolas,monospace;color:var(--faint);padding-top:4px}
.line .s{font-weight:600;color:var(--soft);font-size:13.5px;padding-top:1px;overflow:hidden;text-overflow:ellipsis}
.line.you .s{color:var(--accent)}.line.hit{background:var(--hit)}.line.near{background:var(--chip)}
.from{background:var(--hit);border-radius:8px}
.side-card{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:14px 16px 6px}
.side-h{display:flex;align-items:center;justify-content:space-between;font-size:12px;letter-spacing:.09em;text-transform:uppercase;color:var(--faint);font-weight:650;margin:0 0 6px}
.side-h .count{background:var(--chip);color:var(--accent);border-radius:999px;padding:1px 9px;letter-spacing:0;font-size:12px}
.todo-list li{padding:10px 0;border-bottom:1px solid var(--line)}.todo-list li:last-child{border-bottom:0}
.todo-list label{display:grid;grid-template-columns:20px minmax(0,1fr);gap:10px;cursor:pointer;font-size:14.5px;line-height:1.5}
.todo-list input{margin-top:3px}.todo-list .tm{margin:3px 0 0 30px;font-size:12.5px;color:var(--faint)}
.todo-list .tm b{color:var(--soft);font-weight:600}.todo-list .tm .at{margin-left:6px}
.todo-list li.done .tt{text-decoration:line-through;color:var(--faint)}
.side-note{font-size:12px;color:var(--faint);margin:10px 2px 0}
.ask{position:sticky;bottom:16px;margin-top:40px;z-index:5}
.ask form{display:flex;gap:8px;background:var(--card);border:1px solid var(--line);border-radius:14px;padding:6px 6px 6px 16px;box-shadow:0 6px 24px rgba(0,0,0,.18)}
.ask form:focus-within{border-color:var(--accent)}
.ask input{flex:1;border:0;outline:0;background:none;color:var(--ink);font:inherit;font-size:15px;min-width:0}
.ask form button{border:0;border-radius:10px;background:var(--accent);color:var(--card);font:600 14px "Segoe UI",system-ui,sans-serif;padding:8px 16px;cursor:pointer}
.ask-panel{display:none;background:var(--card);border:1px solid var(--line);border-radius:14px;margin-bottom:8px;box-shadow:0 6px 24px rgba(0,0,0,.18)}
.ask.open .ask-panel{display:block}
.ask-h{display:flex;justify-content:space-between;align-items:center;padding:10px 14px;border-bottom:1px solid var(--line);font-size:12px;letter-spacing:.09em;text-transform:uppercase;color:var(--faint);font-weight:650}
.ask-h button{border:0;background:none;color:var(--soft);font:inherit;letter-spacing:0;text-transform:none;cursor:pointer;margin-left:8px}
.thread{max-height:48vh;overflow:auto;padding:4px 16px}.ask:not(.has) .thread{display:none}
.msg{padding:10px 0;font-size:14.5px;line-height:1.6}.msg p{margin:0 0 6px}.msg ul{margin:0 0 6px 18px;list-style:disc}.msg li{margin:2px 0}
.msg.user{color:var(--soft);font-weight:600;border-top:1px solid var(--line)}.msg.user:first-child{border-top:0}.msg.wait{color:var(--faint);font-style:italic}
.chips{display:flex;flex-wrap:wrap;gap:6px;padding:12px 14px}.ask.has .chips{display:none}
.chips button{border:1px solid var(--line);background:var(--chip);color:var(--ink);border-radius:999px;padding:5px 12px;font:13.5px "Segoe UI",system-ui,sans-serif;cursor:pointer}
.chips button:hover{border-color:var(--accent)}
.day{font-size:12px;letter-spacing:.09em;text-transform:uppercase;color:var(--faint);font-weight:650;margin:26px 0 8px}
.list a.m .top{display:flex;justify-content:space-between;gap:12px;align-items:baseline}.list .when{color:var(--faint);font-size:13px;white-space:nowrap}
.return{position:fixed;right:20px;bottom:90px;padding:10px 16px;border-radius:999px;border:0;background:var(--accent);color:var(--card);
font:600 14px "Segoe UI",system-ui,sans-serif;cursor:pointer;box-shadow:0 4px 16px rgba(0,0,0,.25)}
.list{max-width:740px;margin:0 auto}.list a.m{display:block;background:var(--card);border:1px solid var(--line);border-radius:12px;padding:15px 18px;margin:0 0 10px;color:var(--ink)}
.list a.m:hover{text-decoration:none;border-color:var(--accent)}.list .mt{font-weight:650;font-size:16.5px}.list .ms{color:var(--soft);font-size:13.5px;margin-top:3px}
.solo{max-width:740px;margin:0 auto;padding:40px 24px 96px}
@media (max-width:640px){.shell,.solo{padding:22px 16px 72px}h1{font-size:23px}.glance{gap:6px}.glance a{padding:9px 10px}.glance b{font-size:20px}
.card,.topic{padding-left:14px;padding-right:14px}.line{grid-template-columns:44px minmax(0,1fr)}.line .s{grid-column:2}.line .x{grid-column:1/-1}
.pts .row.response{margin-left:18px}td.own,td.due{white-space:normal}}
"""

JS = """
const key='hex-meeting-done-'+document.body.dataset.id;
let done={};try{done=JSON.parse(localStorage.getItem(key)||'{}')}catch(e){}
const boxes=[...document.querySelectorAll('input[data-i]')];
const paint=()=>boxes.forEach(b=>{b.checked=!!done[b.dataset.i];b.closest('tr,li').classList.toggle('done',b.checked)});paint();
boxes.forEach(b=>b.addEventListener('change',()=>{done[b.dataset.i]=b.checked;paint();try{localStorage.setItem(key,JSON.stringify(done))}catch(e){}}));
const f=document.querySelector('.filter');if(f)f.addEventListener('input',()=>{const q=f.value.toLowerCase();
document.querySelectorAll('.line').forEach(l=>{l.style.display=l.textContent.toLowerCase().includes(q)?'':'none'})});
// A timestamp opens the transcript at its line, highlights it with its neighbours, and offers a way back.
const back=document.querySelector('.return');let from=null;
document.addEventListener('click',ev=>{const a=ev.target.closest('a.at');if(!a)return;ev.preventDefault();
const line=document.querySelector(a.getAttribute('href'));if(!line)return;
document.querySelector('details').open=true;if(f){f.value='';document.querySelectorAll('.line').forEach(l=>l.style.display='')}
document.querySelectorAll('.hit,.near,.from').forEach(x=>x.classList.remove('hit','near','from'));
line.classList.add('hit');[line.previousElementSibling,line.nextElementSibling].forEach(n=>n&&n.classList.contains('line')&&n.classList.add('near'));
from=a.closest('.row,tr,li,.msg');if(from)from.classList.add('from');
line.scrollIntoView({behavior:'smooth',block:'center'});if(back)back.hidden=false});
if(back)back.addEventListener('click',()=>{if(from)from.scrollIntoView({behavior:'smooth',block:'center'});back.hidden=true});
"""


def e(text):
    return html.escape(str(text or ""))


def seconds(stamp):
    parts = [int(x) for x in stamp.split(":")]
    while len(parts) < 3:
        parts.insert(0, 0)
    return parts[0] * 3600 + parts[1] * 60 + parts[2]


LINE_STARTS = []  # start second of each transcript line on the page being built


def stamps_html(at):
    links = []
    for a in (at or [])[:1]:
        try:
            wanted = seconds(a)
        except ValueError:
            continue
        label = e(short(a))
        if not LINE_STARTS:
            links.append(f'<span class="at">{label}</span>')
            continue
        # The line being spoken at that moment: the last one starting at or before it.
        index = max([i for i, s in enumerate(LINE_STARTS) if s <= wanted + 1] or [0])
        links.append(f'<a class="at" href="#line-{index}" title="Read this in the transcript">{label}</a>')
    return "".join(links)


def page(title, body, meeting_id=""):
    return (f'<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">'
            f"<title>{e(title)}</title><style>{CSS}</style></head><body data-id=\"{e(meeting_id)}\">{body}"
            f"<script>{JS}</script></body></html>")


def meeting_html(data, meta, lines):
    LINE_STARTS[:] = [seconds(l["at"]) for l in lines]
    decided = [p for _, p in points(data) if p.get("type") == "decided"]
    todo, settle, topics = data.get("todo", []), data.get("to_settle", []), data.get("topics", [])
    icons = {"fact": "", "decided": "✓", "concern": "!", "response": "↳", "open": "?"}

    def row(kind, text, who=None, at=None, icon=None):
        mark = icons.get(kind, "") if icon is None else icon
        tail = (f' <span class="who">{e(who)}</span>' if who else "") + stamps_html(at)
        return (f'<div class="row {e(kind)}"><span class="ic {e(kind if icon is None else "num")}">{e(mark)}</span>'
                f'<div class="tx">{e(text)}{tail}</div></div>')

    chips = [meta["started"] and meta["started"].strftime("%a %d %b %Y · %H:%M"),
             meta["minutes"] and f"{meta['minutes']} min", data.get("kind"), ", ".join(data.get("people") or [])]
    nav = ['<div class="label">On this page</div>', '<a href="#context">Context</a>']
    b = ['<a class="back" href="index.html">← All meetings</a>', f"<h1>{e(meta['title'])}</h1>",
         '<div class="meta">' + "".join(f'<span class="chip">{e(c)}</span>' for c in chips if c) + "</div>",
         '<div class="glance">'
         f'<a class="g-ok" href="#decisions"><b>{len(decided)}</b><span>decisions</span></a>'
         f'<a class="g-todo" href="#todo"><b>{len(todo)}</b><span>to do</span></a>'
         f'<a class="g-open" href="#settle"><b>{len(settle)}</b><span>still open</span></a></div>',
         '<h2 id="context">Context</h2><div class="card"><ul class="rows">'
         + "".join(f"<li>{row('fact', c)}</li>" for c in data.get("context", [])) + "</ul></div>"]
    if decided:
        nav.append('<a href="#decisions">Decisions</a>')
        b.append('<h2 id="decisions">Decisions</h2><div class="card"><ul class="rows">' + "".join(
            f"<li>{row('decided', p['text'], p.get('who'), p.get('at'), icon=str(i))}</li>" for i, p in enumerate(decided, 1))
            + "</ul></div>")
    side = ""
    if todo:
        items = "".join(
            f'<li><label><input type="checkbox" data-i="{i}"><span class="tt">{e(item["task"])}</span></label>'
            f'<div class="tm"><b>{e(item.get("owner") or "Owner to confirm")}</b> · by {e(nice_date(item.get("due")))}'
            + (" (default)" if item.get("due_default") else "") + f'{stamps_html(item.get("at"))}</div></li>'
            for i, item in enumerate(todo))
        listing = f'<ul class="todo-list">{items}</ul>'
        side = (f'<aside class="side"><div class="side-card"><div class="side-h">To do <span class="count">{len(todo)}</span></div>'
                f'{listing}</div><p class="side-note">Ticks are saved in this browser.</p></aside>')
        b.append(f'<div class="todo-inline"><h2 id="todo">To do</h2><div class="card">{listing}</div></div>')
    if settle:
        nav.append('<a href="#settle">Still to settle</a>')
        b.append('<h2 id="settle">Still to settle</h2><div class="card"><ul class="rows">' + "".join(
            f"<li>{row('open', q['question'], q.get('waits_on') and 'waits on ' + q['waits_on'])}</li>" for q in settle)
            + "</ul></div>")
    nav.append('<a href="#discussion">Discussion</a>')
    b.append('<h2 id="discussion">Discussion</h2>')
    for n, topic in enumerate(topics, 1):
        nav.append(f'<a class="sub" href="#topic-{n}">{e(topic["title"])}</a>')
        rows = "".join(row(p.get("type", "fact"), p["text"], p.get("who"), p.get("at")) for p in topic.get("points", []))
        b.append(f'<section class="card topic" id="topic-{n}"><h3>{e(topic["title"])}</h3><div class="pts">{rows}</div></section>')
    if data.get("verify"):
        nav.append('<a href="#verify">Check before acting</a>')
        b.append('<h2 id="verify">Check before acting</h2><div class="card"><ul class="rows verify">' + "".join(
            f"<li>{row('concern', v)}</li>" for v in data["verify"]) + "</ul></div>")
    if lines:
        nav.append('<a href="#transcript">Transcript</a>')
        b.append(f'<h2 id="transcript">Transcript</h2><details><summary>Show the full English transcript ({len(lines)} lines)</summary>'
                 '<input class="filter" placeholder="Filter lines…">' + "".join(
                     f'<div class="line{" you" if l["who"] == "You" else ""}" id="line-{i}"><span class="t">{e(short(l["at"]))}</span>'
                     f'<span class="s">{e(l["who"])}</span><span class="x">{e(l["text"])}</span></div>' for i, l in enumerate(lines))
                 + "</details>" '<button class="return" hidden>↑ Back to the summary point</button>')
    body =f'<div class="shell"><nav class="toc">{"".join(nav)}</nav><main>{"".join(b)}</main>{side}</div>'
    return page(meta["title"], body, meta["id"])


def slug(meta):
    stamp = meta["started"].strftime("%Y-%m-%d-%H%M") if meta["started"] else meta["id"]
    name = re.sub(r"[^A-Za-z0-9]+", "-", meta["title"]).strip("-")
    if len(name) > 60:
        name = name[:60].rsplit("-", 1)[0]
    return f"{stamp}-{name}.html"


def write_index(folder, selected=None):
    # A shareable index contains only the explicitly selected meeting, with no hidden corpus.
    if selected is None:
        return
    _data, meta, _lines = selected
    body = f'<main class="solo"><h1>Meeting</h1><a href="{e(slug(meta))}">{e(meta["title"])}</a></main>'
    safe_export(folder / "index.html").write_text(page("Meeting", body), encoding="utf-8")


def render(meeting):
    data, meta, lines = load(meeting)
    if meta.get("export"):
        safe_export(meta["export"])
    folder = safe_export(viewer_dir())
    no_links(Path(meeting, "english-summary.md")).write_text(markdown(data, meta), encoding="utf-8")
    folder.mkdir(parents=True, exist_ok=True)
    target = safe_export(folder / slug(meta))
    target.write_text(meeting_html(data, meta, lines), encoding="utf-8")
    write_index(folder, (data, meta, lines))
    return target


if __name__ == "__main__":
    targets = [p.parent for p in HEX_MEETINGS.glob("*/english-summary.json")] if sys.argv[1:] == ["--all"] else sys.argv[1:]
    for meeting in targets:
        print(f"Meeting page: {render(meeting)}")
