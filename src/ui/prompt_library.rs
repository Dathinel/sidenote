//! Settings > Prompt library: every prompt Sidenote sends to a model, grouped by job. A list on
//! the left, the selected prompt on the right. You edit the base instructions; the parts Sidenote
//! fixes for safety and for the reply format are shown read only and never saved.
use super::{
    State,
    ai_notes::{self, Editor},
    area::button,
    cards, roles,
    theme::Palette,
    widgets::{self, ButtonDisabled, ButtonRole},
};
use rune::{Color, El, Key, KeyEvent, Tx, col, row};
use sidenote::core::{
    agent_instructions::Agent,
    prompt_library::{self as library, Change, Chunk, Entry, Group, Storage},
};

/// A window narrower than this shows the list, then the prompt, as two steps.
pub const TWO_PANE: f32 = 900.0;
const LIST_WIDTH: f32 = 272.0;
const KEY_PREFIX: &str = "prompt-row-";

#[derive(Default)]
pub struct Library {
    /// The prompt on the right, as a place in the catalog. In a wide window the first prompt is
    /// open until you pick another.
    pub selected: Option<usize>,
    /// Showing the changes from the default instead of the editor.
    pub changes: bool,
    /// Which fixed blocks of the open prompt are expanded.
    pub open: [bool; 4],
}

pub fn single_pane(s: &State) -> bool {
    s.app_w.get() < TWO_PANE
}
fn current(s: &State) -> Option<usize> {
    let selected = s.ai_notes.library.selected;
    if single_pane(s) {
        selected
    } else {
        Some(selected.unwrap_or(0))
    }
}
pub fn select(s: &mut State, index: usize) {
    let entry = &library::catalog()[index];
    let lib = &mut s.ai_notes.library;
    lib.selected = Some(index);
    lib.changes = false;
    // A read-only prompt is nothing but its fixed blocks, so they open by default.
    lib.open = [!entry.editable(); 4];
}
fn back(s: &mut State) {
    s.ai_notes.library.selected = None;
    s.ai_notes.library.changes = false;
}
/// Moves the selection one prompt in list order, which is the order the groups are listed in.
fn step(s: &mut State, delta: isize) {
    let order: Vec<usize> = Group::ALL
        .into_iter()
        .flat_map(|g| library::in_group(g).map(|(i, _)| i))
        .collect();
    let at = current(s).and_then(|i| order.iter().position(|o| *o == i));
    let next = match at {
        Some(at) => (at as isize + delta).clamp(0, order.len() as isize - 1) as usize,
        None => 0,
    };
    select(s, order[next]);
}

/// Arrow keys move through the list while the list or nothing has focus. In a narrow window
/// Escape goes from the editor back to the list.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    let in_list = tx.focused().is_none_or(|f| f.starts_with(KEY_PREFIX));
    match e.key {
        Key::Escape if single_pane(s) && s.ai_notes.library.selected.is_some() => {
            if s.ai_notes.library.changes {
                s.ai_notes.library.changes = false;
            } else {
                back(s);
            }
            tx.blur();
            true
        }
        Key::Up | Key::Down if in_list && !e.mods.ctrl && !e.mods.alt && !e.mods.cmd => {
            if single_pane(s) && s.ai_notes.library.selected.is_none() {
                return false;
            }
            step(s, if e.key == Key::Up { -1 } else { 1 });
            true
        }
        _ => false,
    }
}

fn editor_index(entry: &Entry) -> Option<usize> {
    entry
        .agent()
        .and_then(|a| Agent::ALL.iter().position(|x| *x == a))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Default,
    Edited,
    Unsaved,
    ReadOnly,
}
impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Default => "Default",
            Status::Edited => "Edited",
            Status::Unsaved => "Unsaved",
            Status::ReadOnly => "Read only",
        }
    }
    fn badge(self, p: Palette) -> El {
        let (ink, fill) = match self {
            Status::Default => (p.soft, p.chip),
            Status::Edited => (p.accent, p.sel),
            Status::Unsaved => (p.warn, p.warnbg),
            Status::ReadOnly => (p.faint, p.chip),
        };
        widgets::badge(self.word(), ink, Some(fill))
    }
}
fn status(s: &State, entry: &Entry) -> Status {
    match editor_index(entry).map(|i| &s.ai_notes.editors[i]) {
        None => Status::ReadOnly,
        Some(e) if e.dirty() => Status::Unsaved,
        Some(e) if e.edited_at.is_some() => Status::Edited,
        Some(_) => Status::Default,
    }
}

