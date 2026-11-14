//! Meeting engine tab: edits the reviewed `engine_settings` model and shows its read-only status.
//!
//! Drafts are plain text. Only a field the person changed is applied, so the reviewed save path
//! validates only that field and a stored value that is already wrong never blocks another edit.
//! Status lines say where a value came from and whether it is valid. They never print a value
//! the status did not come from, so nothing a script import rejected can appear here.
use super::{group_highlight, icon_tile, panel, row_icon};
use crate::ui::{
    State, roles,
    theme::Palette,
    widgets::{ButtonDisabled, action, hairline},
};
use rune::{El, TextEdit, col};
use rune_kit::{Kit, TextInput};
use sidenote::core::{
    Config,
    engine_settings::{FieldStatus, Source, Validation},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const NOT_USED: &str = "Not used by the built-in engine yet";
const NOT_SAVED: &str = "Nothing was saved. Fix the values marked not valid.";
const SAVE_FAILED: &str = "Could not save. Reload Settings and try again.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Field {
    Threads,
    Deadline,
    RequestDeadline,
}
impl Field {
    pub(super) const TRANSCRIPTION: [Field; 3] =
        [Self::Threads, Self::Deadline, Self::RequestDeadline];
    const COUNT: usize = 3;
    fn index(self) -> usize {
        self as usize
    }
    /// The row key, also the scroll target for search.
    fn key(self) -> &'static str {
        match self {
            Self::Threads => "engine-worker-threads",
            Self::Deadline => "engine-transcription-deadline",
            Self::RequestDeadline => "engine-request-deadline",
        }
    }
    /// The name `Config::engine_field_status` and `validate_engine_settings` both use.
    fn status_key(self) -> &'static str {
        match self {
            Self::Threads => "worker_threads",
            Self::Deadline => "stage_deadline_seconds",
            Self::RequestDeadline => "stages.asr_request_deadline_seconds",
        }
    }
    /// Label, help and the hint shown in an empty box.
    fn text(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Threads => (
                "Worker threads",
                "How many transcription workers run at the same time. The default is 4.",
                "4",
            ),
            Self::Deadline => (
                "Transcription deadline (seconds)",
                "The longest a whole transcription may take. Leave empty to work it out from the audio length.",
                "Automatic",
            ),
            Self::RequestDeadline => (
                "Request deadline (seconds)",
                "The longest one transcription request may take. The default is 180.",
                "180",
            ),
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Threads => "Not valid. Use a whole number from 1 to 256.",
            Self::Deadline | Self::RequestDeadline => {
                "Not valid. Use a whole number of seconds from 1 to 3600."
            }
        }
    }
}

