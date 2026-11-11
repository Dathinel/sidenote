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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Affects,
    CanUse,
    New,
}
impl Group {
    pub const ALL: [Self; 3] = [Self::Affects, Self::CanUse, Self::New];
    pub fn name(self) -> &'static str {
        match self {
            Self::Affects => "Affects Sidenote",
            Self::CanUse => "Sidenote can use",
            Self::New => "New in Hex",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Affects => "affects",
            Self::CanUse => "can-use",
            Self::New => "new",
        }
    }
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
        id: "meetings",
        name: "Meetings (beta)",
        class: Class::Changes,
        effect: "Sidenote needs Hex to record meetings and save their transcripts.",
        action: "Required for Sidenote. Keep meeting recording on in Hex.",
    },
    Lab {
        id: "voice-filter",
        name: "Voice Filter",
        class: Class::Changes,
        effect: "Voice effects can change the microphone audio Hex records for Sidenote.",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "playground",
        name: "Playground",
        class: Class::None,
        effect: "Scene and story browser in the window.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "live-text",
        name: "Live text",
        class: Class::None,
        effect: "Show provisional words beside the HUD while dictating.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "failure-notices",
        name: "Failure notices",
        class: Class::None,
        effect: "Show why a dictation failed beside the HUD.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "live-typing",
        name: "Live typing",
        class: Class::None,
        effect: "Show provisional words in the focused text field while dictating on Wayland, then type the final text there",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "live-chapters",
        name: "Live chapters",
        class: Class::Opportunity,
        effect: "Hex can divide a meeting transcript into chapters while recording.",
        action: "Consider showing chapters after checking platform support.",
    },
    Lab {
        id: "vocabulary-biasing",
        name: "Vocabulary biasing",
        class: Class::Changes,
        effect: "Hex may use your saved names and terms when transcribing meeting audio.",
        action: "Verify requested models and preserve source text.",
    },
    Lab {
        id: "terminal-paste",
        name: "Terminal paste",
        class: Class::None,
        effect: "Paste with Ctrl+Shift+V, for apps that ignore Ctrl+V.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "keyboard-model-switching",
        name: "Switch model with keyboard language",
        class: Class::Changes,
        effect: "Changing keyboard language may switch Hex's model and interrupt Sidenote's language pass.",
        action: "Test selection races and preserve the router guard.",
    },
    Lab {
        id: "app-model-switching",
        name: "Switch model with app",
        class: Class::Changes,
        effect: "Changing apps may change Hex's model and interrupt Sidenote's language pass.",
        action: "Test selection races and avoid false router aborts.",
    },
    Lab {
        id: "mouse-shortcuts",
        name: "Mouse shortcuts",
        class: Class::None,
        effect: "Record mouse buttons as shortcuts",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "pause-browser",
        name: "Also pause the browser",
        class: Class::None,
        effect: "While dictating with Pause media: YouTube and other tabs, resumed only if Hex paused them",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "share-shortcut",
        name: "Share dictation shortcut with other apps",
        class: Class::None,
        effect: "Let apps such as Discord see the dictation shortcut too.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "unload-idle",
        name: "Unload model when idle",
        class: Class::Changes,
        effect: "The first pass after a quiet period waits while Hex reloads the model.",
        action: "Measure and tolerate cold starts.",
    },
    Lab {
        id: "restore-shortcut-default",
        name: "Restore shortcut default",
        class: Class::None,
        effect: "Offer Restore default next to Paste last dictation's shortcut.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "keep-on-clipboard",
        name: "Keep dictation on clipboard",
        class: Class::None,
        effect: "Leave the dictation on the clipboard after pasting instead of restoring what was there.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "copy-only",
        name: "Copy dictation only",
        class: Class::None,
        effect: "Put the dictation on the clipboard instead of pasting it.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "return-to-app",
        name: "Return to app",
        class: Class::None,
        effect: "If another app comes to the front while you dictate, Hex switches back to the app you started in before pasting. If it can't, nothing is pasted and Paste last dictation has the text.",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "spoken-formatting",
        name: "Spoken formatting",
        class: Class::None,
        effect: "Dictation output transform; API Global processing is opt-in.",
        action: "Keep raw API output by default.",
    },
    Lab {
        id: "filler-words",
        name: "Remove filler words",
        class: Class::None,
        effect: "Dictation output transform; API Global processing is opt-in.",
        action: "Keep raw API output by default.",
    },
    Lab {
        id: "subtitle-export",
        name: "Subtitle export",
        class: Class::Opportunity,
        effect: "Hex can export a meeting transcript as a subtitle file.",
        action: "Consider user-initiated export after checking platform support.",
    },
    Lab {
        id: "second-dictation-shortcut",
        name: "Second dictation shortcut",
        class: Class::None,
        effect: "Another shortcut that dictates like the main one (second_dictation_hotkey), such as a key on an external keyboard.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "terminal-paste-shortcut",
        name: "Terminal paste shortcut",
        class: Class::None,
        effect: "Another dictation shortcut that always pastes with Ctrl+Shift+V (terminal_paste_hotkey).",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "smart-spacing",
        name: "Smart spacing",
        class: Class::None,
        effect: "Add a space before a dictation pasted right after a word",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "spoken-symbols",
        name: "Spoken symbols",
        class: Class::None,
        effect: "Dictation output transform; API Global processing is opt-in.",
        action: "Keep raw API output by default.",
    },
    Lab {
        id: "picker-sort-filter",
        name: "Sort and filter models",
        class: Class::None,
        effect: "Sort the Local transcription picker by size or name and show only models for the saved language.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "sided-modifier-chords",
        name: "Sided modifier chords",
        class: Class::None,
        effect: "Record which side each modifier of a shortcut is on, such as Right ⌘ + Right ⌥",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "tap-or-hold",
        name: "Tap or hold",
        class: Class::None,
        effect: "In Toggle mode, hold the shortcut to dictate until you let go; a short tap still starts and stops",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "double-tap-modifiers",
        name: "Double-tap on a lone modifier",
        class: Class::None,
        effect: "Let Double-tap shortcut mode work when the shortcut has no key, such as Option, Globe, or ⌃⌥",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "benchmark-details",
        name: "Benchmark details",
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
        name: "Special keys alone",
        class: Class::None,
        effect: "Use Insert, Home, End, Page Up, or Page Down alone as a shortcut.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "history-stats",
        name: "Dictation stats",
        class: Class::None,
        effect: "Add a Stats button to History that shows a summary of your retained dictations, with hex history stats.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "history-multi-select",
        name: "Select multiple dictations",
        class: Class::None,
        effect: "Shift-click, Cmd-click (Ctrl on Windows and Linux) or drag to select several dictations, then copy or delete them together",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "recent-dictations",
        name: "Copy recent dictations",
        class: Class::None,
        effect: "Copy one of your last five dictations from the menu bar, under Paste Last Dictation",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "skip-pauses",
        name: "Skip long pauses",
        class: Class::None,
        effect: "Described as whole-dictation preprocessing; applicability to API not verified.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "stop-when-call-ends",
        name: "Stop when the call ends",
        class: Class::Changes,
        effect: "Hex may stop recording automatically when a call ends.",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "output-fixes",
        name: "Fix results in Output",
        class: Class::None,
        effect: "Hex Output UI; vocabulary edits can indirectly change future recognition.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "picker-active-first",
        name: "Model in use first",
        class: Class::None,
        effect: "The speech model picker lists the model you're using first.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "model-in-place",
        name: "Use model files where they are",
        class: Class::None,
        effect: "Adds model storage option; direct API or routing impact not verified.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "picker-downloaded",
        name: "Downloaded models only",
        class: Class::None,
        effect: "Show only downloaded speech models in the picker, with the space they take.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "section-key",
        name: "Section key alone",
        class: Class::None,
        effect: "Use the § key, left of 1 on ISO keyboards, as a shortcut without a modifier. Hex takes § from every app while it's bound",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "keep-recordings",
        name: "Keep recordings",
        class: Class::None,
        effect: "Retains dictation history audio rather than meeting tracks.",
        action: "Do not rely on it for meeting audio.",
    },
    Lab {
        id: "settings-search",
        name: "Settings search",
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
        name: "Release notes link",
        class: Class::None,
        effect: "Show a link to this build's release notes in the sidebar footer.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "hide-menu-bar-icon",
        name: "Hide menu bar icon",
        class: Class::None,
        effect: "Hide Hex's icon from the menu bar. Shortcuts keep working; open Hex again from Finder or Spotlight for Settings",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "movable-meeting-capsules",
        name: "Movable meeting capsules",
        class: Class::None,
        effect: "Drag meeting capsules out of the way; they come back in the corner you left them",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "gnome-clipboard-restore",
        name: "Restore clipboard on GNOME",
        class: Class::None,
        effect: "Put back what you copied before a dictation, through XWayland. Works only while XWayland is already running",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "typed-text",
        name: "Transform typed text",
        class: Class::None,
        effect: "Adds a shortcut that runs your Output steps on text you typed and replaces it.",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "meeting-notes-vocabulary",
        name: "Vocabulary in meeting notes",
        class: Class::Changes,
        effect: "Hex may use your saved names and terms in meeting titles and summaries.",
        action: "Treat them as upstream generated text.",
    },
    Lab {
        id: "notch-indicator",
        name: "Notch indicator",
        class: Class::None,
        effect: "On a screen with a notch, the recording indicator grows out of the notch instead of sitting below the menu bar",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "meeting-finished-event",
        name: "Meeting finished event",
        class: Class::Opportunity,
        effect: "Hex tells Sidenote when a meeting transcript is ready.",
        action: "Keep catch-up and reconnect handling.",
    },
    Lab {
        id: "gnome-meeting-capsules",
        name: "Floating meeting capsules on GNOME",
        class: Class::None,
        effect: "Show the recording offer and recording indicator floating over your call, through XWayland. Works only while XWayland is already running",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "gnome-annotate",
        name: "Annotate on GNOME",
        class: Class::None,
        effect: "Show Annotate's recording capsule and ink over your screen, through XWayland. Works only while XWayland is already running; Esc can't stop drawing there, so use the capsule's pen button or let go of a held Annotate key",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "screenshots",
        name: "Screenshots",
        class: Class::None,
        effect: "Press ⌘⇧4 to capture an area, or press Space for a window. The shot goes on the clipboard and into a folder",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "follow-live-transcript",
        name: "Follow live transcript",
        class: Class::None,
        effect: "Hex window scrolling only; Sidenote reads rolling transcript independently.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "more-call-apps",
        name: "More call apps",
        class: Class::Changes,
        effect: "Hex can record more call apps, so more meetings can appear in Sidenote.",
        action: "Accept unfamiliar source labels.",
    },
    Lab {
        id: "annotate-replay",
        name: "Annotate 15 s replay",
        class: Class::Opportunity,
        effect: "Sidenote can make notes from a saved replay of your last 15 seconds on screen.",
        action: "Use only the captures and frames the user selects for notes.",
    },
    Lab {
        id: "meeting-import",
        name: "Import audio",
        class: Class::Changes,
        effect: "Audio files imported into Hex can appear as meetings in Sidenote.",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "meeting-descriptive-filenames",
        name: "Descriptive meeting file names",
        class: Class::Opportunity,
        effect: "Hex can name exported meeting files after the meeting title.",
        action: "Use manifest identity rather than export filenames.",
    },
    Lab {
        id: "meeting-notes-file",
        name: "Separate notes file",
        class: Class::Opportunity,
        effect: "Hex can save meeting notes in a separate file beside the transcript.",
        action: "Support this only when importing Hex notes.",
    },
    Lab {
        id: "quit-closes-window",
        name: "⌘Q closes Settings",
        class: Class::None,
        effect: "⌘Q closes the Settings window instead of quitting Hex. Dictation keeps running; quit from the menu bar icon",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "expire-meeting-audio",
        name: "Expire meeting audio separately",
        class: Class::Changes,
        effect: "Hex may delete meeting audio before Sidenote's language pass runs.",
        action: "Show missing audio and preserve transcript-only access.",
    },
    Lab {
        id: "caps-lock-dictation",
        name: "Caps Lock dictation",
        class: Class::None,
        effect: "Press Caps Lock to start dictation and again to finish. Hex keeps Caps Lock off and uses its light while it records and transcribes",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "opencode-in-wsl",
        name: "OpenCode in WSL",
        class: Class::None,
        effect: "Hex rewrite provider selection; API Global processing is not requested.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "transcribe-files",
        name: "Transcribe files",
        class: Class::Opportunity,
        effect: "Hex can turn a saved WAV audio file into a transcript.",
        action: "Evaluate as an alternative without replacing the current path.",
    },
    Lab {
        id: "api-processing",
        name: "Process API transcripts",
        class: Class::Opportunity,
        effect: "Sidenote can ask Hex for transcripts with your corrections and rewrites applied.",
        action: "Keep raw transcripts by default.",
    },
    Lab {
        id: "api-timestamps",
        name: "Timestamps in API transcripts",
        class: Class::Opportunity,
        effect: "Sidenote can ask Hex for the timing of each spoken word or passage.",
        action: "Probe model support before opting in.",
    },
    Lab {
        id: "speakers-only",
        name: "Only on speakers",
        class: Class::None,
        effect: "Apply \"While dictating\" only when sound plays through speakers; headphones are left alone.",
        action: "No action pending deeper verification.",
    },
    Lab {
        id: "pause-menu",
        name: "Pause from menu bar",
        class: Class::None,
        effect: "Add Pause Hex to the menu bar. While paused, Hex ignores its shortcuts so other apps get the keys",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "more-dictation-shortcuts",
        name: "More dictation shortcuts",
        class: Class::None,
        effect: "Up to four more shortcuts that each dictate like the main one, each with its own mode",
        action: "No change to Sidenote meeting processing is needed.",
    },
    Lab {
        id: "annotate",
        name: "Annotate",
        class: Class::Opportunity,
        effect: "Sidenote can turn Hex screen recordings and drawings into notes.",
        action: "Use only the captures and frames the user selects for notes.",
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
        "labs.section_key_alone" => "section-key".into(),
        "experimental.hex_microphone" => "voice-filter".into(),
        "pause_browser_media" => "pause-browser".into(),
        _ => key.strip_prefix("labs.")?.replace('_', "-"),
    };
    valid_id(&id).then_some(id)
}

