//! Capture review, explicit approval and locally saved annotation notes.
mod workflow;
use super::{
    Event, State, hex, nav, roles,
    theme::{self, Palette},
    widgets::{link, pressable, scroll},
};
use rune::{El, Image, ImageCache, Key, TextEdit, col, el, row};
use rune_kit::TextArea;
use sidenote::{
    core::{
        annotations::{
            self, Capture, Keyframe, Mark, Scan, Word,
            generation::{self, Epoch, Progress, Ticket},
            media::{self, ApprovedCapture, ReviewFrame},
            note,
        },
        file_reads,
    },
    hex_engine::HexEngine,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
pub(super) use workflow::agents_section;
pub(super) use workflow::new_button;
use workflow::*;

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
    can_record: bool,
    record_busy: bool,
    review: Option<Vec<ReviewFrame>>,
    dragging: Option<(u64, media::BoxNorm)>,
    preview: Option<ApprovedCapture>,
    preview_images: BTreeMap<u64, Image>,
    approved: Option<ApprovedCapture>,
    drafts: BTreeMap<u64, serde_json::Value>,
    note: Option<serde_json::Value>,
    stream: String,
    busy: bool,
    status: String,
    epoch: Epoch,
    ticket: Option<Ticket>,
    editing: BTreeMap<String, TextEdit>,
    note_images: BTreeMap<u64, Image>,
    note_open: bool,
    source_fingerprint: Option<String>,
    review_source: Option<String>,
    last_review: Option<Vec<ReviewFrame>>,
    options: Option<generation::Options>,
    model: TextEdit,
}
pub enum Msg {
    Scanned {
        scan: Scan,
        shortcut: Option<String>,
        disabled: bool,
        can_record: bool,
    },
    Images {
        folder: PathBuf,
        images: Vec<Result<Image, String>>,
        fingerprint: Option<String>,
    },
    Recording(Result<(), String>),
    Progress {
        id: u64,
        progress: Progress,
    },
    Approved {
        id: u64,
        approved: ApprovedCapture,
    },
    Finished {
        id: u64,
        result: Result<(), String>,
    },
    Loaded {
        folder: PathBuf,
        approved: ApprovedCapture,
        note: serde_json::Value,
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
        self.load_annotation_options();
        self.refresh_annotations();
        self.nav_done();
    }
    pub fn refresh_annotations(&mut self) {
        if self.annotations.loading
            || self.annotations.busy
            || self.annotations.review.is_some()
            || rune::tooling::headless_run().is_some()
        {
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
            let can_record = hex_program().is_ok();
            let _ = sender.send(Event::Annotations(Msg::Scanned {
                scan,
                shortcut,
                disabled,
                can_record,
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
        self.cancel_annotation_note();
        self.annotations.review = None;
        self.annotations.approved = None;
        self.annotations.note = None;
        self.annotations.drafts.clear();
        self.annotations.note_open = false;
        self.annotations.note_images.clear();
        self.annotations.last_review = None;
        self.annotations.source_fingerprint = None;
        self.annotations.review_source = None;
        self.annotations.editing.clear();
        self.annotations.selected = Some(capture.folder.clone());
        self.annotations.transcript = false;
        self.annotations.images.clear();
        let sender = self.events_tx.clone();
        let wake = self.waker.clone().unwrap_or_else(|| Arc::new(|| {}));
        let root = self.store.root.clone();
        std::thread::spawn(move || {
            let fingerprint = media::source_fingerprint(&capture).ok();
            let images = images(&capture, wake.clone(), false);
            let fingerprint = fingerprint
                .filter(|f| media::source_fingerprint(&capture).ok().as_ref() == Some(f));
            let _ = sender.send(Event::Annotations(Msg::Images {
                folder: capture.folder.clone(),
                images,
                fingerprint,
            }));
            if let Ok(Some((approved, note))) = load_note(&root, &capture) {
                let _ = sender.send(Event::Annotations(Msg::Loaded {
                    folder: capture.folder,
                    approved,
                    note,
                }));
            }
            wake();
        });
    }
    pub fn apply_annotations(&mut self, msg: Msg) {
        match msg {
            Msg::Scanned {
                scan,
                shortcut,
                disabled,
                can_record,
            } => {
                self.annotations.captures = scan.captures;
                self.annotations.errors = scan.errors;
                self.annotations.shortcut = shortcut;
                self.annotations.disabled = disabled;
                self.annotations.can_record = can_record;
                self.annotations.loading = false;
                if let Some(folder) = self.annotations.selected.clone()
                    && !self.annotations.captures.iter().any(|c| c.folder == folder)
                {
                    self.annotations.selected = None;
                    self.annotations.images.clear();
                }
            }
            Msg::Images {
                folder,
                images,
                fingerprint,
            } if self.annotations.selected.as_ref() == Some(&folder) => {
                self.annotations.images = images;
                self.annotations.source_fingerprint = fingerprint;
            }
            Msg::Images { .. } => {}
            Msg::Recording(result) => {
                self.annotations.record_busy = false;
                self.annotations.status = match result {
                    Ok(()) => "Recording in Hex. Stop it in Hex, then refresh Annotations.".into(),
                    Err(e) => e,
                };
            }
            Msg::Approved { id, approved } if self.annotation_job_current(id) => {
                self.annotations.status =
                    if generation::uses_frame_drafts(approved.approval.frames.len()) {
                        "Writing frame drafts...".into()
                    } else {
                        "Writing the annotation note...".into()
                    };
                self.annotations.approved = Some(approved);
                self.load_annotation_note_images();
            }
            Msg::Progress { id, progress } if self.annotation_job_current(id) => match progress {
                Progress::Frame { index, draft, .. } => {
                    self.annotations.drafts.insert(index, draft);
                    let ready = self.annotations.drafts.len();
                    let total = self
                        .annotations
                        .approved
                        .as_ref()
                        .map_or(ready, |a| a.approval.frames.len());
                    self.annotations.status = if ready < total {
                        format!("{ready} of {total} frame drafts ready.")
                    } else {
                        format!(
                            "{ready} of {total} frame drafts ready. Writing the annotation note..."
                        )
                    };
                }
                Progress::NoteDelta(text) => self.annotations.stream = text,
                Progress::Final { note, .. } => {
                    self.annotations.note = Some(note);
                    self.annotations.busy = false;
                    self.annotations.status = "Annotation note saved on this PC.".into();
                    if let Some(a) = &self.annotations.approved
                        && save_latest(&self.store.root, a).is_err()
                    {
                        self.annotations.status="The note is saved, but Sidenote could not save its capture link. Save your edits to try again.".into();
                    }
                    self.annotations.note_open = true;
                    self.load_annotation_note_images();
                }
            },
            Msg::Finished { id, result } if self.annotation_job_current(id) => {
                self.annotations.busy = false;
                if let Err(error) = result {
                    self.annotations.status = error;
                }
            }
            Msg::Loaded {
                folder,
                approved,
                note,
            } if self.annotations.selected.as_ref() == Some(&folder) && !self.annotations.busy => {
                self.annotations.approved = Some(approved);
                self.annotations.note = Some(note);
                self.load_annotation_note_images();
            }
            Msg::Approved { .. }
            | Msg::Progress { .. }
            | Msg::Finished { .. }
            | Msg::Loaded { .. } => {}
        }
    }
}

pub fn on_key(s: &mut State, e: &rune::KeyEvent) -> bool {
    if e.key == Key::Escape {
        if s.annotations.review.take().is_some() {
            return true;
        }
        if s.annotations.note_open {
            s.cancel_annotation_note();
            s.annotations.note_open = false;
            return true;
        }
        s.cancel_annotation_note();
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
    let mut page = col().w_full().gap(14.0).child(
        row()
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .items_center()
            .key("annotation-breadcrumb")
            .child(
                link("Annotations", p)
                    .key("annotations-back")
                    .on_press(|s: &mut State| {
                        s.cancel_annotation_note();
                        s.annotations.selected = None;
                        s.annotations.images.clear();
                        s.annotations.review = None;
                        s.annotations.note_open = false;
                    }),
            )
            .child(roles::META.line("/", p.faint))
            .child(roles::META.line(c.title(), p.soft).grow(1.0).min_w(0.0)),
    );
    if s.annotations.busy || s.annotations.note_open {
        return note_page(s, c, width, p, page);
    }
    page = page
        .child(roles::GROUP.wrap(c.app(), p.soft))
        .child(roles::SECTION.wrap(c.title(), p.ink).max_lines(2))
        .child(facts(c, p));
    if s.annotations.review.is_some() {
        return review_page(s, c, width, p, page);
    }
    page = page.child(
        row()
            .gap(12.0)
            .child(
                super::area::button("Write note", "annotation-write-note", p)
                    .on_press(|s: &mut State| s.review_annotation()),
            )
            .children(s.annotations.note.as_ref().map(|_| {
                link("Open annotation note", p)
                    .key("annotation-open-note")
                    .on_press(|s: &mut State| s.annotations.note_open = true)
            })),
    );
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
    if !s.annotations.note_open
        && s.annotations.review.is_none()
        && !s.annotations.status.is_empty()
    {
        content = content.child(
            roles::SMALL
                .wrap(&s.annotations.status, p.soft)
                .role("status"),
        );
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
    s.annotations.can_record = true;
    s.ai_notes.settings = Some(sidenote::core::notes::NotesSettings {
        helper: Some(sidenote::core::notes::Helper::Claude),
        model: "claude-sonnet-5-5".into(),
        ..Default::default()
    });
    s.annotations.captures = if name == "annotations-empty" {
        Vec::new()
    } else {
        annotations::scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations"),
        )
        .captures
    };
    super::scene_sidebar(s, true);
    if (name.starts_with("annotation-capture")
        || name == "annotation-transcript"
        || matches!(
            name,
            "annotation-review" | "annotation-drafting" | "annotation-note"
        ))
        && let Some(c) = s.annotations.captures.first().cloned()
    {
        s.annotations.selected = Some(c.folder.clone());
        s.annotations.images = images(&c, Arc::new(|| {}), true);
        s.annotations.source_fingerprint = media::source_fingerprint(&c).ok();
        s.annotations.transcript = name == "annotation-transcript";
        if name == "annotation-review" {
            s.review_annotation();
            if let Some(frames) = &mut s.annotations.review {
                frames[0].masks.push(media::SYNTHETIC_MASK);
            }
        }
        if name == "annotation-review" {
            s.prepare_annotation_preview();
        }
        if matches!(name, "annotation-drafting" | "annotation-note") {
            let review: Vec<_> = c
                .data
                .keyframes
                .iter()
                .enumerate()
                .map(|(i, f)| ReviewFrame {
                    index: f.index,
                    included: true,
                    masks: if i == 0 {
                        vec![media::SYNTHETIC_MASK]
                    } else {
                        Vec::new()
                    },
                })
                .collect();
            let approved =
                media::approve(&c, &review, &s.store.root).expect("synthetic annotation approval");
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tools/ui-fixtures/annotation-note.json");
            let value = std::fs::read(fixture)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .filter(|n| note::validate(n, &approved).is_ok())
                .unwrap_or_else(|| note::synthetic_note(&approved));
            s.annotations.approved = Some(approved);
            s.annotations.note_open = true;
            s.load_annotation_note_images();
            if name == "annotation-note" {
                s.annotations.note = Some(value);
                s.annotations.status = "Annotation note saved on this PC.".into();
            } else {
                s.annotations.busy = true;
                s.annotations.ticket = Some(s.annotations.epoch.begin());
                s.annotations.status = "Writing the annotation note...".into();
                let summary = value["summary"].as_str().unwrap_or("");
                let preview: String = summary.chars().take(135).collect();
                s.annotations.stream = format!(
                    "{{\"title\":{},\"summary\":{}",
                    value["title"],
                    serde_json::to_string(&preview)
                        .unwrap()
                        .trim_end_matches('"')
                );
            }
        }
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
