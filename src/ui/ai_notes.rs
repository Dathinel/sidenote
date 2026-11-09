//! AI notes controls. Disk access and helper calls return through one worker mailbox.
use super::{Event, State, area::button, roles, theme::Palette};
use rune::{El, TextEdit, col, row};
use rune_kit::{Kit, Switch, TextInput};
use sidenote::core::{
    agent_instructions::{self as instructions, Agent},
    notes::{self, Helper, NotesSettings, Request},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    thread,
};

pub const HELPERS: [Helper; 3] = [Helper::Claude, Helper::Codex, Helper::OpenCode];
pub const PRIVACY: &str = "When AI notes are on, Sidenote sends each meeting's text transcript and your typed notes to the helper you choose. The helper sends them to its AI provider under your own account. Audio never leaves this PC. Sidenote has no server of its own.";

pub struct Editor {
    pub text: TextEdit,
    pub saved: String,
    pub edited_at: Option<String>,
    pub fixed_open: bool,
    pub reset_confirm: bool,
}
impl Editor {
    fn new(agent: Agent) -> Self {
        let text = instructions::default_base(agent).to_string();
        Self {
            text: TextEdit::new(&text),
            saved: text,
            edited_at: None,
            fixed_open: false,
            reset_confirm: false,
        }
    }
    fn loaded(&mut self, agent: Agent, entry: Option<instructions::Entry>, reset: bool) {
        let text = entry
            .as_ref()
            .map(|e| e.text.clone())
            .unwrap_or_else(|| instructions::default_base(agent).into());
        if reset || !self.dirty() {
            self.text = TextEdit::new(&text);
        }
        self.saved = text;
        self.edited_at = entry.map(|e| e.edited_at);
        self.reset_confirm = false;
    }
    fn dirty(&self) -> bool {
        self.text.text != self.saved
    }
}
pub struct AiNotes {
    pub settings: Option<NotesSettings>,
    pub found: Vec<(Helper, bool)>,
    pub models: [TextEdit; 3],
    pub name: TextEdit,
    pub editors: [Editor; 2],
    pub pending: Option<&'static str>,
    pub editor_target: Option<usize>,
    pub testing: Option<usize>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub test_results: BTreeMap<usize, String>,
    pub jobs: BTreeMap<String, super::notes_status::Job>,
    pub dismissed: BTreeSet<String>,
    pub switch_confirm: Option<Helper>,
    pub generation: u64,
    pub scene: bool,
}
impl Default for AiNotes {
    fn default() -> Self {
        Self {
            settings: None,
            found: Vec::new(),
            models: HELPERS.map(|h| TextEdit::new(h.default_model())),
            name: TextEdit::default(),
            editors: [Editor::new(Agent::Notes), Editor::new(Agent::Tasks)],
            pending: None,
            editor_target: None,
            testing: None,
            error: None,
            notice: None,
            test_results: BTreeMap::new(),
            jobs: BTreeMap::new(),
            dismissed: BTreeSet::new(),
            switch_confirm: None,
            generation: 0,
            scene: false,
        }
    }
}
pub struct Snapshot {
    settings: NotesSettings,
    found: Vec<(Helper, bool)>,
    store: instructions::Store,
}
const INSTRUCTIONS_LOAD_ERROR: &str =
    "Could not read agent instructions. Your saved text was kept.";

