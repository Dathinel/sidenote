use super::*;
use std::io::BufReader;

/// Seekable PCM16 sources. Only a bounded inference window is expanded to f32.
pub struct Audio {
    tracks: Vec<(String, PathBuf, hound::WavReader<BufReader<fs::File>>)>,
}
impl Audio {
    pub fn open(paths: &[(String, PathBuf)]) -> Result<Self> {
        let mut tracks = Vec::new();
        for (track, path) in paths {
            let reader = hound::WavReader::open(path)?;
            let spec = reader.spec();
            if spec.sample_rate != 16000
                || spec.channels != 1
                || spec.bits_per_sample != 16
                || spec.sample_format != hound::SampleFormat::Int
            {
                return Err("Audio must be mono 16 kHz PCM16 WAV".into());
            }
            tracks.push((track.clone(), path.clone(), reader));
        }
        if tracks.is_empty() {
            return Err("No audio tracks".into());
        }
        Ok(Self { tracks })
    }
    pub fn duration(&self) -> f64 {
        self.tracks
            .iter()
            .map(|(_, _, r)| r.duration() as f64 / 16000.0)
            .fold(0.0, f64::max)
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.tracks.iter().map(|(_, p, _)| p.clone()).collect()
    }
    pub fn track_duration(&self, track: &str) -> Option<f64> {
        self.tracks
            .iter()
            .find(|(t, _, _)| t == track)
            .map(|(_, _, r)| r.duration() as f64 / 16000.0)
    }
    pub fn read(&mut self, chunk: &Chunk) -> Result<Vec<f32>> {
        let index = if self.tracks.len() == 1 {
            0
        } else {
            self.tracks
                .iter()
                .position(|(t, _, _)| t == &chunk.track)
                .ok_or("Unknown audio track")?
        };
        let reader = &mut self.tracks[index].2;
        let start = chunk.audio_start.unwrap_or(chunk.start);
        let length = chunk.end - chunk.start;
        if !start.is_finite()
            || !length.is_finite()
            || start < 0.0
            || !(0.1..=24.00001).contains(&length)
        {
            return Err("Invalid speech window".into());
        }
        let a = (start * 16000.0).round() as u32;
        let n = (length * 16000.0).round() as u32;
        if a.checked_add(n).is_none_or(|end| end > reader.duration()) {
            return Err("Speech window exceeds WAV".into());
        }
        reader.seek(a)?;
        let pcm = reader
            .samples::<i16>()
            .take(n as usize)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if pcm.len() != n as usize {
            return Err("Truncated WAV".into());
        }
        Ok(pcm.into_iter().map(|s| s as f32 / 32768.0).collect())
    }
    pub fn speech_chunks(&mut self) -> Result<Vec<Chunk>> {
        let mut chunks = Vec::new();
        for (track, _, reader) in &mut self.tracks {
            reader.seek(0)?;
            let total = reader.duration() as usize;
            let mut samples = reader.samples::<i16>();
            let mut frame_index = 0usize;
            let mut start = None;
            let mut last = 0;
            loop {
                let frame = samples
                    .by_ref()
                    .take(320)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if frame.is_empty() {
                    break;
                }
                let energy = frame
                    .iter()
                    .map(|&s| (s as f64 / 32768.0).powi(2))
                    .sum::<f64>()
                    / frame.len() as f64;
                let speech = energy > 0.00001;
                if speech {
                    start.get_or_insert(frame_index.saturating_sub(10));
                    last = frame_index;
                }
                if let Some(a) = start
                    && ((!speech && frame_index.saturating_sub(last) >= 20)
                        || (frame_index + 1) * 320 >= total)
                {
                    let a = a * 320;
                    let b = ((last + 11) * 320).min(total);
                    if b.saturating_sub(a) >= 1600 {
                        let count = (b - a).div_ceil(MAX_SAMPLES);
                        for i in 0..count {
                            chunks.push(Chunk {
                                id: format!("{track}-speech-{}", chunks.len()),
                                start: (a + (b - a) * i / count) as f64 / 16000.0,
                                end: (a + (b - a) * (i + 1) / count) as f64 / 16000.0,
                                track: track.clone(),
                                speaker: None,
                                turn: None,
                                english: String::new(),
                                audio_start: None,
                                words: Vec::new(),
                            });
                        }
                    }
                    start = None;
                }
                frame_index += 1;
            }
        }
        chunks.sort_by(|a, b| a.start.total_cmp(&b.start));
        Ok(chunks)
    }
}