pub fn meetings_setting_can_enable(settings: &[Setting]) -> bool {
    settings
        .iter()
        .find(|setting| setting.key == "meetings.enabled")
        .is_some_and(|setting| {
            setting.kind == "bool"
                && setting.writable
                && setting.available != Some(false)
                && !setting.secret
                && setting.value.as_bool() == Some(false)
        })
}
/// Missing entries and invalid values are unknown, including in an enabled-only status list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub states: BTreeMap<String, Option<bool>>,
    pub source: &'static str,
    pub names: BTreeMap<String, String>,
}
impl Snapshot {
    pub fn name(&self, id: &str) -> String {
        if let Some(name) = self.names.get(id) {
            return name.clone();
        }
        if let Some(lab) = find(id) {
            return lab.name.to_owned();
        }
        let readable = id.replace('-', " ");
        let mut chars = readable.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    }
    pub fn group(&self, id: &str, show_all: bool) -> Option<Group> {
        let state = self.state(id);
        if state.is_none() && !show_all {
            return None;
        }
        match find(id).map(|lab| lab.class) {
            Some(Class::Changes | Class::Breaks) if state == Some(true) || show_all => {
                Some(Group::Affects)
            }
            Some(Class::Opportunity) => Some(Group::CanUse),
            None => Some(Group::New),
            _ => None,
        }
    }
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
                if let Some(name) = setting
                    .display_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    out.names.insert(id.clone(), name.to_owned());
                }
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
            source: "Hex status",
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
        // Windows hides Meetings from the Labs list, but status reports its explicit state.
        if let Some(enabled) = value["features"]["meetings"].as_bool() {
            out.states.insert("meetings".into(), Some(enabled));
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
    fn windows_meetings_off_is_explicit_even_when_hidden_from_labs() {
        let off = Snapshot::status(&serde_json::json!({"features":{"meetings":false},"labs":[]}));
        assert_eq!(off.state("meetings"), Some(false));
        let on = Snapshot::status(&serde_json::json!({"features":{"meetings":true},"labs":[]}));
        assert_eq!(on.state("meetings"), Some(true));
        let conflicting = Snapshot::status(
            &serde_json::json!({"features":{"meetings":false},"labs":["meetings"]}),
        );
        assert_eq!(conflicting.state("meetings"), Some(false));
        for value in [
            serde_json::json!({"labs":[]}),
            serde_json::json!({"features":{"meetings":"false"}}),
        ] {
            assert_eq!(Snapshot::status(&value).state("meetings"), None);
        }
    }
    #[test]
    fn registry_matches_review_and_ids_are_unique() {
        let catalog: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/hex-labs/catalog.json")).unwrap();
        assert_eq!(REGISTRY.len(), catalog.len());
        for entry in catalog {
            let id = entry["id"].as_str().unwrap();
            assert_eq!(find(id).unwrap().name, entry["name"].as_str().unwrap());
            assert_eq!(
                setting_id(entry["setting"].as_str().unwrap()).as_deref(),
                Some(id)
            );
        }
        assert_eq!(find("meetings").unwrap().class, Class::Changes);
        assert!(find("meetings").unwrap().action.starts_with("Required"));
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
    #[test]
    fn groups_hide_off_changes_and_unknown_states_until_show_all() {
        let mut snapshot = Snapshot::default();
        for (id, class_group) in [
            ("unload-idle", Group::Affects),
            ("api-processing", Group::CanUse),
            ("future-synthetic-lab", Group::New),
        ] {
            for state in [None, Some(false), Some(true)] {
                snapshot.states.insert(id.into(), state);
                let visible =
                    state.is_some() && (class_group != Group::Affects || state == Some(true));
                assert_eq!(
                    snapshot.group(id, false),
                    visible.then_some(class_group),
                    "{id} {state:?}"
                );
                assert_eq!(snapshot.group(id, true), Some(class_group));
            }
        }
        snapshot.states.insert("appearance".into(), Some(true));
        assert_eq!(snapshot.group("appearance", true), None);
        assert_eq!(snapshot.group("meetings", true), Some(Group::Affects));
        assert_eq!(snapshot.group("annotate", true), Some(Group::CanUse));
    }
    #[test]
    fn names_prefer_hex_metadata_then_catalog_then_readable_id() {
        let value: Value =
            serde_json::from_str(include_str!("../tests/fixtures/hex-labs/settings.json")).unwrap();
        let mut settings: Vec<Setting> = serde_json::from_value(value["settings"].clone()).unwrap();
        let snapshot = Snapshot::settings(&settings);
        assert_eq!(snapshot.name("future-synthetic-lab"), "New Hex experiment");
        assert_eq!(snapshot.name("api-processing"), "Process API transcripts");
        assert_eq!(snapshot.name("unknown-example-lab"), "Unknown example lab");
        assert_eq!(snapshot.name("meetings"), "Meetings (beta)");
        settings[0].display_name = Some("Name supplied by Hex".into());
        assert_eq!(
            Snapshot::settings(&settings).name("live-chapters"),
            "Name supplied by Hex"
        );
        for field in ["name", "label", "title", "display_name"] {
            let mut raw = serde_json::json!({"key":"labs.future_example", "kind":"bool", "value":false, "writable":true});
            raw[field] = Value::String("Hex display name".into());
            let setting: Setting = serde_json::from_value(raw).unwrap();
            assert_eq!(
                Snapshot::settings(&[setting]).name("future-example"),
                "Hex display name"
            );
        }
    }
}