pub fn view(s: &State, p: Palette) -> El {
    let a = &s.ai_notes;
    let mut c = col()
        .key("agent-instructions")
        .fill(super::hex::settings::group_highlight(
            s,
            "agent-instructions",
            p,
        ))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(12.0)
        .pt(4.0)
        .child(roles::SMALL.wrap(
            "Change how your agent writes. Each prompt has instructions you can edit and parts Sidenote keeps fixed for safety and for the reply format. Edited prompts sync to your other devices.",
            p.soft,
        ));
    if let Some(error) = &a.error
        && a.editor_target.is_none()
    {
        c = c.child(widgets::banner(
            "Prompts need attention",
            error,
            widgets::BannerKind::Problem,
            p,
        ));
    }
    if a.settings.is_none() && a.pending.is_none() {
        c = c.child(
            button("Reload agent status", "ai-notes-reload", p)
                .self_start()
                .on_press(|s: &mut State| s.refresh_ai_notes()),
        );
    }
    if let Some(pending) = a.pending
        && a.editor_target.is_none()
    {
        c = c.child(roles::SMALL.wrap(pending, p.soft).role("status"));
    }
    match (single_pane(s), current(s)) {
        (false, Some(index)) => c.child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .gap(16.0)
                .child(
                    col()
                        .key("prompt-list-pane")
                        .w(LIST_WIDTH)
                        .h_full()
                        .min_h(0.0)
                        .child(widgets::scroll(list(s, p), "prompt-list-scroll")),
                )
                .child(panel(s, p, index).grow(1.0).min_w(0.0).h_full().min_h(0.0)),
        ),
        (true, Some(index)) => c
            .child(
                widgets::pressable(
                    row()
                        .items_center()
                        .gap(6.0)
                        .py(6.0)
                        .child(roles::CONTROL.icon_label("\u{2039}", p.soft))
                        .child(roles::CONTROL.line("All prompts", p.soft)),
                )
                .key("prompt-back")
                .label("Back to all prompts (Esc)")
                .self_start()
                .on_press(back),
            )
            .child(panel(s, p, index).w_full().grow(1.0).min_h(0.0)),
        (_, None) => c.child(widgets::scroll(list(s, p), "prompt-list-scroll")),
    }
}

fn list(s: &State, p: Palette) -> El {
    let selected = current(s);
    let mut c = col()
        .key("prompt-list")
        .w_full()
        .gap(2.0)
        .pb(24.0)
        .role("list");
    for (n, group) in Group::ALL.into_iter().enumerate() {
        c = c.child(
            roles::GROUP
                .line(group.label(), p.faint)
                .role("heading")
                .px(10.0)
                .mt(if n == 0 { 0.0 } else { 16.0 })
                .mb(4.0),
        );
        for (index, entry) in library::in_group(group) {
            let state = status(s, entry);
            let open = selected == Some(index);
            c = c.child(
                widgets::pressable(
                    col()
                        .w_full()
                        .gap(3.0)
                        .px(10.0)
                        .py(8.0)
                        .child(
                            row()
                                .w_full()
                                .items_center()
                                .gap(8.0)
                                .child(roles::NAME.line(entry.name, p.ink).grow(1.0))
                                .child(state.badge(p)),
                        )
                        .child(roles::META.line(entry.purpose, p.soft)),
                )
                .key(format!("{KEY_PREFIX}{}", entry.id))
                .label(format!(
                    "{}, {}. {}{}",
                    entry.name,
                    state.word(),
                    entry.purpose,
                    if open { " Open." } else { "" }
                ))
                .radius(8.0)
                .fill(if open { p.sel } else { Color::TRANSPARENT })
                .hover(p.chip)
                .focus_ring(2.0, 1.0, p.soft)
                .on_press(move |s: &mut State| select(s, index)),
            );
        }
    }
    c
}

fn storage_badge(storage: Storage, p: Palette) -> El {
    match storage {
        Storage::Synced => widgets::badge(storage.label(), p.ok, Some(p.okbg)),
        Storage::ThisDevice | Storage::BuiltIn => {
            widgets::badge(storage.label(), p.soft, Some(p.chip))
        }
    }
}
fn copy_button(text: String, p: Palette) -> El {
    button("Copy prompt", "prompt-copy", p).on_press_tx(move |s: &mut State, tx| {
        tx.copy(text.clone());
        s.show_toast("Copied the whole prompt", tx);
    })
}

