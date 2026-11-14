//! Meeting engine tab: shows the reviewed `engine_settings` model and its read-only status.
//!
//! Status lines say where a value came from and whether it is valid. They never print a value
//! the status did not come from, so nothing a script import rejected can appear here.
use super::{group_highlight, icon_tile, panel, row_icon};
use crate::ui::{State, roles, theme::Palette, widgets::hairline};
use rune::{El, col};
use sidenote::core::{
    Config,
    engine_settings::{FieldStatus, Source, Validation},
};
use std::collections::BTreeMap;

const NOT_USED: &str = "Not used by the built-in engine yet";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Field {
    Threads,
    Deadline,
    RequestDeadline,
}
impl Field {
    const TRANSCRIPTION: [Field; 3] = [Self::Threads, Self::Deadline, Self::RequestDeadline];
    /// The row key, also the scroll target for search.
    fn key(self) -> &'static str {
        match self {
            Self::Threads => "engine-worker-threads",
            Self::Deadline => "engine-transcription-deadline",
            Self::RequestDeadline => "engine-request-deadline",
        }
    }
    /// The name `Config::engine_field_status` uses.
    fn status_key(self) -> &'static str {
        match self {
            Self::Threads => "worker_threads",
            Self::Deadline => "stage_deadline_seconds",
            Self::RequestDeadline => "stages.asr_request_deadline_seconds",
        }
    }
    /// Label and help.
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Self::Threads => (
                "Worker threads",
                "How many transcription workers run at the same time. The default is 4.",
            ),
            Self::Deadline => (
                "Transcription deadline (seconds)",
                "The longest a whole transcription may take. Empty means it is worked out from the audio length.",
            ),
            Self::RequestDeadline => (
                "Request deadline (seconds)",
                "The longest one transcription request may take. The default is 180.",
            ),
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
            .map_or_else(|| "Automatic".into(), |v| v.to_string()),
        Field::RequestDeadline => settings.stages.asr_request_deadline_seconds.to_string(),
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

fn field_row(s: &State, p: Palette, field: Field, statuses: &BTreeMap<String, FieldStatus>) -> El {
    let (name, help) = field.text();
    let key = field.key();
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
    let value = roles::CONTROL.wrap(read(&s.config, field), p.ink);
    crate::ui::cards::row_shell()
        .key(key)
        .flex_wrap()
        .fill(group_highlight(s, key, p))
        .child(icon_tile(row_icon(key), p))
        .child(copy)
        .child(value.key(format!("{key}-value")).max_w(220.0))
}
fn group(s: &State, p: Palette, title: &str, key: &str, fields: &[Field]) -> El {
    let statuses = s.config.engine_field_status();
    let mut rows = col().w_full();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            rows = rows.child(hairline(p));
        }
        rows = rows.child(field_row(s, p, *field, &statuses));
    }
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .key(key)
        .child(roles::GROUP.wrap(title, p.soft))
        .child(panel(p).child(rows))
}

pub(super) fn body(s: &State, p: Palette) -> El {
    let intro = "Choices for how Sidenote turns a meeting into notes. They are saved on this PC. Each one says whether the built-in engine uses it yet.";
    col()
        .w_full()
        .key("meeting-engine")
        .child(roles::SMALL.wrap(intro, p.soft).mt(12.0))
        .child(group(
            s,
            p,
            "Transcription",
            "engine-transcription",
            &Field::TRANSCRIPTION,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_read_what_is_stored_and_a_blank_deadline_means_automatic() {
        let mut config = Config::default();
        assert_eq!(read(&config, Field::Threads), "4");
        assert_eq!(read(&config, Field::Deadline), "Automatic");
        assert_eq!(read(&config, Field::RequestDeadline), "180");
        config.engine_settings.stage_deadline_seconds = Some(900);
        config.engine_settings.worker_threads = 8;
        assert_eq!(read(&config, Field::Deadline), "900");
        assert_eq!(read(&config, Field::Threads), "8");
        config.engine_settings.stage_deadline_seconds = Some(0);
        assert_eq!(read(&config, Field::Deadline), "Automatic");
    }
    #[test]
    fn every_field_has_a_status_and_a_distinct_row_key() {
        let status = Config::default().engine_field_status();
        let keys: std::collections::BTreeSet<_> =
            Field::TRANSCRIPTION.iter().map(|f| f.key()).collect();
        assert_eq!(keys.len(), Field::TRANSCRIPTION.len());
        for field in Field::TRANSCRIPTION {
            assert!(status.contains_key(field.status_key()), "{field:?}");
        }
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
}
