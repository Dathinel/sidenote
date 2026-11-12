use super::*;
use crate::core::meeting_router::Chunk;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    pub wav: PathBuf,
    pub chunks: Vec<Chunk>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub meeting_languages: Vec<String>,
    pub jobs: Vec<Job>,
}
#[derive(Serialize, Deserialize)]
pub struct Sidecar {
    pub job: String,
    pub source_wav: PathBuf,
    pub schema: String,
    pub experimental: bool,
    pub thresholds_calibrated: bool,
    pub confidence_semantics: String,
    pub model_sha256: String,
    pub settings: Settings,
    pub language_mode: String,
    pub meeting_languages: Vec<String>,
    pub chunks: Vec<ChunkResult>,
    pub warnings: Vec<String>,
}

/// Stable audio identity and evidence for M6; no transcript text is copied.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkResult {
    #[serde(flatten)]
    pub guess: Guess,
    pub start: f64,
    pub end: f64,
    pub track: String,
    pub speaker: Option<String>,
    pub turn: Option<String>,
    pub audio_start: Option<f64>,
    pub pcm_f32le_sha256: Option<String>,
    pub inherited_from: Option<String>,
}
impl ChunkResult {
    pub fn for_chunk(
        chunk: &Chunk,
        mut guess: Guess,
        pcm_f32le_sha256: Option<String>,
        inherited_from: Option<String>,
    ) -> Self {
        guess.chunk.clone_from(&chunk.id);
        Self {
            guess,
            start: chunk.start,
            end: chunk.end,
            track: chunk.track.clone(),
            speaker: chunk.speaker.clone(),
            turn: chunk.turn.clone(),
            audio_start: chunk.audio_start,
            pcm_f32le_sha256,
            inherited_from,
        }
    }
}