fn panel(s: &State, p: Palette, index: usize) -> El {
    let entry = &library::catalog()[index];
    let state = status(s, entry);
    let editor = editor_index(entry);
    let meta = match editor.map(|i| &s.ai_notes.editors[i]) {
        Some(e) => format!(
            "{}. {}",
            ai_notes::edit_time(e.edited_at.as_deref()),
            entry.storage.detail()
        ),
        None => entry.storage.detail().to_owned(),
    };
    let mut top = col()
        .w_full()
        .gap(6.0)
        .px(16.0)
        .pt(14.0)
        .pb(12.0)
        .child(
            row()
                .w_full()
                .items_center()
                .gaps(6.0, 8.0)
                .flex_wrap()
                .child(roles::TITLE.wrap(entry.name, p.ink).role("heading"))
                .child(state.badge(p))
                .child(storage_badge(entry.storage, p)),
        )
        .child(roles::SMALL.wrap(entry.purpose, p.soft))
        .child(roles::META.wrap(meta, p.faint));
    let mut body = col().w_full().gap(14.0).px(16.0).pt(14.0).pb(20.0);
    match editor {
        Some(ei) => {
            let e = &s.ai_notes.editors[ei];
            top = top.child(toolbar(s, entry, ei, e, p));
            let text = &e.text.text;
            top = top.child(counts(entry, ei, text, e.dirty(), p));
            body = editable_body(s, entry, ei, e, body, p);
        }
        None => {
            top = top.child(
                row()
                    .gaps(8.0, 8.0)
                    .flex_wrap()
                    .child(copy_button(entry.assembled(""), p)),
            );
            if let Some(reason) = entry.reason() {
                body = body.child(widgets::banner(
                    if entry.fixed_parts().is_empty() {
                        "Nothing is sent to a model"
                    } else {
                        "Why this prompt is read only"
                    },
                    reason,
                    widgets::BannerKind::Info,
                    p,
                ));
            }
        }
    }
    if entry.id == "tasks-sort" {
        body = body.child(teams_note(s, p));
    }
    let editable = entry.editable();
    if !editable && !entry.fixed_parts().is_empty() {
        body = body.child(fixed_page(s, entry, p));
    }
    cards::panel(p)
        .key(format!("prompt-panel-{}", entry.id))
        .child(top)
        .child(widgets::hairline(p))
        .child(widgets::scroll(
            body,
            &format!("prompt-scroll-{}", entry.id),
        ))
        .children(editable.then(|| widgets::hairline(p)))
        .children(editable.then(|| fixed_footer(s, entry, p)))
}

fn toolbar(s: &State, entry: &Entry, ei: usize, e: &Editor, p: Palette) -> El {
    let a = &s.ai_notes;
    let ready = a.pending.is_none() && a.settings.is_some();
    let valid = ai_notes::valid_instructions(&e.text.text);
    let dirty = e.dirty();
    let changes = a.library.changes;
    let save = widgets::button(
        "Save instructions",
        &format!("instructions-save-{ei}"),
        if dirty {
            ButtonRole::Primary
        } else {
            ButtonRole::Ghost
        },
        p,
    );
    let save = if ready && valid && dirty {
        save.on_press(move |s: &mut State| s.notes_save_editor(ei))
    } else {
        save.button_disabled(true)
    };
    let reset = button("Reset to default", &format!("instructions-reset-{ei}"), p);
    let reset = if ready && (e.edited_at.is_some() || dirty) {
        reset.on_press(move |s: &mut State| s.ai_notes.editors[ei].reset_confirm = true)
    } else {
        reset.button_disabled(true)
    };
    let view = button(
        if changes {
            "Hide changes"
        } else {
            "View changes"
        },
        "prompt-changes",
        p,
    )
    .on_press(|s: &mut State| s.ai_notes.library.changes = !s.ai_notes.library.changes);
    row()
        .w_full()
        .gaps(8.0, 8.0)
        .flex_wrap()
        .child(save)
        .child(reset)
        .child(view)
        .child(copy_button(entry.assembled(&e.text.text), p))
}

