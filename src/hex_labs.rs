//! Compatibility assessments from the Hex Labs report, not runtime safety guarantees.
use crate::hex_engine::Setting;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    None,
    Changes,
    Opportunity,
    Breaks,
}
pub struct Lab {
    pub id: &'static str,
    pub name: &'static str,
    pub class: Class,
    pub effect: &'static str,
    pub action: &'static str,
}
pub const SOURCE_REVISION: &str = "7f54733e22b13e38f8bd220e402a2b64651ae94b";
pub const UNKNOWN_EFFECT: &str = "New in Hex, not reviewed by Sidenote yet";
pub const REGISTRY: &[Lab] = &[
    Lab {
        id: "playground",
        name: "Playground",
        class: Class::None,
        effect: "Scene and story browser in the window.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "live-text",
        name: "Live Text",
        class: Class::None,
        effect: "Show provisional words beside the HUD while dictating.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "failure-notices",
        name: "Failure Notices",
        class: Class::None,
        effect: "Show why a dictation failed beside the HUD.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "vocabulary-biasing",
        name: "Vocabulary Biasing",
        class: Class::Changes,
        effect: "Parakeet vocabulary bias may change recognition.",
        action: "Verify requested models and preserve source text.",
    },
    Lab {
        id: "terminal-paste",
        name: "Terminal Paste",
        class: Class::None,
        effect: "Paste with Ctrl+Shift+V, for apps that ignore Ctrl+V.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "keyboard-model-switching",
        name: "Keyboard Model Switching",
        class: Class::Changes,
        effect: "Keyboard language may change the selected speech model.",
        action: "Test selection races and preserve the router guard.",
    },
    Lab {
        id: "app-model-switching",
        name: "App Model Switching",
        class: Class::Changes,
        effect: "The foreground app may change the selected speech model.",
        action: "Test selection races and avoid false router aborts.",
    },
    Lab {
        id: "share-shortcut",
        name: "Share Shortcut",
        class: Class::None,
        effect: "Let apps such as Discord see the dictation shortcut too.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "unload-idle",
        name: "Unload Idle",
        class: Class::Changes,
        effect: "Idle unloading can add model reload time.",
        action: "Measure and tolerate cold starts.",
    },
    Lab {
        id: "restore-shortcut-default",
        name: "Restore Shortcut Default",
        class: Class::None,
        effect: "Offer Restore default next to Paste last dictation's shortcut.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "keep-on-clipboard",
        name: "Keep On Clipboard",
        class: Class::None,
        effect: "Leave the dictation on the clipboard after pasting instead of restoring what was there.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "copy-only",
        name: "Copy Only",
        class: Class::None,
        effect: "Put the dictation on the clipboard instead of pasting it.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "spoken-formatting",
        name: "Spoken Formatting",
        class: Class::None,
        effect: "Dictation output transform; API Global processing is opt-in.",
        action: "Keep raw API output by default.",
    },
    Lab {
        id: "filler-words",
        name: "Filler Words",
        class: Class::None,
        effect: "Dictation output transform; API Global processing is opt-in.",
        action: "Keep raw API output by default.",
    },
    Lab {
        id: "second-dictation-shortcut",
        name: "Second Dictation Shortcut",
        class: Class::None,
        effect: "Another shortcut that dictates like the main one (second_dictation_hotkey), such as a key on an external keyboard.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "terminal-paste-shortcut",
        name: "Terminal Paste Shortcut",
        class: Class::None,
        effect: "Another dictation shortcut that always pastes with Ctrl+Shift+V (terminal_paste_hotkey).",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "spoken-symbols",
        name: "Spoken Symbols",
        class: Class::None,
        effect: "Dictation output transform; API Global processing is opt-in.",
        action: "Keep raw API output by default.",
    },
    Lab {
        id: "picker-sort-filter",
        name: "Picker Sort Filter",
        class: Class::None,
        effect: "Sort the Local transcription picker by size or name and show only models for the saved language.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "benchmark-details",
        name: "Benchmark Details",
        class: Class::None,
        effect: "Show each model's raw error rate, benchmark, and source in the Local transcription picker.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "parakeet-ultra",
        name: "Parakeet Ultra",
        class: Class::None,
        effect: "Adds picker choice; router has selection invariant. Impact of selecting it covered by model switching.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "special-keys",
        name: "Special Keys",
        class: Class::None,
        effect: "Use Insert, Home, End, Page Up, or Page Down alone as a shortcut.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "history-stats",
        name: "History Stats",
        class: Class::None,
        effect: "Add a Stats button to History that shows a summary of your retained dictations, with hex history stats.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "skip-pauses",
        name: "Skip Pauses",
        class: Class::None,
        effect: "Described as whole-dictation preprocessing; applicability to API not verified.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "output-fixes",
        name: "Output Fixes",
        class: Class::None,
        effect: "Hex Output UI; vocabulary edits can indirectly change future recognition.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "picker-active-first",
        name: "Picker Active First",
        class: Class::None,
        effect: "The speech model picker lists the model you're using first.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "model-in-place",
        name: "Model In Place",
        class: Class::None,
        effect: "Adds model storage option; direct API or routing impact not verified.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "picker-downloaded",
        name: "Picker Downloaded",
        class: Class::None,
        effect: "Show only downloaded speech models in the picker, with the space they take.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "keep-recordings",
        name: "Keep Recordings",
        class: Class::None,
        effect: "Retains dictation history audio rather than meeting tracks.",
        action: "Do not rely on it for meeting audio.",
    },
    Lab {
        id: "settings-search",
        name: "Settings Search",
        class: Class::None,
        effect: "Find any setting in General with a search field.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "appearance",
        name: "Appearance",
        class: Class::None,
        effect: "Use light or dark for the main window instead of the system's.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "release-notes-link",
        name: "Release Notes Link",
        class: Class::None,
        effect: "Show a link to this build's release notes in the sidebar footer.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "meeting-notes-vocabulary",
        name: "Meeting Notes Vocabulary",
        class: Class::Changes,
        effect: "Hex titles and summaries may use your vocabulary.",
        action: "Treat them as upstream generated text.",
    },
    Lab {
        id: "meeting-finished-event",
        name: "Meeting Finished Event",
        class: Class::Opportunity,
        effect: "Notifies Sidenote when a meeting transcript is ready.",
        action: "Keep catch-up and reconnect handling.",
    },
    Lab {
        id: "follow-live-transcript",
        name: "Follow Live Transcript",
        class: Class::None,
        effect: "Hex window scrolling only; Sidenote reads rolling transcript independently.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "more-call-apps",
        name: "More Call Apps",
        class: Class::Changes,
        effect: "Additional call apps can create meetings with new source labels.",
        action: "Accept unfamiliar source labels.",
    },
    Lab {
        id: "meeting-descriptive-filenames",
        name: "Meeting Descriptive Filenames",
        class: Class::Opportunity,
        effect: "Exported Markdown may be renamed to the meeting title.",
        action: "Use manifest identity rather than export filenames.",
    },
    Lab {
        id: "meeting-notes-file",
        name: "Meeting Notes File",
        class: Class::Opportunity,
        effect: "Hex exports meeting notes beside the transcript.",
        action: "Support this only when importing Hex notes.",
    },
    Lab {
        id: "expire-meeting-audio",
        name: "Expire Meeting Audio",
        class: Class::Changes,
        effect: "Meeting WAV tracks can expire before a retry.",
        action: "Show missing audio and preserve transcript-only access.",
    },
    Lab {
        id: "opencode-in-wsl",
        name: "Opencode In Wsl",
        class: Class::None,
        effect: "Hex rewrite provider selection; API Global processing is not requested.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "transcribe-files",
        name: "Transcribe Files",
        class: Class::Opportunity,
        effect: "Hex can transcribe a WAV with an explicit model and language.",
        action: "Evaluate as an alternative without replacing the current path.",
    },
    Lab {
        id: "api-processing",
        name: "Api Processing",
        class: Class::Opportunity,
        effect: "API callers can request corrected and rewritten text.",
        action: "Keep raw transcripts by default.",
    },
    Lab {
        id: "api-timestamps",
        name: "Api Timestamps",
        class: Class::Opportunity,
        effect: "API callers can request segment or word timings.",
        action: "Probe model support before opting in.",
    },
    Lab {
        id: "speakers-only",
        name: "Speakers Only",
        class: Class::None,
        effect: "Apply \"While dictating\" only when sound plays through speakers; headphones are left alone.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "live-chapters",
        name: "Live chapters",
        class: Class::Opportunity,
        effect: "Hex can add and update chapters in the meeting manifest.",
        action: "Consider showing chapters after checking platform support.",
    },
    Lab {
        id: "subtitle-export",
        name: "Subtitle export",
        class: Class::Opportunity,
        effect: "Hex can export meeting transcripts as SRT or VTT.",
        action: "Consider user-initiated export after checking platform support.",
    },
];
pub fn find(id: &str) -> Option<&'static Lab> {
    REGISTRY.iter().find(|lab| lab.id == id)
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn setting_id(key: &str) -> Option<String> {
    let id = match key {
        "paste_with_shift" => "terminal-paste".into(),
        "share_dictation_shortcut" => "share-shortcut".into(),
        "keyboard_model_switching.enabled" => "keyboard-model-switching".into(),
        "meetings.enabled" => "meetings".into(),
        "capture.enabled" => "annotate".into(),
        "labs.standalone_special_keys" => "special-keys".into(),
        _ => key.strip_prefix("labs.")?.replace('_', "-"),
    };
    valid_id(&id).then_some(id)
}
/// Missing entries and invalid values are unknown, including in an enabled-only status list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub states: BTreeMap<String, Option<bool>>,
    pub source: &'static str,
}
impl Snapshot {
    pub fn can_enable(&self, id: &str) -> bool {
        self.state(id) == Some(false) && find(id).is_some_and(|lab| lab.class == Class::Opportunity)
    }
    pub fn state(&self, id: &str) -> Option<bool> {
        self.states.get(id).copied().flatten()
    }
    pub fn settings(settings: &[Setting]) -> Self {
        let mut out = Self {
            source: "Hex settings",
            ..Self::default()
        };
        for setting in settings {
            if setting.kind == "bool"
                && !setting.secret
                && setting.available != Some(false)
                && let Some(id) = setting_id(&setting.key)
            {
                let state = setting.value.as_bool();
                out.states
                    .entry(id)
                    .and_modify(|old| {
                        if *old != state {
                            *old = None;
                        }
                    })
                    .or_insert(state);
            }
        }
        out
    }
    pub fn status(value: &Value) -> Self {
        let mut out = Self {
            source: "Hex status (enabled only)",
            ..Self::default()
        };
        if let Some(labs) = value.get("labs").and_then(Value::as_array) {
            for id in labs
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| valid_id(id))
            {
                out.states.insert(id.into(), Some(true));
            }
        }
        out
    }
    pub fn list(text: &str) -> Self {
        let mut out = Self {
            source: "Hex CLI fallback",
            ..Self::default()
        };
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let Some(id) = words.next().filter(|id| valid_id(id)) else {
                continue;
            };
            let state = match words.next() {
                Some("on") => Some(true),
                Some("off") => Some(false),
                _ => None,
            };
            // A settings-key marker distinguishes a lab row from help, headers, and diagnostics.
            if line.contains("(hex settings ") {
                out.states
                    .entry(id.into())
                    .and_modify(|old| {
                        if *old != state {
                            *old = None;
                        }
                    })
                    .or_insert(state);
            }
        }
        out
    }
    pub fn relevant(&self) -> Vec<(String, Option<&'static Lab>, Option<bool>)> {
        let mut ids: std::collections::BTreeSet<_> = REGISTRY
            .iter()
            .filter(|lab| lab.class != Class::None)
            .map(|lab| lab.id.to_owned())
            .collect();
        ids.extend(self.states.keys().filter(|id| find(id).is_none()).cloned());
        ids.into_iter()
            .map(|id| {
                let lab = find(&id);
                let state = self.state(&id);
                (id, lab, state)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_matches_review_and_ids_are_unique() {
        assert_eq!(REGISTRY.len(), 45);
        let ids: std::collections::BTreeSet<_> = REGISTRY.iter().map(|lab| lab.id).collect();
        assert_eq!(ids.len(), REGISTRY.len());
        assert!(REGISTRY.iter().all(|lab| valid_id(lab.id)
            && !lab.name.is_empty()
            && !lab.effect.is_empty()
            && !lab.action.is_empty()));
        assert!(REGISTRY.iter().all(|lab| lab.class != Class::Breaks));
    }
    #[test]
    fn missing_or_invalid_status_never_means_off() {
        for raw in [
            include_str!("../tests/fixtures/hex-labs/status-missing.json"),
            include_str!("../tests/fixtures/hex-labs/status-invalid.json"),
        ] {
            let state = Snapshot::status(&serde_json::from_str(raw).unwrap());
            assert_eq!(state.state("api-processing"), None);
        }
        let state = Snapshot::status(
            &serde_json::from_str(include_str!("../tests/fixtures/hex-labs/status-new.json"))
                .unwrap(),
        );
        assert_eq!(state.state("api-processing"), Some(true));
        assert_eq!(state.state("api-timestamps"), None);
        assert!(
            state
                .relevant()
                .iter()
                .any(|(id, lab, state)| id == "future-synthetic-lab"
                    && lab.is_none()
                    && *state == Some(true))
        );
    }
    #[test]
    fn settings_preserve_chapters_unknowns_invalid_values_and_aliases() {
        let value: Value =
            serde_json::from_str(include_str!("../tests/fixtures/hex-labs/settings.json")).unwrap();
        let settings: Vec<Setting> = serde_json::from_value(value["settings"].clone()).unwrap();
        let state = Snapshot::settings(&settings);
        assert_eq!(state.state("live-chapters"), Some(true));
        assert_eq!(state.state("subtitle-export"), Some(false));
        assert!(state.can_enable("subtitle-export"));
        for id in [
            "live-chapters",
            "api-timestamps",
            "future-synthetic-lab",
            "special-keys",
        ] {
            assert!(!state.can_enable(id));
        }
        assert_eq!(state.state("api-timestamps"), None);
        assert_eq!(state.state("keyboard-model-switching"), Some(true));
        assert_eq!(state.state("special-keys"), Some(false));
        assert!(!state.states.contains_key("unload-idle-minutes"));
        assert!(!state.states.contains_key("not-on-this-platform"));
        assert!(
            state
                .relevant()
                .iter()
                .any(|(id, lab, state)| id == "future-synthetic-lab"
                    && lab.is_none()
                    && *state == Some(false))
        );
        assert!(
            state
                .relevant()
                .iter()
                .all(|(_, lab, _)| lab.is_none_or(|lab| lab.class != Class::None))
        );
    }
    #[test]
    fn text_fallback_accepts_whitespace_but_not_new_state_spellings() {
        let state = Snapshot::list(include_str!("../tests/fixtures/hex-labs/labs-list.txt"));
        assert_eq!(state.state("api-processing"), Some(true));
        assert_eq!(state.state("subtitle-export"), Some(false));
        assert_eq!(state.state("future-synthetic-lab"), None);
        assert_eq!(state.state("api-timestamps"), None);
        assert_eq!(Snapshot::list("unrecognizable format").states.len(), 0);
        let duplicate = Snapshot::list(
            "api-processing on (hex settings labs.api_processing)\napi-processing off (hex settings labs.api_processing)",
        );
        assert_eq!(duplicate.state("api-processing"), None);
    }
}