/// Only the notes writer and model are read by the built-in engine today. Every other control
/// here is stored for the engine that will read it, and says so.
fn in_use(status_key: &str) -> bool {
    matches!(status_key, "notes.helper" | "notes.model")
}
fn read(config: &Config, field: Field) -> String {
    let settings = &config.engine_settings;
    match field {
        Field::Threads => settings.worker_threads.to_string(),
        Field::Deadline => settings
            .stage_deadline_seconds
            .filter(|v| *v > 0)
            .map_or(String::new(), |v| v.to_string()),
        Field::RequestDeadline => settings.stages.asr_request_deadline_seconds.to_string(),
    }
}
fn write(config: &mut Config, field: Field, text: &str) -> Result<(), ()> {
    let number = || text.parse::<u32>().map_err(|_| ());
    let settings = &mut config.engine_settings;
    match field {
        Field::Threads => settings.worker_threads = number()?,
        Field::Deadline => {
            settings.stage_deadline_seconds = if text.is_empty() {
                None
            } else {
                Some(number()?)
            }
        }
        Field::RequestDeadline => settings.stages.asr_request_deadline_seconds = number()?,
    }
    Ok(())
}
/// Would this one edit be valid? Other fields, even ones already wrong, do not count.
fn probe(base: &Config, field: Field, text: &str) -> bool {
    let mut config = base.clone();
    write(&mut config, field, text).is_ok()
        && !config
            .validate_engine_settings()
            .contains(&field.status_key())
}
/// The base with every changed, valid draft applied. `Ok(None)` means nothing changed.
fn apply(base: &Config, edits: &[(Field, &str)]) -> Result<Option<Config>, Vec<Field>> {
    let mut edited = base.clone();
    let (mut bad, mut changed) = (Vec::new(), false);
    for &(field, text) in edits {
        let text = text.trim();
        if text == read(base, field) {
            continue;
        }
        if probe(base, field, text) && write(&mut edited, field, text).is_ok() {
            changed = true;
        } else {
            bad.push(field);
        }
    }
    if bad.is_empty() {
        Ok(changed.then_some(edited))
    } else {
        Err(bad)
    }
}
/// Source and validity in plain words, and whether the line needs attention.
fn status_text(status: &FieldStatus) -> (String, bool) {
    let source = match status.source {
        Source::Default => "Default value",
        Source::Saved => "Saved on this PC",
        Source::LegacyEnvironment => "Imported from your scripts",
        Source::ExistingRoute => "Set in your language settings",
    };
    let validity = match status.validation {
        Validation::Empty => "not set",
        Validation::Valid => "valid",
        Validation::Invalid => "not valid",
        Validation::Unavailable => "not available on this PC",
        Validation::ModelQualificationRequired => "name not checked on this PC yet",
    };
    let warn = matches!(
        status.validation,
        Validation::Invalid | Validation::Unavailable
    );
    (format!("{source} \u{b7} {validity}"), warn)
}

#[derive(Default)]
pub(super) struct Engine {
    base: Option<Config>,
    drafts: Vec<TextEdit>,
    bad: Vec<Field>,
    error: Option<&'static str>,
}
impl Engine {
    fn dirty(&self, fields: &[Field]) -> bool {
        self.base.as_ref().is_some_and(|base| {
            fields.iter().any(|field| {
                let draft = self.drafts.get(field.index());
                draft.is_some_and(|d| d.text.trim() != read(base, *field))
            })
        })
    }
}

/// Start the tab from what is stored now.
pub(super) fn load(s: &mut State) {
    let config = s.config.clone();
    s.hex.shell.engine = Engine {
        drafts: Field::TRANSCRIPTION
            .iter()
            .map(|field| TextEdit::new(read(&config, *field)))
            .collect(),
        base: Some(config),
        ..Default::default()
    };
}
fn draft_mut(s: &mut State, index: usize) -> &mut TextEdit {
    if s.hex.shell.engine.drafts.len() < Field::COUNT {
        load(s);
    }
    &mut s.hex.shell.engine.drafts[index]
}
/// After a failed save: fields the person changed keep their text, the rest take what is stored.
fn rebase(engine: &mut Engine, fresh: Config) {
    if let Some(old) = &engine.base {
        for field in Field::TRANSCRIPTION {
            if let Some(draft) = engine.drafts.get_mut(field.index())
                && draft.text == read(old, field)
            {
                *draft = TextEdit::new(read(&fresh, field));
            }
        }
    }
    engine.base = Some(fresh);
}
fn save(s: &mut State, fields: &[Field]) {
    let Some(base) = s.hex.shell.engine.base.clone() else {
        return;
    };
    let drafts: Vec<_> = fields
        .iter()
        .filter_map(|field| {
            let draft = s.hex.shell.engine.drafts.get(field.index())?;
            Some((*field, draft.text.clone()))
        })
        .collect();
    let edits: Vec<_> = drafts.iter().map(|(f, t)| (*f, t.as_str())).collect();
    let outcome = apply(&base, &edits);
    s.hex.shell.engine.bad.clear();
    s.hex.shell.engine.error = None;
    match outcome {
        Err(bad) => {
            s.hex.shell.engine.bad = bad;
            s.hex.shell.engine.error = Some(NOT_SAVED);
        }
        Ok(None) => {}
        Ok(Some(mut config)) => match config.save(&s.store.root) {
            Ok(()) => {
                s.config = config;
                load(s);
                s.hex.shell.saved = Some(Instant::now() + Duration::from_secs(2));
            }
            Err(_) => {
                s.hex.shell.engine.error = Some(SAVE_FAILED);
                if let Ok(fresh) = Config::load_in(&s.store.root) {
                    rebase(&mut s.hex.shell.engine, fresh);
                }
            }
        },
    }
}