fn counts(entry: &Entry, ei: usize, text: &str, dirty: bool, p: Palette) -> El {
    let count = text.chars().count();
    let valid = ai_notes::valid_instructions(text);
    let line = if count > library::MAX_CHARS {
        let over = count - library::MAX_CHARS;
        format!(
            "{over} character{} over the limit. Remove at least {over} to save.",
            if over == 1 { "" } else { "s" }
        )
    } else if !valid {
        "This text contains a character that cannot be saved. Remove it and try again.".into()
    } else {
        format!(
            "{} / 16,000 characters. About {} tokens in your instructions, about {} in the whole prompt.{}",
            ai_notes::count_text(count),
            ai_notes::count_text(library::tokens(text)),
            ai_notes::count_text(library::tokens(&entry.assembled(text))),
            if dirty { " Unsaved changes." } else { "" }
        )
    };
    let mut c = col()
        .w_full()
        .gap(2.0)
        .key(format!("instructions-count-{ei}"))
        .child(roles::META.wrap(line, if valid { p.soft } else { p.warn }));
    if entry.id == "meeting-summary" {
        c = c.child(roles::META.wrap("Summaries are written in English.", p.soft));
    }
    c
}

fn editable_body(s: &State, entry: &Entry, ei: usize, e: &Editor, mut body: El, p: Palette) -> El {
    let a = &s.ai_notes;
    if a.editor_target == Some(ei) {
        body = body
            .children(
                a.pending
                    .map(|text| roles::SMALL.wrap(text, p.soft).role("status")),
            )
            .children(a.error.as_ref().map(|text| {
                widgets::banner(
                    "Instructions were not saved",
                    text,
                    widgets::BannerKind::Problem,
                    p,
                )
            }))
            .children(
                a.notice
                    .as_ref()
                    .map(|text| roles::SMALL.wrap(text, p.ok).role("status")),
            );
    }
    if e.reset_confirm {
        body = body
            .child(roles::SMALL.wrap(
                "Replace your instructions with the default? Your saved copy and unsaved changes will be removed.",
                p.warn,
            ))
            .child(
                row()
                    .flex_wrap()
                    .gaps(8.0, 8.0)
                    .child(
                        button("Reset instructions", &format!("instructions-confirm-{ei}"), p)
                            .color(p.warn)
                            .on_press(move |s: &mut State| s.notes_reset_editor(ei)),
                    )
                    .child(
                        button(
                            "Keep my instructions",
                            &format!("instructions-cancel-{ei}"),
                            p,
                        )
                        .fill(p.sel)
                        .on_press(move |s: &mut State| s.ai_notes.editors[ei].reset_confirm = false),
                    ),
            );
    }
    if a.library.changes {
        body.child(changes_view(entry, e, p))
    } else {
        body.child(input(s, entry, ei, e, p))
    }
}

/// The editable area. It grows with its text, so the page scrolls as one instead of the text
/// scrolling inside a small box, and it is at least about as tall as the window.
fn input(s: &State, entry: &Entry, ei: usize, e: &Editor, p: Palette) -> El {
    let valid = ai_notes::valid_instructions(&e.text.text);
    let editable = s.ai_notes.pending.is_none() && s.ai_notes.settings.is_some();
    let lines = (((s.app_h.get() - 360.0) / roles::SMALL.lh).floor() as usize).clamp(10, 60);
    let field = rune::text_input(&e.text, roles::SMALL.style())
        .key(format!("instructions-input-{ei}"))
        .color(p.ink)
        .wrap_anywhere()
        .caret(p.ink, p.hit)
        .label(format!("{} instructions", entry.name))
        .on_edit(move |s: &mut State| &mut s.ai_notes.editors[ei].text)
        .on_submit(move |s: &mut State, _| s.ai_notes.editors[ei].text.insert("\n"))
        .w_full()
        .min_w(0.0)
        .min_h(lines as f32 * roles::SMALL.lh + 24.0)
        .px(14.0)
        .py(12.0)
        .fill(p.bg)
        .border(1.0, if valid { p.line } else { p.warn })
        .radius(8.0);
    let field = if editable { field } else { field.dim(0.6) };
    col()
        .w_full()
        .gap(6.0)
        .child(roles::GROUP.wrap("Your instructions", p.ink))
        .child(rune::auto_grow(field, lines, 20_000))
}

