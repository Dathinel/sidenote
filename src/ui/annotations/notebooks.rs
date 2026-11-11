use super::*;
use crate::ui::widgets::ButtonDisabled;
use rune_kit::TextInput;
use serde_json::{Value, json};
use sidenote::core::annotations::{
    notebook::{self, Document, Input, Notebook},
    share,
};
#[derive(Default)]
pub(super) struct Ui {
    pub books: Vec<Notebook>,
    pub selected: Option<String>,
    pub meta: bool,
    pub title: TextEdit,
    pub purpose: TextEdit,
    pub add: bool,
    pub picker: Option<Capture>,
    pub note_open: bool,
    pub pending: Option<String>,
    pub editing: BTreeMap<String, TextEdit>,
    pub images: BTreeMap<String, Image>,
    pub dragging: Option<usize>,
    pub shared: Option<PathBuf>,
}
impl State {
    pub(super) fn load_notebooks(&mut self) {
        match notebook::list(&self.store.root) {
            Ok(books) => self.annotations.notebooks.books = books,
            Err(e) => self
                .annotations
                .errors
                .push(format!("Could not load notebooks: {e}")),
        }
    }
    fn book(&self) -> Option<&Notebook> {
        let id = self.annotations.notebooks.selected.as_ref()?;
        self.annotations
            .notebooks
            .books
            .iter()
            .find(|b| &b.id == id)
    }
    fn persist_book(&mut self, b: Notebook) -> bool {
        if let Err(e) = notebook::save(&self.store.root, &b) {
            self.annotations.status = e.to_string();
            return false;
        }
        if let Some(old) = self
            .annotations
            .notebooks
            .books
            .iter_mut()
            .find(|v| v.id == b.id)
        {
            *old = b;
        } else {
            self.annotations.notebooks.books.insert(0, b);
        }
        true
    }
    pub(super) fn open_notebook(&mut self, id: &str) {
        self.cancel_annotation_note();
        self.annotations.selected = None;
        self.annotations.review = None;
        self.annotations.note_open = false;
        self.annotations.notebooks.selected = Some(id.into());
        self.annotations.notebooks.note_open = false;
        self.annotations.notebooks.pending = None;
        self.annotations.notebooks.meta = false;
        self.annotations.notebooks.add = false;
        self.annotations.notebooks.picker = None;
        self.annotations.notebooks.editing.clear();
        self.annotations.notebooks.shared = None;
        self.annotations.status.clear();
        self.load_book_images();
    }
    fn new_notebook(&mut self) {
        let mut b = Notebook::new("Untitled notebook");
        if let Some(c) = self.annotations.notebooks.picker.take() {
            b.add(&c);
        }
        let id = b.id.clone();
        if self.persist_book(b) {
            self.open_notebook(&id);
            self.edit_notebook_metadata();
        }
    }
    fn edit_notebook_metadata(&mut self) {
        let Some(b) = self.book().cloned() else {
            return;
        };
        self.annotations.notebooks.title = TextEdit::new(&b.title);
        self.annotations.notebooks.purpose = TextEdit::new(&b.purpose);
        self.annotations.notebooks.meta = true;
    }
    fn save_notebook_metadata(&mut self) {
        let Some(mut b) = self.book().cloned() else {
            return;
        };
        b.title = self.annotations.notebooks.title.text.trim().into();
        b.purpose = self
            .annotations
            .notebooks
            .purpose
            .text
            .trim()
            .replace(['\r', '\n'], " ");
        b.revision += 1;
        if self.persist_book(b) {
            self.annotations.notebooks.meta = false;
        }
    }
    fn add_book_capture(&mut self, index: usize) {
        let Some(c) = self.annotations.captures.get(index).cloned() else {
            return;
        };
        let Some(mut b) = self.book().cloned() else {
            return;
        };
        if b.add(&c) {
            self.persist_book(b);
        }
    }
    fn move_book_capture(&mut self, index: usize, target: usize) {
        let Some(mut b) = self.book().cloned() else {
            return;
        };
        if b.reorder(index, target) {
            self.persist_book(b);
        }
    }
    fn remove_book_capture(&mut self, index: usize) {
        let Some(mut b) = self.book().cloned() else {
            return;
        };
        if b.remove(index) {
            self.persist_book(b);
        }
    }
    pub(super) fn choose_notebook_for_capture(&mut self) {
        let c = self
            .annotations
            .captures
            .iter()
            .find(|c| Some(&c.folder) == self.annotations.selected.as_ref())
            .cloned();
        self.load_notebooks();
        self.annotations.notebooks.picker = c;
    }
    fn add_to_chosen_notebook(&mut self, id: &str) {
        let Some(c) = self.annotations.notebooks.picker.clone() else {
            return;
        };
        let Some(mut b) = self
            .annotations
            .notebooks
            .books
            .iter()
            .find(|b| b.id == id)
            .cloned()
        else {
            return;
        };
        b.add(&c);
        if self.persist_book(b) {
            self.open_notebook(id);
        }
    }
    pub(super) fn continue_notebook(&mut self) {
        if let Some(id) = self.annotations.notebooks.pending.take() {
            self.open_notebook(&id);
            self.write_notebook_note();
        }
    }
    fn write_notebook_note(&mut self) {
        let Some(book) = self.book().cloned() else {
            return;
        };
        if book.captures.is_empty() {
            self.annotations.status = "Add captures before writing the notebook note.".into();
            return;
        }
        let options = match self.annotation_options() {
            Ok(o) => o,
            Err(e) => {
                self.annotations.status = e;
                return;
            }
        };
        let mut inputs = Vec::new();
        for reference in &book.captures {
            let c = match annotations::read_capture(&reference.folder) {
                Ok(c) => c,
                Err(_) => {
                    self.annotations.status = format!(
                        "{} is unavailable. Remove it or restore its files.",
                        reference.title
                    );
                    return;
                }
            };
            match notebook::load_capture_note(&self.store.root, &c) {
                Ok(Some((approved, note))) => inputs.push(Input {
                    key: reference.key.clone(),
                    approved,
                    note,
                }),
                Ok(None) | Err(_) => {
                    self.annotations.notebooks.pending = Some(book.id.clone());
                    self.annotations.notebooks.selected = None;
                    let index = if let Some(i) = self
                        .annotations
                        .captures
                        .iter()
                        .position(|v| v.folder == c.folder)
                    {
                        i
                    } else {
                        self.annotations.captures.push(c);
                        self.annotations.captures.len() - 1
                    };
                    self.select_annotation(index);
                    self.annotations.status="Review and approve this capture first. The notebook continues after its note is saved.".into();
                    return;
                }
            }
        }
        if rune::tooling::headless_run().is_some() {
            return;
        }
        self.cancel_annotation_note();
        let ticket = self.annotations.epoch.begin();
        let id = ticket.id;
        self.annotations.ticket = Some(ticket.clone());
        self.annotations.busy = true;
        self.annotations.stream.clear();
        self.annotations.notebooks.note_open = true;
        self.annotations.status =
            "Writing the notebook note from existing annotation notes...".into();
        let folder = self
            .store
            .root
            .join("annotations/notebooks/jobs")
            .join(format!("{}-{id}", book.id));
        let sender = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result =
                generation::run_notebook(&book, &inputs, &options, &folder, &ticket, |text| {
                    let _ = sender.send(Event::Annotations(Msg::NotebookDelta { id, text }));
                    if let Some(w) = &wake {
                        w();
                    }
                });
            let _ = sender.send(Event::Annotations(Msg::NotebookReady {
                id,
                book_id: book.id,
                revision: book.revision,
                result,
            }));
            if let Some(w) = wake {
                w();
            }
        });
    }
    pub(super) fn apply_notebook_ready(
        &mut self,
        book_id: String,
        revision: u64,
        result: Result<Document, String>,
    ) {
        self.annotations.busy = false;
        match result {
            Ok(doc) => {
                let Some(mut book) = self
                    .annotations
                    .notebooks
                    .books
                    .iter()
                    .find(|b| b.id == book_id)
                    .cloned()
                else {
                    return;
                };
                if book.revision != revision {
                    self.annotations.status =
                        "Notebook changed while writing. Write the note again.".into();
                    return;
                }
                book.note = Some(doc);
                book.note_revision = revision;
                if self.persist_book(book) {
                    self.load_book_images();
                    self.annotations.status = "Notebook note saved on this PC.".into();
                }
            }
            Err(e) => self.annotations.status = e,
        }
    }
    fn load_book_images(&mut self) {
        self.annotations.notebooks.images.clear();
        let Some(doc) = self.book().and_then(|b| b.note.as_ref()).cloned() else {
            return;
        };
        if let Err(e) = notebook::validate(&doc) {
            self.annotations.status = e;
            return;
        }
        let cache = ImageCache::new(self.waker.clone().unwrap_or_else(|| Arc::new(|| {})));
        for asset in &doc.assets {
            for frame in &asset.approved.approval.frames {
                if let Ok(bytes) = file_reads::read(
                    &asset.approved.folder.join(&frame.marked),
                    annotations::IMAGE_MAX_BYTES,
                ) {
                    let image = cache.load(bytes);
                    if rune::tooling::headless_run().is_some() {
                        let _ = image.wait();
                    }
                    self.annotations
                        .notebooks
                        .images
                        .insert(format!("{}:{}", asset.key, frame.index), image);
                }
            }
        }
    }
    fn edit_book_note(&mut self) {
        let Some(doc) = self.book().and_then(|b| b.note.as_ref()).cloned() else {
            return;
        };
        let mut fields = BTreeMap::new();
        super::workflow::editable_fields(&doc.value, "", &mut fields);
        self.annotations.notebooks.editing = fields;
    }
    fn save_book_note_edits(&mut self) {
        let Some(mut book) = self.book().cloned() else {
            return;
        };
        let Some(doc) = &mut book.note else {
            return;
        };
        for (path, input) in &self.annotations.notebooks.editing {
            if let Some(v) = doc.value.pointer_mut(path) {
                *v = json!(input.text);
            }
        }
        match notebook::validate(doc) {
            Ok(()) => {
                if self.persist_book(book) {
                    self.annotations.notebooks.editing.clear();
                    self.annotations.status = "Your edits are saved on this PC.".into();
                }
            }
            Err(e) => self.annotations.status = e,
        }
    }
    pub(super) fn share_annotation(&mut self, book: bool) {
        if self.annotations.busy
            || (book && !self.annotations.notebooks.editing.is_empty())
            || (!book && !self.annotations.editing.is_empty())
        {
            self.annotations.status = "Finish writing and save your edits before sharing.".into();
            return;
        }
        if rune::tooling::headless_run().is_some() {
            return;
        }
        let result = if book {
            self.book()
                .filter(|b| b.fresh())
                .and_then(|b| b.note.as_ref())
                .ok_or_else(|| "Write the current notebook note before sharing.".to_string())
                .and_then(|doc| {
                    share::notebook(doc)
                        .map(|html| (doc.value["title"].as_str().unwrap().to_owned(), html))
                })
        } else {
            self.annotations
                .note
                .as_ref()
                .zip(self.annotations.approved.as_ref())
                .ok_or_else(|| "Write and approve a note before sharing.".to_string())
                .and_then(|(note, a)| {
                    share::capture(note, a)
                        .map(|html| (note["title"].as_str().unwrap().to_owned(), html))
                })
        };
        match result {
            Ok((title, html)) => {
                match sidenote::platform::save_html_dialog(&share::default_name(&title)) {
                    Ok(Some(path)) => match share::save(&path, &html) {
                        Ok(()) => self.annotations.notebooks.shared = Some(path),
                        Err(e) => self.annotations.status = e,
                    },
                    Ok(None) => {}
                    Err(e) => self.annotations.status = e.to_string(),
                }
            }
            Err(e) => self.annotations.status = e,
        }
    }
}
fn primary(label: &str, key: &str, p: Palette) -> El {
    crate::ui::widgets::button(label, key, crate::ui::widgets::ButtonRole::Primary, p)
}
pub(super) fn list_section(s: &State, p: Palette) -> El {
    let mut items = col().w_full().gap(4.0);
    for b in &s.annotations.notebooks.books {
        let id = b.id.clone();
        items = items.child(
            pressable(
                col()
                    .w_full()
                    .gap(4.0)
                    .padding(16.0)
                    .child(roles::TITLE.wrap(&b.title, p.ink).max_lines(2))
                    .child(roles::SMALL.wrap(
                        if b.purpose.is_empty() {
                            format!("{} captures", b.captures.len())
                        } else {
                            format!(
                                "{} captures  {}  {}",
                                b.captures.len(),
                                roles::DOT,
                                b.purpose
                            )
                        },
                        p.soft,
                    )),
            )
            .key(format!("notebook-row-{}", b.id))
            .hover(p.chip)
            .on_press(move |s: &mut State| s.open_notebook(&id)),
        );
    }
    if s.annotations.notebooks.books.is_empty() {
        items = items.child(
            roles::SMALL
                .wrap(
                    "Group captures into a notebook, then explain the whole story.",
                    p.soft,
                )
                .padding(16.0),
        );
    }
    col()
        .w_full()
        .gap(10.0)
        .mt(18.0)
        .mb(16.0)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(roles::SECTION.wrap("Notebooks", p.ink))
                .child(
                    crate::ui::widgets::action("New notebook", p)
                        .key("new-notebook")
                        .on_press(|s: &mut State| s.new_notebook()),
                ),
        )
        .child(hex::settings::card("Your notebooks", items, p))
        .key("notebooks-list")
}
pub(super) fn picker(s: &State, p: Palette) -> El {
    let mut page = col()
        .w_full()
        .gap(16.0)
        .child(roles::SECTION.wrap("Add to notebook", p.ink))
        .child(roles::SMALL.wrap("Choose a notebook for this capture.", p.soft));
    for b in &s.annotations.notebooks.books {
        let id = b.id.clone();
        page = page.child(
            link(&b.title, p)
                .key(format!("choose-notebook-{}", b.id))
                .on_press(move |s: &mut State| s.add_to_chosen_notebook(&id)),
        );
    }
    page.child(
        primary("New notebook", "picker-new-notebook", p)
            .on_press(|s: &mut State| s.new_notebook()),
    )
    .child(
        crate::ui::widgets::action("Cancel", p)
            .on_press(|s: &mut State| s.annotations.notebooks.picker = None),
    )
}
pub(super) fn confirmation(s: &State, p: Palette) -> El {
    let path = s.annotations.notebooks.shared.as_ref().unwrap();
    let copy = path.clone();
    let reveal = path.clone();
    col()
        .w_full()
        .gap(18.0)
        .key("share-confirmation")
        .child(
            roles::SECTION
                .wrap("HTML page saved", p.ink)
                .font(theme::display(28.0)),
        )
        .child(roles::READ.wrap(
            "One file with your note and approved images. It works offline and is ready to share.",
            p.ink,
        ))
        .child(roles::SMALL.wrap(
            path.file_name().unwrap_or_default().to_string_lossy(),
            p.soft,
        ))
        .child(
            row()
                .gap(12.0)
                .child(
                    primary("Copy file", "share-copy-file", p).on_press(move |s: &mut State| {
                        if rune::tooling::headless_run().is_none() {
                            s.annotations.status = match sidenote::platform::copy_file(&copy) {
                                Ok(()) => {
                                    "File copied. Paste it where you want to share it.".into()
                                }
                                Err(e) => format!("Could not copy the file: {e}"),
                            };
                        }
                    }),
                )
                .child(link("Show in folder", p).key("share-show-folder").on_press(
                    move |s: &mut State| {
                        if rune::tooling::headless_run().is_none()
                            && sidenote::platform::reveal_file(&reveal).is_err()
                        {
                            s.annotations.status = "Could not show the file in its folder.".into();
                        }
                    },
                )),
        )
        .child(
            link("Back to note", p)
                .key("share-back")
                .on_press(|s: &mut State| s.annotations.notebooks.shared = None),
        )
}
fn breadcrumb(s: &State, p: Palette) -> El {
    row()
        .gap(8.0)
        .items_center()
        .child(
            link("Annotations", p)
                .key("notebook-breadcrumb")
                .on_press(|s: &mut State| {
                    s.cancel_annotation_note();
                    s.annotations.notebooks.selected = None;
                    s.annotations.notebooks.note_open = false;
                }),
        )
        .child(roles::META.wrap("/", p.soft))
        .child(
            roles::META
                .wrap(s.book().map_or("Notebook", |b| b.title.as_str()), p.soft)
                .max_lines(1),
        )
}
pub(super) fn page(s: &State, width: f32, p: Palette) -> El {
    let Some(book) = s.book() else {
        return roles::READ.wrap("This notebook is unavailable.", p.soft);
    };
    let mut page = col()
        .w_full()
        .gap(16.0)
        .key("notebook-page")
        .child(breadcrumb(s, p));
    if s.annotations.notebooks.note_open {
        return note_page(s, book, width, p, page);
    }
    if s.annotations.notebooks.meta {
        let kit = theme::kit(&p);
        return page
            .child(roles::SECTION.wrap("Notebook details", p.ink))
            .child(
                TextInput::new(&s.annotations.notebooks.title)
                    .label("Notebook title")
                    .placeholder("Notebook title")
                    .on_edit(|s: &mut State| &mut s.annotations.notebooks.title)
                    .build(&kit)
                    .key("notebook-title")
                    .w_full(),
            )
            .child(
                TextInput::new(&s.annotations.notebooks.purpose)
                    .label("Purpose, optional")
                    .placeholder("Purpose, optional")
                    .on_edit(|s: &mut State| &mut s.annotations.notebooks.purpose)
                    .build(&kit)
                    .key("notebook-purpose")
                    .w_full(),
            )
            .child(
                primary("Save notebook", "save-notebook", p)
                    .on_press(|s: &mut State| s.save_notebook_metadata()),
            );
    }
    page = page.child(
        roles::SECTION
            .wrap(&book.title, p.ink)
            .font(theme::display(28.0)),
    );
    if !book.purpose.is_empty() {
        page = page.child(roles::READ.wrap(&book.purpose, p.soft).max_lines(1));
    }
    let mut actions = row()
        .flex_wrap()
        .gap(8.0)
        .items_center()
        .child(
            primary("Write notebook note", "write-notebook-note", p)
                .button_disabled(book.captures.is_empty())
                .on_press(|s: &mut State| s.write_notebook_note()),
        )
        .child(
            crate::ui::widgets::action("Edit details", p)
                .key("edit-notebook")
                .on_press(|s: &mut State| s.edit_notebook_metadata()),
        );
    if book.note.is_some() {
        actions = actions.child(
            link("Open note", p)
                .key("open-notebook-note")
                .on_press(|s: &mut State| s.annotations.notebooks.note_open = true),
        );
    }
    page = page.child(actions).child(roles::SMALL.wrap(
        "Uses each capture's note, no images sent. Drag or Alt+Up/Down to reorder.",
        p.soft,
    ));
    for (i, r) in book.captures.iter().enumerate() {
        let key = r.key.clone();
        let keyboard_key = key.clone();
        let count = book.captures.len();
        let folder = r.folder.clone();
        let capture = s.annotations.captures.iter().find(|c| c.folder == r.folder);
        let meta = format!(
            "{} {} {} marks {} {}",
            capture.map_or_else(|| "Date unknown".into(), date),
            roles::DOT,
            capture.map_or(0, |c| c.data.marks.len()),
            roles::DOT,
            if s.annotations.thumbnails.contains_key(&r.folder) {
                "Note ready"
            } else {
                "Needs review"
            }
        );
        let card = pressable(
            row()
                .w_full()
                .gap(10.0)
                .padding(12.0)
                .items_center()
                .child(number(i + 1, p))
                .child(
                    crate::ui::icons::draw(&crate::ui::icons::GRIP_VERTICAL, 18.0, p.soft)
                        .key(format!("notebook-grip-{i}"))
                        .help("Drag to reorder"),
                )
                .child(thumbnail(
                    s,
                    &r.folder,
                    format!("notebook-thumbnail-{i}"),
                    p,
                ))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(5.0)
                        .child(roles::TITLE.wrap(&r.title, p.ink).max_lines(1))
                        .child(roles::META.wrap(meta, p.soft).max_lines(1)),
                )
                .child(
                    crate::ui::widgets::action("Open", p)
                        .key(format!("notebook-open-{i}"))
                        .on_press(move |s: &mut State| {
                            if let Some(index) = s
                                .annotations
                                .captures
                                .iter()
                                .position(|c| c.folder == folder)
                            {
                                s.annotations.notebooks.selected = None;
                                s.select_annotation(index);
                            }
                        }),
                )
                .child(
                    crate::ui::widgets::action("Remove", p)
                        .key(format!("notebook-remove-{i}"))
                        .on_press(move |s: &mut State| s.remove_book_capture(i)),
                ),
        )
        .key(format!("notebook-capture-{key}"))
        .h(76.0)
        .border(
            1.0,
            if s.annotations.notebooks.dragging == Some(i) {
                p.accent
            } else {
                p.line
            },
        )
        .radius(9.0)
        .fill(p.card)
        .help("Drag to reorder. Alt+Up and Alt+Down also move this capture.")
        .on_key(move |s: &mut State, e: &rune::KeyEvent, _| {
            if e.mods.alt && matches!(e.key, Key::Up | Key::Down) {
                if let Some(index) = s
                    .book()
                    .and_then(|b| b.captures.iter().position(|r| r.key == keyboard_key))
                {
                    let target = (index as isize + if e.key == Key::Up { -1 } else { 1 })
                        .clamp(0, count.saturating_sub(1) as isize)
                        as usize;
                    s.move_book_capture(index, target);
                }
                true
            } else {
                false
            }
        })
        .on_drag(move |s: &mut State, d: &mut rune::Drag, _| {
            if d.phase == rune::Phase::Up {
                if !d.cancelled() {
                    let distance = ((d.y - d.start_y) / (d.h + 16.0)).round() as isize;
                    let target =
                        (i as isize + distance).clamp(0, count.saturating_sub(1) as isize) as usize;
                    s.move_book_capture(i, target);
                }
                s.annotations.notebooks.dragging = None;
            } else {
                s.annotations.notebooks.dragging = Some(i);
            }
        });
        page = page.child(card);
    }
    page = page.child(
        crate::ui::widgets::action("Add captures", p)
            .key("notebook-add-captures")
            .on_press(|s: &mut State| s.annotations.notebooks.add = !s.annotations.notebooks.add),
    );
    if s.annotations.notebooks.add || book.captures.is_empty() {
        for (i, c) in s.annotations.captures.iter().enumerate() {
            if !book.captures.iter().any(|r| r.folder == c.folder) {
                page = page.child(
                    crate::ui::widgets::action(format!("Add {}", c.title()), p)
                        .key(format!("notebook-add-{i}"))
                        .on_press(move |s: &mut State| s.add_book_capture(i)),
                );
            }
        }
    }
    if book.note.is_some() && !book.fresh() {
        page = page.child(roles::SMALL.wrap(
            "Notebook changed. Write the note again before sharing.",
            p.warn,
        ));
    }
    page
}
fn field_label(path: &str, value: &Value) -> String {
    let parts = path.split('/').collect::<Vec<_>>();
    let number = |i: usize| {
        parts
            .get(i)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            + 1
    };
    match parts.get(1).copied().unwrap_or("") {
        "title" => "Title".into(),
        "summary" => "Summary".into(),
        "open_questions" => format!("Open question {}", number(2)),
        "todos" => format!(
            "To-do {} {}",
            number(2),
            parts.last().unwrap_or(&"text").replace('_', " ")
        ),
        "sections" => match parts.get(3).copied().unwrap_or("") {
            "heading" => format!("Capture {} heading", number(2)),
            "paragraphs" => format!("Capture {} explanation {}", number(2), number(4)),
            "figures" => {
                if parts.get(5) == Some(&"caption") {
                    format!("Capture {} frame {} caption", number(2), number(4))
                } else {
                    format!(
                        "Capture {} frame {} mark {} {}",
                        number(2),
                        number(4),
                        value
                            .pointer(path.rsplit_once('/').map_or("", |(parent, _)| parent))
                            .and_then(|c| c["mark_number"].as_u64())
                            .unwrap_or(number(6) as u64),
                        parts.last().unwrap_or(&"text")
                    )
                }
            }
            _ => "Note text".into(),
        },
        _ => "Note text".into(),
    }
}