fn instructions_warning(store: &instructions::Store) -> Option<String> {
    store
        .using_defaults_after_error
        .then(|| INSTRUCTIONS_LOAD_ERROR.to_owned())
}
impl Snapshot {
    fn read(root: &Path) -> Result<Self, String> {
        Ok(Self {
            settings: notes::settings()
                .map_err(|_| "Could not read AI notes settings. Try Reload.".to_string())?,
            found: notes::detect(),
            store: instructions::load(root).map_err(|_| INSTRUCTIONS_LOAD_ERROR.to_string())?,
        })
    }
}
pub enum Completion {
    Loaded(Box<Snapshot>),
    Settings(NotesSettings),
    Instructions(usize, Option<instructions::Entry>, bool),
    Tested(usize, Result<std::time::Duration, String>),
    Requested(String),
}
enum Work {
    Load,
    Settings(NotesSettings),
    Save(usize, String),
    Reset(usize),
    Test(usize, String),
    Request(String, Request),
}
fn agent(index: usize) -> Agent {
    if index == 0 {
        Agent::Notes
    } else {
        Agent::Tasks
    }
}
fn run(root: &Path, work: Work) -> Result<Completion, String> {
    match work {
        Work::Load => Snapshot::read(root).map(|s| Completion::Loaded(Box::new(s))),
        Work::Settings(settings) => {
            notes::save_settings(&settings).map_err(|_| {
                "Could not save AI notes settings. Your previous choice was kept.".to_string()
            })?;
            notes::settings().map(Completion::Settings).map_err(|_| {
                "Settings were written but could not be read back. Reload to check.".into()
            })
        }
        Work::Save(index, text) => {
            instructions::save(root, agent(index), &text).map_err(|_| {
                "Could not save instructions. Check the length and try again.".to_string()
            })?;
            let store = instructions::load(root).map_err(|_| {
                "Instructions were written but could not be read back. Reload to check.".to_string()
            })?;
            Ok(Completion::Instructions(
                index,
                store.get(agent(index)).cloned(),
                false,
            ))
        }
        Work::Reset(index) => {
            instructions::reset(root, agent(index))
                .map_err(|_| "Could not reset instructions. Your text was kept.".to_string())?;
            let store = instructions::load(root).map_err(|_| {
                "Instructions were reset but could not be read back. Reload to check.".to_string()
            })?;
            Ok(Completion::Instructions(
                index,
                store.get(agent(index)).cloned(),
                true,
            ))
        }
        Work::Test(index, model) => Ok(Completion::Tested(
            index,
            notes::test_helper(HELPERS[index], &model),
        )),
        Work::Request(id, mode) => {
            notes::request(&id, mode).map_err(|_| "Could not queue notes. Choose an installed helper in AI notes settings, then try again.".to_string())?;
            Ok(Completion::Requested(id))
        }
    }
}
impl State {
    fn notes_work(&mut self, work: Work, label: &'static str) {
        if self.ai_notes.pending.is_some() || self.ai_notes.scene {
            return;
        }
        self.ai_notes.pending = Some(label);
        self.ai_notes.editor_target = match &work {
            Work::Save(i, _) | Work::Reset(i) => Some(*i),
            _ => None,
        };
        self.ai_notes.testing = match &work {
            Work::Test(i, _) => Some(*i),
            _ => None,
        };
        self.ai_notes.error = None;
        self.ai_notes.notice = None;
        self.ai_notes.generation += 1;
        let generation = self.ai_notes.generation;
        let (root, events, waker) = (
            self.store.root.clone(),
            self.events_tx.clone(),
            self.waker.clone(),
        );
        if thread::Builder::new()
            .name("ai-notes-operation".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| run(&root, work)).unwrap_or_else(|_| {
                    Err("AI notes hit a problem. Reload before trying again.".into())
                });
                let _ = events.send(Event::AiNotes {
                    generation,
                    worker: thread::current().id(),
                    result,
                });
                if let Some(waker) = waker {
                    waker();
                }
            })
            .is_err()
        {
            self.ai_notes.pending = None;
            self.ai_notes.error = Some("Could not start the AI notes worker. Try again.".into());
        }
    }
    pub fn refresh_ai_notes(&mut self) {
        self.notes_work(Work::Load, "Loading AI notes...");
    }
    pub fn open_ai_notes(&mut self) {
        if self.ai_notes.scene {
            self.go_home();
            self.hex_open = true;
        } else {
            self.open_hex();
        }
        self.hex.section = super::hex::Section::AiNotes;
        self.hex.filter = TextEdit::default();
        self.nav_done();
        super::scene_scroll(self, "ai-notes-settings");
        if self.ai_notes.settings.is_none() {
            self.refresh_ai_notes();
        }
    }
    pub fn notes_finished(&mut self, generation: u64, result: Result<Completion, String>) {
        if generation != self.ai_notes.generation {
            return;
        }
        self.ai_notes.pending = None;
        match result {
            Err(error) => self.ai_notes.error = Some(error),
            Ok(Completion::Loaded(snapshot)) => {
                let s = &mut self.ai_notes;
                s.error = instructions_warning(&snapshot.store);
                for (index, h) in HELPERS.iter().enumerate() {
                    s.models[index] = TextEdit::new(if snapshot.settings.helper == Some(*h) {
                        &snapshot.settings.model
                    } else {
                        h.default_model()
                    });
                }
                s.name = TextEdit::new(&snapshot.settings.your_name);
                s.settings = Some(snapshot.settings);
                s.found = snapshot.found;
                for index in 0..2 {
                    self.notes_editor_loaded(
                        index,
                        snapshot.store.get(agent(index)).cloned(),
                        false,
                    );
                }
            }
            Ok(Completion::Settings(settings)) => {
                self.config.notes = Some(settings.clone());
                self.ai_notes.settings = Some(settings);
                self.ai_notes.notice = Some("AI notes settings saved.".into());
            }
            Ok(Completion::Instructions(index, entry, reset)) => {
                self.notes_editor_loaded(index, entry, reset);
                self.ai_notes.notice = Some("Agent instructions saved.".into());
            }
            Ok(Completion::Tested(index, result)) => {
                let text = match result {
                    Ok(duration) => format!(
                        "Worked in {} s. Only fixed sample text was sent.",
                        duration.as_secs().max(1)
                    ),
                    Err(error) => test_error(&error),
                };
                self.ai_notes.test_results.insert(index, text);
            }
            Ok(Completion::Requested(id)) => {
                self.ai_notes
                    .jobs
                    .insert(id.clone(), super::notes_status::Job::queued());
                self.ai_notes.dismissed.remove(&id);
                self.request_refresh();
                if let Some(handle) = &self.scan_tx {
                    handle.invalidate();
                }
            }
        }
    }
    fn notes_editor_loaded(
        &mut self,
        index: usize,
        entry: Option<instructions::Entry>,
        reset: bool,
    ) {
        self.ai_notes.editors[index].loaded(agent(index), entry, reset);
    }
    fn notes_save_editor(&mut self, index: usize) {
        let text = self.ai_notes.editors[index].text.text.clone();
        if !valid_instructions(&text) {
            return;
        }
        self.notes_work(Work::Save(index, text), "Saving instructions...");
    }
    pub fn choose_notes_helper(&mut self, index: usize) {
        let Some(mut settings) = self.ai_notes.settings.clone() else {
            return;
        };
        if !self.ai_notes.found.contains(&(HELPERS[index], true)) {
            return;
        }
        if settings.engine == "scripts" && self.ai_notes.switch_confirm != Some(HELPERS[index]) {
            self.ai_notes.switch_confirm = Some(HELPERS[index]);
            return;
        }
        self.ai_notes.switch_confirm = None;
        settings.engine = "app".into();
        settings.helper = Some(HELPERS[index]);
        settings.model = self.ai_notes.models[index].text.trim().into();
        settings.your_name = self.ai_notes.name.text.trim().into();
        self.notes_work(Work::Settings(settings), "Saving AI notes...");
    }
    pub fn request_notes(&mut self, id: String, mode: Request) {
        self.notes_work(Work::Request(id, mode), "Adding notes to the queue...");
    }
}
pub(super) fn test_error(message: &str) -> String {
    if message.to_ascii_lowercase().contains("json") {
        "The helper's test answer could not be read. Try Test again or check its setup.".into()
    } else {
        message.into()
    }
}
pub fn valid_instructions(text: &str) -> bool {
    text.chars().count() <= instructions::MAX_CHARS && !text.contains('\0')
}
fn field(edit: &TextEdit, label: &str) -> TextInput {
    TextInput::new(edit).label(label).placeholder(label)
}
fn count_text(count: usize) -> String {
    if count < 1000 {
        count.to_string()
    } else {
        format!("{},{:03}", count / 1000, count % 1000)
    }
}
fn card(p: Palette) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(10.0)
        .px(16.0)
        .py(16.0)
        .radius(10.0)
        .border(1.0, p.line)
        .fill(p.card)
}
fn edit_time(time: Option<&str>) -> String {
    time.and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("Edited %-d %b, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "Default".into())
}
fn editor(s: &State, index: usize, p: Palette) -> El {
    let e = &s.ai_notes.editors[index];
    let a = agent(index);
    let count = e.text.text.chars().count();
    let valid = valid_instructions(&e.text.text);
    let editable = s.ai_notes.pending.is_none() && s.ai_notes.settings.is_some();
    let mut c = card(p)
        .key(format!("instructions-{index}"))
        .child(roles::TITLE.wrap(if index == 0 { "Meeting notes" } else { "Tasks" }, p.ink))
        .child(roles::META.wrap(edit_time(e.edited_at.as_deref()), p.soft))
        .child(roles::META.wrap(
            "Scroll inside the box to read the full instructions.",
            p.soft,
        ));
    let input = rune::text_input(&e.text, roles::SMALL.style())
        .key(format!("instructions-input-{index}"))
        .min_h(8.0 * roles::SMALL.lh)
        .color(p.ink)
        .wrap_anywhere()
        .caret(p.ink, p.hit)
        .label(if index == 0 {
            "Meeting notes instructions"
        } else {
            "Tasks instructions"
        })
        .on_edit(move |s: &mut State| &mut s.ai_notes.editors[index].text)
        .on_submit(move |s: &mut State, _| s.ai_notes.editors[index].text.insert("\n"))
        .w_full()
        .min_w(0.0)
        .px(12.0)
        .py(12.0)
        .fill(p.bg)
        .border(1.0, if valid { p.line } else { p.warn })
        .radius(6.0);
    c = c
        .child(rune::auto_grow(input, 8, 12))
        .child(roles::META.wrap(
            format!(
                "{} / 16,000 characters{}",
                count_text(count),
                if e.dirty() { ". Unsaved changes" } else { "" }
            ),
            if valid { p.soft } else { p.warn },
        ));
    if !valid {
        c = c.child(
            roles::SMALL
                .wrap(
                    if count > instructions::MAX_CHARS { "Use at most 16,000 characters before saving." } else { "This text contains a character that cannot be saved. Remove it and try again." },
                    p.warn,
                )
                .role("alert"),
        );
    }
    let mut actions = row().w_full().flex_wrap().gaps(8.0, 8.0);
    let save = button(
        "Save instructions",
        &format!("instructions-save-{index}"),
        p,
    );
    actions = actions.child(if editable && valid && e.dirty() {
        save.on_press(move |s: &mut State| s.notes_save_editor(index))
    } else {
        save.dim(0.5)
    });
    let reset = button(
        "Reset to default",
        &format!("instructions-reset-{index}"),
        p,
    );
    actions = actions.child(if editable {
        reset.on_press(move |s: &mut State| s.ai_notes.editors[index].reset_confirm = true)
    } else {
        reset.dim(0.5)
    });
    actions = actions.child(
        button(
            if e.fixed_open {
                "Hide the fixed part"
            } else {
                "Show the fixed part"
            },
            &format!("instructions-fixed-{index}"),
            p,
        )
        .on_press(move |s: &mut State| {
            let e = &mut s.ai_notes.editors[index];
            e.fixed_open = !e.fixed_open;
        }),
    );
    c = c.child(actions);
    if s.ai_notes.editor_target == Some(index) {
        c = c
            .children(
                s.ai_notes
                    .pending
                    .map(|text| roles::SMALL.wrap(text, p.soft).role("status")),
            )
            .children(
                s.ai_notes
                    .error
                    .as_ref()
                    .map(|text| roles::SMALL.wrap(text, p.warn).role("alert")),
            )
            .children(
                s.ai_notes
                    .notice
                    .as_ref()
                    .map(|text| roles::SMALL.wrap(text, p.ok).role("status")),
            );
    }
    if e.reset_confirm {
        c = c.child(roles::SMALL.wrap("Replace your instructions with the default? Your saved copy and unsaved changes will be removed.", p.warn))
            .child(row().flex_wrap().gaps(8.0, 8.0)
                .child(button("Reset instructions", &format!("instructions-confirm-{index}"), p).on_press(move |s: &mut State| s.notes_work(Work::Reset(index), "Resetting instructions...")))
                .child(button("Keep my instructions", &format!("instructions-cancel-{index}"), p).on_press(move |s: &mut State| s.ai_notes.editors[index].reset_confirm = false)));
    }
    if e.fixed_open {
        c = c
            .child(roles::GROUP.wrap("Fixed safety rules (read only)", p.ink))
            .child(roles::SMALL.wrap(instructions::core_head(a), p.soft))
            .child(roles::GROUP.wrap("Your instructions go here", p.accent))
            .child(roles::GROUP.wrap("Fixed output format (read only)", p.ink))
            .child(roles::SMALL.wrap(instructions::core_tail(a), p.soft));
    }
    c
}
pub fn section(s: &State, k: &Kit, p: Palette) -> El {
    let a = &s.ai_notes;
    let mut c = col()
        .key("ai-notes-settings")
        .w_full()
        .gap(16.0)
        .mt(20.0)
        .child(roles::SECTION.wrap("AI notes", p.ink));
    if let Some(pending) = a.pending {
        c = c.child(roles::SMALL.wrap(pending, p.soft).role("status"));
    }
    if let Some(error) = &a.error {
        c = c.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    if let Some(notice) = &a.notice {
        c = c.child(roles::SMALL.wrap(notice, p.ok).role("status"));
    }
    c = c.child(
        button("Reload helper status", "ai-notes-reload", p)
            .self_start()
            .on_press(|s: &mut State| s.refresh_ai_notes()),
    );
    let Some(settings) = &a.settings else {
        return c;
    };
    let status = match (settings.engine.as_str(), settings.helper) {
        ("app", Some(h)) => format!(
            "AI notes are on. Notes are written by {} ({}).",
            h.name(),
            if settings.model.is_empty() {
                if h == Helper::OpenCode {
                    "configured model"
                } else {
                    h.default_model()
                }
            } else {
                &settings.model
            }
        ),
        ("scripts", _) => "Your own scripts write notes on this PC.".into(),
        _ => "AI notes are off. New meetings get a transcript only.".into(),
    };
    c = c.child(roles::SMALL.wrap(status, p.ink));
    if let Some((_, job)) = a
        .jobs
        .iter()
        .filter(|(_, j)| super::notes_status::is_failure(&j.state))
        .max_by_key(|(_, j)| j.updated_at)
    {
        c = c.child(roles::SMALL.wrap(
            format!(
                "Last error: {}",
                super::notes_status::message(job, settings.helper)
            ),
            p.warn,
        ));
    }
    c = c.child(card(p).key("notes-privacy")
        .child(roles::TITLE.wrap("What is sent", p.ink)).child(roles::SMALL.wrap(PRIVACY, p.ink))
        .child(roles::SMALL.wrap("Text is sent only after you choose Use this, for meetings that end afterwards or when you click Make notes or Try again. Test sends only fixed sample text. With no helper, nothing is sent.", p.soft))
        .child(roles::SMALL.wrap("Each request also includes the title, date, time, length, glossary, teams and members, projects, your name if set, and agent instructions. Other meetings, audio, keys and tokens are never included.", p.soft))
        .child(roles::SMALL.wrap("Claude Code uses Anthropic with all tools off. Codex uses OpenAI, and OpenCode uses its configured provider. Both have read-only tools. A transcript could ask those tools to read a file and copy it into notes. Notes and transcripts stay on this PC and your synced devices. Job states stay on this PC. Logs hold no meeting text or helper output.", p.soft))
        .child(roles::SMALL.wrap("Notes are written in English. Audio translation is not included in this version.", p.soft)));
    c = c.child(roles::TITLE.wrap("Make notes with", p.ink));
    for (index, h) in HELPERS.iter().copied().enumerate() {
        let found = a.found.contains(&(h, true));
        let selected = settings.engine == "app" && settings.helper == Some(h);
        let mut r = card(p)
            .key(format!("notes-helper-{index}"))
            .child(roles::TITLE.wrap(
                format!(
                    "{}{}",
                    h.name(),
                    if index == 0 { " (recommended)" } else { "" }
                ),
                p.ink,
            ))
            .child(roles::META.wrap(
                format!(
                    "{}{}",
                    if found { "Found" } else { "Not found" },
                    if selected { ". In use" } else { "" }
                ),
                p.soft,
            ))
            .child(roles::SMALL.wrap(
                if found {
                    match h {
                        Helper::Claude => "Uses your Claude Code account. To sign in again, run claude in a terminal.",
                        Helper::Codex => "Uses your Codex account. To sign in again, run codex in a terminal.",
                        Helper::OpenCode => "Uses the provider and account you chose in OpenCode.",
                    }
                } else {
                    match h {
                        Helper::Claude => "Install Claude Code, then run claude in a terminal and sign in.",
                        Helper::Codex => "Install Codex CLI, then run codex in a terminal and sign in.",
                        Helper::OpenCode => "Install OpenCode, then run opencode in a terminal and choose a provider.",
                    }
                },
                p.soft,
            ));
        if h == Helper::OpenCode {
            r = r.child(roles::META.wrap(
                "OpenCode uses its own configured model. Change it in OpenCode.",
                p.soft,
            ));
        } else {
            r = r.child(roles::META.wrap("Model name", p.soft)).child(
                field(&a.models[index], "Model name")
                    .on_edit(move |s: &mut State| &mut s.ai_notes.models[index])
                    .build(k)
                    .key(format!("notes-model-{index}"))
                    .w_full(),
            );
        }
        let mut actions = row().flex_wrap().gaps(8.0, 8.0);
        let use_this = button(
            if selected {
                "Save model and use this"
            } else {
                "Use this"
            },
            &format!("notes-use-{index}"),
            p,
        );
        let test = button("Test", &format!("notes-test-{index}"), p);
        if found && a.pending.is_none() {
            actions = actions
                .child(use_this.on_press(move |s: &mut State| s.choose_notes_helper(index)))
                .child(test.on_press(move |s: &mut State| {
                    let model = s.ai_notes.models[index].text.clone();
                    s.notes_work(Work::Test(index, model), "Testing fixed sample text...");
                }));
        } else {
            actions = actions.child(use_this.dim(0.5)).child(test.dim(0.5));
        }
        r = r.child(actions);
        if a.testing == Some(index) && a.pending.is_some() {
            r = r.child(
                roles::SMALL
                    .wrap("Testing fixed sample text...", p.soft)
                    .role("status"),
            );
        }
        if let Some(result) = a.test_results.get(&index) {
            r = r.child(
                roles::SMALL
                    .wrap(
                        result,
                        if result.starts_with("Worked in ") {
                            p.ok
                        } else {
                            p.warn
                        },
                    )
                    .role("status"),
            );
        }
        if a.switch_confirm == Some(h) {
            r = r.child(roles::SMALL.wrap("If your scripts also run from somewhere else, such as a scheduled task, turn that off, or both will write notes.", p.warn))
                .child(row().flex_wrap().gaps(8.0, 8.0)
                    .child(button("Switch to built-in notes", "notes-confirm-switch", p).on_press(move |s: &mut State| s.choose_notes_helper(index)))
                    .child(button("Keep scripts", "notes-cancel-switch", p).on_press(|s: &mut State| s.ai_notes.switch_confirm = None)));
        }
        c = c.child(r);
    }
    if settings.script.is_some() {
        c = c.child(
            card(p)
                .key("notes-scripts-card")
                .child(roles::TITLE.wrap("Your own scripts (advanced)", p.ink))
                .child(roles::SMALL.wrap(
                    "Use your configured script. Built-in notes will stop starting new jobs.",
                    p.soft,
                ))
                .child(
                    button("Use scripts", "notes-scripts", p)
                        .self_start()
                        .on_press(|s: &mut State| {
                            if let Some(mut settings) = s.ai_notes.settings.clone() {
                                settings.engine = "scripts".into();
                                s.notes_work(Work::Settings(settings), "Switching to scripts...");
                            }
                        }),
                ),
        );
    }
    c = c.child(
        card(p)
            .child(roles::TITLE.wrap("Off", p.ink))
            .child(roles::SMALL.wrap("Keep new meetings as transcripts only.", p.soft))
            .child(
                button("Turn off AI notes", "notes-off", p)
                    .self_start()
                    .on_press(|s: &mut State| {
                        if let Some(mut settings) = s.ai_notes.settings.clone() {
                            settings.engine = "off".into();
                            s.notes_work(Work::Settings(settings), "Turning off AI notes...");
                        }
                    }),
            ),
    );
    c = c.child(card(p).key("notes-options").child(roles::NAME.wrap("Make notes automatically when a meeting ends", p.ink))
        .child(Switch::new(settings.auto).label("Make notes automatically").on_toggle(|s: &mut State| {
            if let Some(mut settings) = s.ai_notes.settings.clone() { settings.auto = !settings.auto; s.notes_work(Work::Settings(settings), "Saving automatic notes..."); }
        }).build(k).key("notes-auto"))
        .child(roles::SMALL.wrap("When this is off, use Make notes on each meeting.", p.soft))
        .child(roles::NAME.wrap("Your name (optional)", p.ink))
        .child(field(&a.name, "Your name").on_edit(|s: &mut State| &mut s.ai_notes.name).build(k).key("notes-name").w_full())
        .child(roles::META.wrap("Used to mark tasks that are yours. Stored on this PC and included when a meeting is sent to your helper.", p.soft))
        .child(button("Save name", "notes-save-name", p).self_start().on_press(|s: &mut State| {
            if let Some(mut settings) = s.ai_notes.settings.clone() { settings.your_name = s.ai_notes.name.text.trim().into(); s.notes_work(Work::Settings(settings), "Saving your name..."); }
        })));
    c.child(col().key("agent-instructions").w_full().gap(16.0)
        .child(roles::SECTION.wrap("Agent instructions", p.ink))
        .child(roles::SMALL.wrap("Change the base instructions for each agent. Safety rules and the output format stay fixed. Instructions sync to your other devices.", p.soft))
        .child(editor(s, 0, p)).child(editor(s, 1, p)))
        .child(roles::SMALL.wrap("Teams: when no teams are set up, notes leave the team blank.", p.soft))
        .children(s.config.features.tasks.then(|| button("Open Tasks for teams", "notes-teams", p).self_start().on_press(|s: &mut State| {
            s.hex.section = super::hex::Section::Teams;
            s.hex.filter = TextEdit::default();
            s.nav_done();
            super::scene_scroll(s, "hex-top");
        })))
}
pub fn setup_card(s: &State, p: Palette) -> Option<El> {
    let a = &s.ai_notes;
    let settings = a.settings.as_ref()?;
    if settings.engine == "scripts" || (settings.engine == "app" && settings.helper.is_some()) {
        return None;
    }
    let helper = HELPERS.into_iter().find(|h| a.found.contains(&(*h, true)));
    let text = helper.map(|h| format!("{} is installed. Sidenote can use it to write notes for your meetings.", h.name())).unwrap_or_else(|| "Sidenote writes notes with an AI helper you already use: Claude Code, Codex or OpenCode. None was found on this PC.".into());
    Some(card(p).key("today-ai-notes").mb(16.0)
        .child(roles::TITLE.wrap("Turn on AI notes", p.ink)).child(roles::SMALL.wrap(text, p.soft))
        .child(roles::SMALL.wrap("Your meeting text goes to the helper's AI provider under your account after you turn notes on.", p.soft))
        .child(row().flex_wrap().gaps(8.0, 8.0)
            .child(button(if helper.is_some() { "Turn on AI notes" } else { "How to set one up" }, "today-notes-setup", p).on_press(|s: &mut State| s.open_ai_notes()))
            .child(button("What is sent", "today-notes-privacy", p).on_press(|s: &mut State| s.open_ai_notes()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instruction_fallback_reaches_existing_banner_without_blocking_settings() {
        assert!(instructions_warning(&instructions::Store::default()).is_none());
        let mut store = instructions::Store::default();
        store.using_defaults_after_error = true;
        assert_eq!(
            instructions_warning(&store).as_deref(),
            Some(INSTRUCTIONS_LOAD_ERROR)
        );
    }
    #[test]
    fn unreadable_test_answers_use_plain_words() {
        assert!(!test_error("The helper test returned invalid JSON").contains("JSON"));
        assert_eq!(
            test_error("The helper was not found on this PC"),
            "The helper was not found on this PC"
        );
    }

    #[test]
    fn instruction_limit_counts_unicode_and_refuses_nul() {
        assert!(valid_instructions(&"\u{e9}".repeat(16_000)));
        assert!(!valid_instructions(&"\u{e9}".repeat(16_001)));
        assert!(!valid_instructions("keep this\0"));
        assert_eq!(count_text(16_000), "16,000");
    }
    #[test]
    fn a_refresh_keeps_the_unsaved_draft_and_reset_replaces_it() {
        let mut editor = Editor::new(Agent::Notes);
        editor.text = TextEdit::new("Unsaved local text");
        editor.loaded(
            Agent::Notes,
            Some(instructions::Entry {
                text: "Synced text".into(),
                based_on: 1,
                edited_at: "2026-11-10T04:00:00Z".into(),
            }),
            false,
        );
        assert_eq!(editor.text.text, "Unsaved local text");
        assert_eq!(editor.saved, "Synced text");
        assert!(editor.dirty());
        editor.loaded(Agent::Notes, None, true);
        assert_eq!(editor.text.text, instructions::default_base(Agent::Notes));
        assert!(!editor.dirty());
        assert!(editor.edited_at.is_none());
    }
    #[test]
    fn successful_save_marks_only_the_saved_draft_clean() {
        let mut editor = Editor::new(Agent::Tasks);
        editor.text = TextEdit::new("First draft");
        let entry = instructions::Entry {
            text: "First draft".into(),
            based_on: 1,
            edited_at: "2026-11-10T04:00:00Z".into(),
        };
        editor.loaded(Agent::Tasks, Some(entry.clone()), false);
        assert!(!editor.dirty());
        editor.text = TextEdit::new("Newer draft typed while saving");
        editor.loaded(Agent::Tasks, Some(entry), false);
        assert_eq!(editor.text.text, "Newer draft typed while saving");
        assert!(editor.dirty());
    }
    #[test]
    fn instruction_worker_saves_and_resets_each_agent_without_a_helper() {
        let root = std::env::temp_dir().join(format!(
            "notes-ui-worker-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let ui_thread = thread::current().id();
        let worker_root = root.clone();
        thread::spawn(move || {
            assert_ne!(ui_thread, thread::current().id());
            for index in 0..2 {
                let result = run(
                    &worker_root,
                    Work::Save(index, format!("Custom agent {index}")),
                )
                .unwrap();
                assert!(
                    matches!(result, Completion::Instructions(i, Some(_), false) if i == index)
                );
            }
            assert!(instructions::load(&worker_root).unwrap().notes.is_some());
            assert!(instructions::load(&worker_root).unwrap().tasks.is_some());
            assert!(matches!(
                run(&worker_root, Work::Reset(0)).unwrap(),
                Completion::Instructions(0, None, true)
            ));
            assert!(instructions::load(&worker_root).unwrap().tasks.is_some());
            assert!(run(&worker_root, Work::Save(1, "x".repeat(16_001))).is_err());
        })
        .join()
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