fn changes_view(entry: &Entry, e: &Editor, p: Palette) -> El {
    let diff = library::diff(&entry.default_base(), &e.text.text);
    let (added, removed) = library::changed(&diff);
    let summary = if added + removed == 0 {
        "No changes. These instructions match the default.".to_owned()
    } else {
        format!(
            "{added} line{} added, {removed} removed.{}",
            if added == 1 { "" } else { "s" },
            if e.dirty() {
                " This includes changes you have not saved."
            } else {
                ""
            }
        )
    };
    let mut c = col()
        .key("prompt-diff")
        .w_full()
        .gap(6.0)
        .child(roles::GROUP.wrap("Your instructions compared with the default", p.ink))
        .child(roles::META.wrap(summary, p.soft).role("status"));
    let mut rows = col()
        .w_full()
        .py(4.0)
        .radius(8.0)
        .clip_rounded(8.0)
        .border(1.0, p.line)
        .fill(p.bg);
    for chunk in library::with_context(&diff, 2) {
        rows = rows.child(match chunk {
            Chunk::Skipped(n) => roles::META
                .wrap(
                    format!(
                        "{n} unchanged line{} not shown",
                        if n == 1 { "" } else { "s" }
                    ),
                    p.faint,
                )
                .px(12.0)
                .py(4.0),
            Chunk::Line(line) => {
                let (sign, ink, fill) = match line.change {
                    Change::Added => ("+", p.ok, p.okbg),
                    Change::Removed => ("\u{2212}", p.warn, p.warnbg),
                    Change::Same => (" ", p.faint, Color::TRANSPARENT),
                };
                row()
                    .w_full()
                    .items_start()
                    .gap(8.0)
                    .px(12.0)
                    .py(2.0)
                    .fill(fill)
                    .child(roles::CONTROL.line(sign, ink).w(12.0))
                    .child(
                        roles::SMALL
                            .wrap(
                                line.text,
                                if line.change == Change::Same {
                                    p.soft
                                } else {
                                    p.ink
                                },
                            )
                            .wrap_anywhere()
                            .grow(1.0)
                            .min_w(0.0)
                            .min_h(roles::SMALL.lh),
                    )
            }
        });
    }
    if added + removed > 0 {
        c = c.child(rows);
    }
    c
}

fn fixed_title(p: Palette) -> El {
    row()
        .items_center()
        .gap(6.0)
        .child(super::icons::draw(&super::icons::LOCK, 14.0, p.soft))
        .child(
            roles::GROUP
                .wrap("Fixed by Sidenote", p.ink)
                .role("heading"),
        )
}
fn fixed_block(text: String, p: Palette) -> El {
    col()
        .w_full()
        .px(14.0)
        .py(12.0)
        .radius(8.0)
        .fill(p.bg)
        .border(1.0, p.line)
        .child(
            roles::SMALL
                .wrap(text, p.soft)
                .wrap_anywhere()
                .selectable(true),
        )
}
fn toggle_fixed(s: &mut State, i: usize) {
    let open = &mut s.ai_notes.library.open[i];
    *open = !*open;
}

/// A read-only prompt is all fixed text, so it fills the page under its reason.
fn fixed_page(s: &State, entry: &Entry, p: Palette) -> El {
    let mut c = col()
        .key("prompt-fixed")
        .w_full()
        .gap(2.0)
        .child(fixed_title(p))
        .child(roles::META.wrap(
            "Shown exactly as Sidenote sends it, apart from the meeting material it adds.",
            p.soft,
        ));
    for (i, part) in entry.fixed_parts().into_iter().enumerate().take(4) {
        let open = s.ai_notes.library.open[i];
        c = c.child(
            cards::fold_control(part.label, &format!("prompt-fixed-{i}"), open, p)
                .on_press(move |s: &mut State| toggle_fixed(s, i)),
        );
        if open {
            c = c.child(fixed_block(part.text, p));
        }
    }
    c
}