fn note_page(s: &State, book: &Notebook, width: f32, p: Palette, mut page: El) -> El {
    let mut actions = row().gap(12.0).items_center().child(
        link("Back to notebook", p)
            .key("back-to-notebook")
            .on_press(|s: &mut State| {
                s.cancel_annotation_note();
                s.annotations.notebooks.note_open = false;
            }),
    );
    if s.annotations.busy {
        actions = actions.child(
            crate::ui::widgets::action("Cancel", p)
                .key("notebook-cancel")
                .on_press(|s: &mut State| s.cancel_annotation_note()),
        );
    } else if book.note.is_some() {
        let editing = !s.annotations.notebooks.editing.is_empty();
        actions = actions.child(
            crate::ui::widgets::action(if editing { "Save changes" } else { "Edit note" }, p)
                .key("edit-notebook-note")
                .on_press(move |s: &mut State| {
                    if editing {
                        s.save_book_note_edits()
                    } else {
                        s.edit_book_note()
                    }
                }),
        );
        if !editing {
            actions = actions.child(
                primary("Share", "share-notebook", p)
                    .button_disabled(!book.fresh())
                    .on_press(|s: &mut State| s.share_annotation(true)),
            );
        }
    }
    page = page.child(actions);
    if s.annotations.busy {
        return page
            .child(roles::SECTION.wrap("Draft", p.ink).key("notebook-draft"))
            .child(
                roles::READ
                    .wrap(
                        generation::notebook_stream_preview(&s.annotations.stream),
                        p.ink,
                    )
                    .key("notebook-stream"),
            );
    }
    let Some(doc) = &book.note else {
        return page.child(roles::SMALL.wrap(
            "Write the notebook note when the captures are ready.",
            p.soft,
        ));
    };
    if let Err(e) = notebook::validate(doc) {
        return page
            .child(roles::SMALL.wrap(format!("This notebook note needs review: {e}"), p.warn));
    }
    let reading = notebook::reading_value(doc);
    if !s.annotations.notebooks.editing.is_empty() {
        let kit = theme::kit(&p);
        let mut fields = s.annotations.notebooks.editing.iter().collect::<Vec<_>>();
        fields.sort_by_key(|(k, _)| {
            if k.as_str() == "/title" {
                0
            } else if k.as_str() == "/summary" {
                1
            } else {
                2
            }
        });
        for (path, input) in fields {
            let key = path.clone();
            page = page
                .child(roles::GROUP.wrap(field_label(path, &reading), p.soft))
                .child(
                    TextArea::new(input)
                        .rows(3)
                        .width(None)
                        .label(field_label(path, &reading))
                        .on_edit(move |s: &mut State| {
                            s.annotations.notebooks.editing.get_mut(&key).unwrap()
                        })
                        .build(&kit)
                        .key(format!("notebook-edit-{path}"))
                        .w_full(),
                );
        }
        return page;
    }
    if !book.fresh() {
        page = page.child(roles::SMALL.wrap(
            "Notebook changed. Write the note again before sharing.",
            p.warn,
        ));
    }
    let value = &reading;
    page = page
        .child(
            roles::SECTION
                .wrap(value["title"].as_str().unwrap(), p.ink)
                .font(theme::display(28.0))
                .key("notebook-note-title"),
        )
        .child(
            roles::READ
                .wrap(value["summary"].as_str().unwrap(), p.ink)
                .key("notebook-note-summary")
                .selectable(true),
        );
    for (i, section) in value["sections"].as_array().unwrap().iter().enumerate() {
        page = page.child(
            roles::SECTION
                .wrap(section["heading"].as_str().unwrap(), p.ink)
                .mt(16.0)
                .key(format!("notebook-section-{i}")),
        );
        for paragraph in section["paragraphs"].as_array().unwrap() {
            page = page.child(
                roles::READ
                    .wrap(paragraph.as_str().unwrap(), p.ink)
                    .selectable(true),
            );
        }
        for (fi, figure) in section["figures"].as_array().unwrap().iter().enumerate() {
            if let Some(reference) = figure["same_screen"].as_str() {
                page = page.child(
                    roles::SMALL
                        .wrap(reference, p.soft)
                        .key(format!("notebook-reuse-{i}-{fi}")),
                );
                for explanation in figure["reuse_explanations"].as_array().unwrap() {
                    page = page.child(
                        roles::READ
                            .wrap(explanation.as_str().unwrap(), p.ink)
                            .selectable(true),
                    );
                }
                continue;
            }
            let frame = figure["keyframe_index"].as_u64().unwrap();
            let image_key = format!("{}:{frame}", section["capture_key"].as_str().unwrap());
            let side = width >= 860.0;
            let iw = if side { width - 320.0 } else { width };
            let mut image = el()
                .w(iw)
                .h(iw * 0.5625)
                .role("img")
                .label("Approved redacted frame with numbered marks")
                .key(format!("notebook-image-{i}-{frame}"));
            if let Some(img) = s.annotations.notebooks.images.get(&image_key)
                && let Some([w, h]) = img.size()
            {
                image = image.h(iw * h as f32 / w as f32).pixels(img.pixels());
                for c in figure["callouts"].as_array().unwrap() {
                    let n = c["mark_number"].as_u64().unwrap();
                    let old = c["original_mark_number"].as_u64().unwrap_or(n);
                    if old != n
                        && let Some(b) = c["boxNorm"].as_array()
                    {
                        let scale = iw / w as f32;
                        let bw = ((old.to_string().len().max(n.to_string().len()) as f32 * 12.0
                            + 6.0)
                            .max(22.0))
                            * scale;
                        let bh = 22.0 * scale;
                        let x = (b[0].as_f64().unwrap() as f32 * w as f32).floor() * scale;
                        let y = (b[1].as_f64().unwrap() as f32 * h as f32).floor() * scale;
                        image = image.child(
                            el().absolute(
                                x.min((iw - bw).max(0.0)),
                                y.min((iw * h as f32 / w as f32 - bh).max(0.0)),
                            )
                            .w(bw)
                            .h(bh)
                            .fill(rune::Color {
                                r: rune::color::srgb_decode(30.0 / 255.0),
                                g: rune::color::srgb_decode(74.0 / 255.0),
                                b: rune::color::srgb_decode(116.0 / 255.0),
                                a: 1.0,
                            })
                            .items_center()
                            .justify_center()
                            .child(
                                roles::META
                                    .wrap(
                                        n.to_string(),
                                        rune::Color {
                                            r: 1.0,
                                            g: 1.0,
                                            b: 1.0,
                                            a: 1.0,
                                        },
                                    )
                                    .font(theme::display(16.0 * scale)),
                            ),
                        );
                    }
                }
            }
            let mut callouts = col().w(if side { 300.0 } else { width }).gap(10.0);
            for c in figure["callouts"].as_array().unwrap() {
                let n = c["mark_number"].as_u64().unwrap() as usize;
                callouts = callouts.child(
                    col()
                        .w_full()
                        .gap(8.0)
                        .padding(12.0)
                        .border(1.0, p.line)
                        .radius(8.0)
                        .fill(p.card)
                        .key(format!("notebook-callout-{i}-{n}"))
                        .child(
                            row()
                                .gap(8.0)
                                .items_center()
                                .child(number(n, p))
                                .child(roles::NAME.wrap(c["label"].as_str().unwrap(), p.ink)),
                        )
                        .child(
                            roles::SMALL
                                .wrap(c["explanation"].as_str().unwrap(), p.ink)
                                .selectable(true),
                        ),
                );
            }
            page = page
                .child(if side {
                    row().w_full().gap(20.0).child(image).child(callouts)
                } else {
                    col().w_full().gap(12.0).child(image).child(callouts)
                })
                .child(roles::META.wrap(figure["caption"].as_str().unwrap(), p.soft));
        }
    }
    page = page.child(roles::SECTION.wrap("Open questions", p.ink).mt(16.0));
    for q in value["open_questions"].as_array().unwrap() {
        page = page.child(
            roles::READ
                .wrap(q.as_str().unwrap(), p.ink)
                .selectable(true),
        );
    }
    page = page.child(roles::SECTION.wrap("To-dos", p.ink).mt(16.0));
    for t in value["todos"].as_array().unwrap() {
        page = page.child(
            roles::READ
                .wrap(t["action"].as_str().unwrap(), p.ink)
                .selectable(true),
        );
    }
    page
}
pub(in crate::ui) fn scene(s: &mut State, name: &str) {
    super::scene(s, "annotations-list");
    let (mut book, inputs, captures) =
        notebook::synthetic(&s.store.root, 3).expect("synthetic notebooks");
    let mut doc = notebook::synthetic_document(&book, &inputs).unwrap();
    if let Ok(bytes) = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/ui-fixtures/notebook-note.json"),
    ) && let Ok(mut value) = serde_json::from_slice::<Value>(&bytes)
    {
        if let Some(sections) = value["sections"].as_array_mut() {
            for (section, reference) in sections.iter_mut().zip(&book.captures) {
                section["capture_key"] = json!(reference.key);
            }
        }
        let actual = Document {
            value,
            assets: doc.assets.clone(),
        };
        if notebook::validate(&actual).is_ok() {
            doc = actual;
        }
    }
    book.note = Some(doc.clone());
    book.note_revision = book.revision;
    notebook::save(&s.store.root, &book).unwrap();
    s.annotations.captures = captures;
    s.annotations.thumbnails = thumbnails(
        &s.store.root,
        &s.annotations.captures,
        s.waker.clone().unwrap_or_else(|| Arc::new(|| {})),
        true,
    );
    s.annotations.notebooks.books = vec![book.clone()];
    s.annotations.status.clear();
    if name == "notebook-needs-review" {
        let original = inputs[0].approved.folder.join("note.json");
        assert!(original.starts_with(&s.store.root));
        std::fs::rename(&original, original.with_file_name("note.saved.json")).unwrap();
        s.annotations.notebooks.books[0].note = None;
    }
    if name != "notebooks" {
        s.open_notebook(&book.id);
        s.annotations.notebooks.note_open = name == "notebook-note" || name == "share-confirmation";
        s.load_book_images();
    }
    if name == "share-confirmation" {
        let path = s
            .store
            .root
            .join(share::default_name(doc.value["title"].as_str().unwrap()));
        share::save(&path, &share::notebook(&doc).unwrap()).unwrap();
        s.annotations.notebooks.shared = Some(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_uses_the_image_badge_number_across_frames() {
        let value = json!({"sections":[{"figures":[{}, {"callouts":[{"mark_number":3}]}]}]});
        assert_eq!(
            field_label("/sections/0/figures/1/callouts/0/explanation", &value),
            "Capture 1 frame 2 mark 3 explanation"
        );
    }
}
