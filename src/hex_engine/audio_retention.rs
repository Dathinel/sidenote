use super::*;

pub struct AudioRetention {
    pub current: Option<&'static str>,
    pub editable: bool,
    pub follows_meetings: bool,
    legacy: bool,
}

impl AudioRetention {
    pub fn from_settings(settings: &[Setting]) -> Self {
        let audio = settings
            .iter()
            .find(|s| s.key == "meetings.audio_retention");
        let lab = settings
            .iter()
            .find(|s| s.key == "labs.expire_meeting_audio");
        let enabled = lab.map_or(Some(true), |s| s.value.as_bool());
        let current = match enabled {
            Some(false) => Some("Forever"),
            Some(true) if lab.is_none() && audio.is_some_and(|s| s.value.is_null()) => {
                Some("Same as meetings")
            }
            Some(true) => match audio.and_then(|s| s.value.as_str()) {
                Some("day") => Some("24 hours"),
                Some("week") => Some("7 days"),
                Some("month") => Some("1 month"),
                Some("forever") => Some("Forever"),
                _ => None,
            },
            None => None,
        };
        let editable = |s: &Setting| {
            s.writable
                && s.available != Some(false)
                && !s.secret
                && matches!(
                    s.kind.as_str(),
                    "bool" | "choice" | "number" | "integer" | "text" | "path"
                )
        };
        Self {
            current,
            editable: audio.is_some_and(editable) && lab.is_none_or(editable),
            follows_meetings: current == Some("Same as meetings")
                || (lab.is_some() && enabled == Some(true) && current.is_none()),
            legacy: lab.is_some(),
        }
    }

    pub fn choices(&self) -> &'static [&'static str] {
        if self.legacy {
            &["24 hours", "7 days", "1 month", "Forever"]
        } else {
            &[
                "Same as meetings",
                "24 hours",
                "7 days",
                "1 month",
                "Forever",
            ]
        }
    }

    pub fn set(&self, engine: &HexEngine, value: &str) -> Result<()> {
        if !self.editable {
            return Err(HexError::Unsupported);
        }
        engine.settings_set("meetings.audio_retention", value)?;
        if self.legacy {
            engine.settings_set(
                "labs.expire_meeting_audio",
                if value == "forever" { "false" } else { "true" },
            )
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