fn field_row(
    s: &State,
    k: &Kit,
    p: Palette,
    field: Field,
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let (name, help, placeholder) = field.text();
    let key = field.key();
    let engine = &s.hex.shell.engine;
    let mut copy = col()
        .grow(1.0)
        .min_w(170.0)
        .gap(4.0)
        .child(roles::NAME.wrap(name, p.ink))
        .child(roles::SMALL.wrap(help, p.soft));
    if let Some(status) = statuses.get(field.status_key()) {
        let (line, warn) = status_text(status);
        let color = if warn { p.warn } else { p.soft };
        copy = copy.child(roles::META.wrap(line, color).key(format!("{key}-status")));
    }
    if !in_use(field.status_key()) {
        let usage = roles::META.medium().wrap(NOT_USED, p.soft);
        copy = copy.child(usage.key(format!("{key}-usage")));
    }
    if engine.bad.contains(&field) {
        let hint = roles::META.medium().wrap(field.hint(), p.warn);
        copy = copy.child(hint.role("alert"));
    }
    let index = field.index();
    let fallback = TextEdit::default();
    let input = TextInput::new(engine.drafts.get(index).unwrap_or(&fallback))
        .label(name)
        .placeholder(placeholder)
        .on_edit(move |s: &mut State| draft_mut(s, index))
        .build(k)
        .key(format!("{key}-input"))
        .w(220.0);
    crate::ui::cards::row_shell()
        .key(key)
        .flex_wrap()
        .fill(group_highlight(s, key, p))
        .child(icon_tile(row_icon(key), p))
        .child(copy)
        .child(input)
}
fn group(
    s: &State,
    k: &Kit,
    p: Palette,
    title: &str,
    key: &str,
    fields: &'static [Field],
    statuses: &BTreeMap<String, FieldStatus>,
) -> El {
    let mut rows = col().w_full();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            rows = rows.child(hairline(p));
        }
        rows = rows.child(field_row(s, k, p, *field, statuses));
    }
    let engine = &s.hex.shell.engine;
    let mut footer = col().w_full().gap(8.0).padding(14.0);
    if let Some(error) = engine.error {
        footer = footer.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    footer = footer.child(
        action(format!("Save {}", title.to_lowercase()), p)
            .key(format!("{key}-save"))
            .self_start()
            .button_disabled(!engine.dirty(fields))
            .on_press(move |s: &mut State| save(s, fields)),
    );
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .key(key)
        .child(roles::GROUP.wrap(title, p.soft))
        .child(panel(p).child(rows.child(hairline(p)).child(footer)))
}

pub(super) fn body(s: &State, k: &Kit, p: Palette) -> El {
    let statuses = s.config.engine_field_status();
    let intro = "Choices for how Sidenote turns a meeting into notes. They are saved on this PC. Each one says whether the built-in engine uses it yet.";
    col()
        .w_full()
        .key("meeting-engine")
        .child(roles::SMALL.wrap(intro, p.soft).mt(12.0))
        .child(group(
            s,
            k,
            p,
            "Transcription",
            "engine-transcription",
            &Field::TRANSCRIPTION,
            &statuses,
        ))
}