/// An editable prompt keeps its fixed parts one line below the editor, collapsed, so the whole
/// prompt is a click away however long your instructions are.
fn fixed_footer(s: &State, entry: &Entry, p: Palette) -> El {
    let mut toggles = row().w_full().items_center().gaps(4.0, 6.0).flex_wrap();
    let mut blocks = col().w_full().gap(8.0);
    let mut any = false;
    for (i, part) in entry.fixed_parts().into_iter().enumerate().take(4) {
        let open = s.ai_notes.library.open[i];
        toggles = toggles.child(
            widgets::pressable(
                row()
                    .items_center()
                    .gap(4.0)
                    .px(8.0)
                    .py(5.0)
                    .child(roles::CONTROL.line(if open { "\u{25be}" } else { "\u{25b8}" }, p.soft))
                    .child(roles::CONTROL.line(part.label, p.ink)),
            )
            .key(format!("prompt-fixed-{i}"))
            .label(format!("{} (read only)", part.label))
            .expanded(open)
            .radius(6.0)
            .border(1.0, p.line)
            .hover(p.chip)
            .focus_ring(2.0, 1.0, p.soft)
            .on_press(move |s: &mut State| toggle_fixed(s, i)),
        );
        if open {
            any = true;
            blocks = blocks
                .child(roles::GROUP.wrap(part.label, p.soft))
                .child(fixed_block(part.text, p));
        }
    }
    col()
        .key("prompt-fixed")
        .w_full()
        .gap(8.0)
        .px(16.0)
        .py(10.0)
        .child(fixed_title(p))
        .child(
            roles::META
                .wrap(
                    "Read only. Sidenote adds these around your instructions, so a saved edit can never remove them.",
                    p.soft,
                )
                .w_full(),
        )
        .child(toggles)
        .children(any.then(|| {
            col()
                .key("prompt-fixed-scroll")
                .w_full()
                .shrink()
                .max_h(170.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(blocks)
        }))
}

fn teams_note(s: &State, p: Palette) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(roles::SMALL.wrap(
            if s.tasks.teams.teams.is_empty() {
                "No teams yet. Tasks will not be given a team."
            } else {
                "Tasks use the team of the person who owns them."
            },
            p.soft,
        ))
        .children(s.config.features.tasks.then(|| {
            button("Set up teams", "notes-teams", p)
                .self_start()
                .on_press(|s: &mut State| {
                    s.hex.section = super::hex::Section::Teams;
                    s.hex.filter = rune::TextEdit::default();
                    s.nav_done();
                    super::scene_scroll(s, "hex-top");
                })
        }))
}

/// Invented states for the render harness. The isolated state holds no saved instructions, so
/// the scenes set what a few edited prompts would look like after loading.
pub fn scene(s: &mut State, name: &str) {
    let edit = |s: &mut State, id: &str, text: String| {
        let entry = library::find(id).expect("scene prompt");
        let e = &mut s.ai_notes.editors[editor_index(entry).expect("editable prompt")];
        e.text = rune::TextEdit::new(&text);
        e.saved = text;
        e.edited_at = Some("2026-11-10T04:00:00Z".into());
    };
    let summary = library::find("meeting-summary").unwrap();
    let mut lines: Vec<String> = summary.default_base().lines().map(str::to_owned).collect();
    lines[0] = "Write as someone who sat in the meeting and explains it to a colleague who did not: short, plain and accurate. Write the notes in English, even when the transcript is in another language.".into();
    lines.retain(|l| !l.starts_with("- Note the intent behind the talk"));
    lines.push("- Keep every number and date exactly as said.".into());
    edit(s, "meeting-summary", lines.join("\n"));
    edit(
        s,
        "ask-meeting",
        "Lead with the answer in one or two sentences. Use British spelling.".into(),
    );
    let index = |id: &str| {
        library::catalog()
            .iter()
            .position(|e| e.id == id)
            .expect("scene prompt")
    };
    match name {
        "settings-prompts" => {}
        "settings-prompts-edited" => select(s, index("meeting-summary")),
        "settings-prompts-diff" => {
            select(s, index("meeting-summary"));
            s.ai_notes.library.changes = true;
        }
        "settings-prompts-sort" => {
            select(s, index("tasks-sort"));
            s.ai_notes.library.open[0] = true;
        }
        "settings-prompts-readonly" => select(s, index("daily-check")),
        _ => panic!("Unknown prompt library scene"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_editable_prompt_has_one_editor_and_read_only_prompts_have_none() {
        let mut slots = Vec::new();
        for entry in library::catalog() {
            match editor_index(entry) {
                Some(i) => {
                    assert!(entry.editable(), "{}", entry.id);
                    slots.push(i);
                }
                None => assert!(!entry.editable(), "{}", entry.id),
            }
        }
        slots.sort_unstable();
        assert_eq!(slots, (0..Agent::ALL.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_read_only_prompt_opens_its_fixed_blocks_and_an_editable_one_keeps_them_closed() {
        let mut state = State::new();
        for (index, entry) in library::catalog().iter().enumerate() {
            select(&mut state, index);
            assert_eq!(state.ai_notes.library.selected, Some(index));
            assert!(!state.ai_notes.library.changes);
            assert_eq!(state.ai_notes.library.open[0], !entry.editable());
        }
    }
}
