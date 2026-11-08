use crate::{entity::Meeting, ids::Id};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Reported,
    Requested,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub meeting_ids: Vec<Id<Meeting>>,
    pub capped: bool,
}

/// Open strings for harness, model and task names also support third-party agents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Provenance {
    pub harness: Option<String>,
    pub harness_version: Option<String>,
    pub model: Option<String>,
    pub model_source: ModelSource,
    pub model_reported: Option<String>,
    pub model_requested: Option<String>,
    pub source: Option<String>,
    pub agent_name: Option<String>,
    pub task_kind: Option<String>,
    pub duration_ms: Option<u64>,
    pub tools_used: Vec<String>,
    pub context: Context,
}

impl Provenance {
    /// Keep both observations, using reported, then requested, then unknown.
    pub fn from_models(reported: Option<String>, requested: Option<String>) -> Self {
        let reported = reported.filter(|value| !value.trim().is_empty());
        let requested = requested.filter(|value| !value.trim().is_empty());
        let (model, model_source) = match (&reported, &requested) {
            (Some(model), _) => (Some(model.clone()), ModelSource::Reported),
            (None, Some(model)) => (Some(model.clone()), ModelSource::Requested),
            (None, None) => (None, ModelSource::Unknown),
        };
        Self {
            model,
            model_source,
            model_reported: reported,
            model_requested: requested,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_never_guesses_a_model() {
        for (reported, requested, expected, source) in [
            (
                Some("reported"),
                Some("requested"),
                Some("reported"),
                ModelSource::Reported,
            ),
            (
                None,
                Some("requested"),
                Some("requested"),
                ModelSource::Requested,
            ),
            (None, None, None, ModelSource::Unknown),
            (
                Some("  "),
                Some("requested"),
                Some("requested"),
                ModelSource::Requested,
            ),
            (
                Some("reported"),
                None,
                Some("reported"),
                ModelSource::Reported,
            ),
        ] {
            let provenance =
                Provenance::from_models(reported.map(str::to_owned), requested.map(str::to_owned));
            assert_eq!(provenance.model.as_deref(), expected);
            assert_eq!(provenance.model_source, source);
        }
        assert_eq!(Provenance::default().model_source, ModelSource::Unknown);
    }
}