/// Type values that cannot be saved and press Save, as the person would. Render scenes only.
pub(super) fn scene_invalid(s: &mut State) {
    for (field, text) in [(Field::Threads, "0"), (Field::Deadline, "9999")] {
        *draft_mut(s, field.index()) = TextEdit::new(text);
    }
    save(s, &Field::TRANSCRIPTION);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_write_round_trip_and_blank_deadline_means_automatic() {
        let mut config = Config::default();
        assert_eq!(read(&config, Field::Threads), "4");
        assert_eq!(read(&config, Field::Deadline), "");
        assert_eq!(read(&config, Field::RequestDeadline), "180");
        assert!(write(&mut config, Field::Deadline, "900").is_ok());
        assert_eq!(config.engine_settings.stage_deadline_seconds, Some(900));
        assert!(write(&mut config, Field::Deadline, "").is_ok());
        assert_eq!(config.engine_settings.stage_deadline_seconds, None);
        assert!(write(&mut config, Field::Threads, "four").is_err());
    }
    #[test]
    fn only_changed_fields_are_applied_and_only_they_are_checked() {
        let mut base = Config::default();
        // A stored value that is already wrong must not block an unrelated edit.
        base.engine_settings.stages.asr_request_deadline_seconds = 0;
        assert!(apply(&base, &[(Field::Threads, "4")]).unwrap().is_none());
        let edited = apply(&base, &[(Field::Threads, " 8 "), (Field::Deadline, "60")])
            .unwrap()
            .unwrap();
        assert_eq!(edited.engine_settings.worker_threads, 8);
        assert_eq!(edited.engine_settings.stage_deadline_seconds, Some(60));
        assert_eq!(read(&edited, Field::RequestDeadline), "0");
        let edits = [
            (Field::Threads, "0"),
            (Field::Deadline, "3601"),
            (Field::RequestDeadline, "90"),
        ];
        let bad = apply(&base, &edits).err().unwrap();
        assert_eq!(bad, [Field::Threads, Field::Deadline]);
    }
    #[test]
    fn status_words_cover_every_state_without_a_value() {
        let sources = [
            Source::Default,
            Source::Saved,
            Source::LegacyEnvironment,
            Source::ExistingRoute,
        ];
        let validations = [
            Validation::Empty,
            Validation::Valid,
            Validation::Invalid,
            Validation::Unavailable,
            Validation::ModelQualificationRequired,
        ];
        for source in sources {
            for validation in validations {
                let status = FieldStatus {
                    source,
                    validation,
                    conflict: false,
                    invalid_import: false,
                };
                let (line, warn) = status_text(&status);
                assert!(line.contains(" \u{b7} "));
                let attention = matches!(validation, Validation::Invalid | Validation::Unavailable);
                assert_eq!(warn, attention);
            }
        }
        assert!(in_use("notes.model") && !in_use("worker_threads"));
    }
    #[test]
    fn a_failed_save_keeps_edited_drafts_and_refreshes_the_rest() {
        let (mut old, mut fresh) = (Config::default(), Config::default());
        old.engine_settings.worker_threads = 5;
        fresh.engine_settings.worker_threads = 6;
        fresh.engine_settings.stages.asr_request_deadline_seconds = 200;
        let drafts = Field::TRANSCRIPTION.map(|f| TextEdit::new(read(&old, f)));
        let mut engine = Engine {
            drafts: drafts.to_vec(),
            base: Some(old),
            ..Default::default()
        };
        engine.drafts[Field::Deadline.index()] = TextEdit::new("120");
        engine.drafts[Field::Threads.index()] = TextEdit::new("7");
        rebase(&mut engine, fresh);
        assert_eq!(engine.drafts[Field::Deadline.index()].text, "120");
        assert_eq!(engine.drafts[Field::Threads.index()].text, "7");
        assert_eq!(engine.drafts[Field::RequestDeadline.index()].text, "200");
    }
    #[test]
    fn the_reviewed_save_path_keeps_the_edit_and_rejects_a_bad_one_untouched() {
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        std::fs::write(&path, r#"{"version":1,"features":{},"notes":{}}"#).unwrap();
        let base = Config::load_in(root.path()).unwrap();
        let mut edited = apply(&base, &[(Field::Threads, "8")]).unwrap().unwrap();
        edited.save(root.path()).unwrap();
        let loaded = Config::load_in(root.path()).unwrap();
        assert_eq!(read(&loaded, Field::Threads), "8");
        let status = loaded.engine_field_status();
        assert_eq!(status["worker_threads"].source, Source::Saved);
        let prior = serde_json::to_vec(&loaded).unwrap();
        assert!(apply(&loaded, &[(Field::Threads, "0")]).is_err());
        let after = Config::load_in(root.path()).unwrap();
        assert_eq!(serde_json::to_vec(&after).unwrap(), prior);
    }
}
