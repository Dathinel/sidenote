//! Opt-in listener checks. The file probe measures whether growing WAVs are readable for a future
//! process-during-the-meeting design; it never reads audio samples.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    time::{Duration, Instant},
};
#[cfg(windows)]
use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Labs {
    pub language_detection: bool,
    pub after_stop_language_routing: bool,
    pub language_detection_settings: super::lid::Settings,
    pub stop_forgotten_recordings: bool,
    pub silence_minutes: SilenceMinutes,
    pub measure_recording_files: bool,
    pub process_meeting_windows: bool,
}
impl Default for Labs {
    fn default() -> Self {
        Self {
            language_detection: false,
            after_stop_language_routing: super::meeting_router::DEFAULT_CPU_PASSES,
            language_detection_settings: Default::default(),
            stop_forgotten_recordings: false,
            silence_minutes: Default::default(),
            measure_recording_files: false,
            process_meeting_windows: false,
        }
    }
}
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub enum SilenceMinutes {
    Ten,
    #[default]
    Twenty,
    Thirty,
    Sixty,
}
impl TryFrom<u64> for SilenceMinutes {
    type Error = &'static str;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            10 => Ok(Self::Ten),
            20 => Ok(Self::Twenty),
            30 => Ok(Self::Thirty),
            60 => Ok(Self::Sixty),
            _ => Err("Choose 10, 20, 30 or 60 minutes"),
        }
    }
}
impl From<SilenceMinutes> for u64 {
    fn from(value: SilenceMinutes) -> Self {
        value.minutes()
    }
}
impl SilenceMinutes {
    pub fn minutes(self) -> u64 {
        match self {
            Self::Ten => 10,
            Self::Twenty => 20,
            Self::Thirty => 30,
            Self::Sixty => 60,
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.bytes().any(|b| b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
pub(crate) fn shared_read(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    #[cfg(not(windows))]
    let file = super::file_reads::nofollow(path)?;
    if !super::file_reads::regular(&file.metadata()?) {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(file)
}

#[derive(Clone, Copy, Default, Serialize)]
struct Counts {
    words: u64,
    lines: u64,
}
fn rolling_counts(folder: &Path, words: bool) -> io::Result<Counts> {
    let file = match shared_read(&folder.join("transcript.rolling.ndjson")) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Counts::default()),
        Err(e) => return Err(e),
    };
    let size = file.metadata()?.len();
    if size > super::live::ROLLING_MAX_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    if !words {
        let mut reader = file.take(size);
        let mut buffer = [0; 8192];
        let mut lines = 0;
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            lines += buffer[..count].iter().filter(|b| **b == b'\n').count() as u64;
        }
        return Ok(Counts { words: 0, lines });
    }
    let mut reader = BufReader::new(file.take(size));
    let mut line = Vec::new();
    let mut counts = Counts::default();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        counts.lines += u64::from(line.ends_with(b"\n"));
        #[derive(Deserialize)]
        struct Text {
            text: Option<String>,
        }
        // An unfinished write makes the count unknown and resets the silence timer.
        let value: Text = serde_json::from_slice(&line)?;
        counts.words += value
            .text
            .as_deref()
            .map_or(0, |t| t.split_whitespace().count() as u64);
    }
    Ok(counts)
}

#[derive(Default)]
struct SilenceTimer {
    baseline: Option<(String, u64, Duration)>,
}
impl SilenceTimer {
    fn check(&mut self, recording: Option<(&str, u64)>, now: Duration, minutes: u64) -> bool {
        let Some((id, words)) = recording else {
            self.baseline = None;
            return false;
        };
        let Some((old_id, old_words, since)) = &self.baseline else {
            self.baseline = Some((id.into(), words, now));
            return false;
        };
        if old_id != id || *old_words != words {
            self.baseline = Some((id.into(), words, now));
            return false;
        }
        now.saturating_sub(*since) >= Duration::from_secs(minutes * 60)
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct WavHeader {
    pub(crate) riff_bytes: u32,
    pub(crate) format_tag: u16,
    pub(crate) encoding_tag: u16,
    pub(crate) channels: u16,
    pub(crate) sample_rate: u32,
    pub(crate) byte_rate: u32,
    pub(crate) block_align: u16,
    pub(crate) bits_per_sample: u16,
    pub(crate) declared_data_bytes: u32,
    pub(crate) data_offset: u64,
    pub(crate) visible_data_bytes: u64,
    /// Complete sample frames per channel, ignoring an incomplete final frame.
    pub(crate) sample_count: u64,
    pub(crate) trailing_bytes: u64,
}
pub(crate) fn wav_header(reader: &mut (impl Read + Seek), length: u64) -> io::Result<WavHeader> {
    let mut riff = [0; 12];
    reader.read_exact(&mut riff)?;
    if &riff[..4] != b"RIFF" || &riff[8..] != b"WAVE" {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut fmt = None;
    let mut encoding_tag = 0;
    let mut offset = 12u64;
    // Read format metadata, including the extensible subtype. Unknown chunks are skipped.
    while offset <= 64 * 1024 && offset.saturating_add(8) <= length {
        reader.seek(SeekFrom::Start(offset))?;
        let mut chunk = [0; 8];
        reader.read_exact(&mut chunk)?;
        let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
        offset += 8;
        if &chunk[..4] == b"data" {
            let f: [u8; 16] = fmt.ok_or(io::ErrorKind::InvalidData)?;
            let u16_at = |i| u16::from_le_bytes([f[i], f[i + 1]]);
            let u32_at = |i| u32::from_le_bytes(f[i..i + 4].try_into().unwrap());
            let block_align = u16_at(12);
            if block_align == 0 || u16_at(2) == 0 || u16_at(14) == 0 || u32_at(4) == 0 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let visible = length.saturating_sub(offset);
            return Ok(WavHeader {
                riff_bytes: u32::from_le_bytes(riff[4..8].try_into().unwrap()),
                format_tag: u16_at(0),
                encoding_tag,
                channels: u16_at(2),
                sample_rate: u32_at(4),
                byte_rate: u32_at(8),
                block_align,
                bits_per_sample: u16_at(14),
                declared_data_bytes: size,
                data_offset: offset,
                visible_data_bytes: visible,
                sample_count: visible / u64::from(block_align),
                trailing_bytes: visible % u64::from(block_align),
            });
        }
        if &chunk[..4] == b"fmt " {
            if size < 16 || offset + 16 > length {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let mut bytes = [0; 16];
            reader.read_exact(&mut bytes)?;
            encoding_tag = u16::from_le_bytes(bytes[..2].try_into().unwrap());
            if encoding_tag == 0xfffe {
                if size < 40 || offset + 40 > length {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                let mut extension = [0; 24];
                reader.read_exact(&mut extension)?;
                let guid_tail = [0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71];
                if u16::from_le_bytes(extension[..2].try_into().unwrap()) < 22
                    || extension[10..] != guid_tail
                {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                encoding_tag = u16::from_le_bytes(extension[8..10].try_into().unwrap());
            }
            fmt = Some(bytes);
        }
        offset += u64::from(size) + u64::from(size % 2);
    }
    Err(io::ErrorKind::InvalidData.into())
}

#[derive(Clone, Serialize)]
struct Track {
    file_length: Option<u64>,
    open_succeeded: bool,
    open_error_kind: Option<String>,
    header_error_kind: Option<String>,
    header: Option<WavHeader>,
}
fn inspect_track(path: &Path, open: &mut impl FnMut(&Path) -> io::Result<File>) -> Track {
    let mut track = Track {
        file_length: None,
        open_succeeded: false,
        open_error_kind: None,
        header_error_kind: None,
        header: None,
    };
    match open(path) {
        Ok(mut file) => {
            track.open_succeeded = true;
            let result = file.metadata().and_then(|meta| {
                track.file_length = Some(meta.len());
                wav_header(&mut file, meta.len())
            });
            match result {
                Ok(header) => track.header = Some(header),
                Err(e) => track.header_error_kind = Some(format!("{:?}", e.kind())),
            }
        }
        Err(e) => {
            track.file_length = fs::metadata(path).ok().map(|m| m.len());
            track.open_error_kind = Some(format!("{:?}", e.kind()));
        }
    }
    track
}
#[derive(Clone, Serialize)]
struct Sample {
    at_ms: i64,
    rolling_line_count: Option<u64>,
    rolling_error_kind: Option<String>,
    directory_error_kind: Option<String>,
    tracks: BTreeMap<String, Track>,
}
fn sample(folder: &Path, open: &mut impl FnMut(&Path) -> io::Result<File>) -> Sample {
    let counts = rolling_counts(folder, false);
    let mut names: BTreeSet<String> = ["microphone.wav".into(), "system.wav".into()].into();
    let mut directory_error_kind = None;
    match fs::read_dir(folder) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry)
                        if entry
                            .path()
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("wav")) =>
                    {
                        names.insert(entry.file_name().to_string_lossy().into_owned());
                    }
                    Err(e) => directory_error_kind = Some(format!("{:?}", e.kind())),
                    _ => (),
                }
            }
        }
        Err(e) => directory_error_kind = Some(format!("{:?}", e.kind())),
    }
    Sample {
        at_ms: chrono::Utc::now().timestamp_millis(),
        rolling_line_count: counts.as_ref().ok().map(|c| c.lines),
        rolling_error_kind: counts.err().map(|e| format!("{:?}", e.kind())),
        directory_error_kind,
        tracks: names
            .into_iter()
            .map(|name| {
                let track = inspect_track(&folder.join(&name), open);
                (name, track)
            })
            .collect(),
    }
}
#[derive(Serialize)]
struct Comparison {
    last_file_length: Option<u64>,
    final_file_length: Option<u64>,
    file_length_delta: Option<i128>,
    last_sample_count: Option<u64>,
    final_sample_count: Option<u64>,
    sample_count_delta: Option<i128>,
}
#[derive(Serialize)]
struct Probe {
    schema: u32,
    meeting_id: String,
    samples: Vec<Sample>,
    final_sample: Option<Sample>,
    comparison: BTreeMap<String, Comparison>,
}
impl Probe {
    fn new(id: &str) -> Self {
        Self {
            schema: 1,
            meeting_id: id.into(),
            samples: Vec::new(),
            final_sample: None,
            comparison: BTreeMap::new(),
        }
    }
    fn finish(&mut self, final_sample: Sample) {
        for (name, track) in &final_sample.tracks {
            let last = self.samples.last().and_then(|s| s.tracks.get(name));
            let last_length = last.and_then(|t| t.file_length);
            let last_count = last.and_then(|t| t.header.as_ref()).map(|h| h.sample_count);
            let final_count = track.header.as_ref().map(|h| h.sample_count);
            self.comparison.insert(
                name.clone(),
                Comparison {
                    last_file_length: last_length,
                    final_file_length: track.file_length,
                    file_length_delta: last_length
                        .zip(track.file_length)
                        .map(|(a, b)| i128::from(b) - i128::from(a)),
                    last_sample_count: last_count,
                    final_sample_count: final_count,
                    sample_count_delta: last_count
                        .zip(final_count)
                        .map(|(a, b)| i128::from(b) - i128::from(a)),
                },
            );
        }
        self.final_sample = Some(final_sample);
    }
    fn save(&self, state: &Path) -> io::Result<()> {
        super::store_io::write(
            &state
                .join("local/probe")
                .join(format!("{}.json", self.meeting_id)),
            self,
        )
    }
}

pub(crate) fn recording_id(
    status: &crate::hex_engine::EngineState,
    meetings: &Path,
) -> Option<String> {
    if status.meeting_state != "recording" || status.dictating != Some(false) {
        return None;
    }
    let recording = status.recording.as_ref()?;
    let id = match &recording.id {
        Some(id) => id.clone(),
        None => {
            let start_ms = recording
                .since?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis()
                .try_into()
                .ok()?;
            super::live::find_folder(meetings, None, Some(start_ms))?
                .file_name()?
                .to_str()?
                .to_owned()
        }
    };
    valid_id(&id).then_some(id)
}
#[derive(Default)]
struct Watch {
    timer: SilenceTimer,
    next_silence: Duration,
    probe: Option<(PathBuf, Probe)>,
    attempted: BTreeSet<String>,
}
impl Watch {
    fn finished(&mut self, id: &str, state: &Path) {
        if self.probe.as_ref().is_some_and(|(_, p)| p.meeting_id == id) {
            let (folder, mut probe) = self.probe.take().unwrap();
            let mut final_sample = sample(&folder, &mut shared_read);
            if let Some(last) = probe.samples.last() {
                for name in last.tracks.keys() {
                    final_sample
                        .tracks
                        .entry(name.clone())
                        .or_insert_with(|| inspect_track(&folder.join(name), &mut shared_read));
                }
            }
            probe.finish(final_sample);
            let _ = probe.save(state);
        }
    }
    fn tick(
        &mut self,
        labs: &Labs,
        status: Option<&crate::hex_engine::EngineState>,
        (state, meetings): (&Path, &Path),
        now: Duration,
        mut stop: impl FnMut(&str) -> bool,
        mut log: impl FnMut(&str),
    ) {
        if !labs.measure_recording_files {
            self.probe = None;
        }
        let id = status.and_then(|status| recording_id(status, meetings));
        let id = id.as_deref();
        if let Some(status) = status
            && matches!(
                status.meeting_state.as_str(),
                "idle" | "stopped" | "complete" | "finished"
            )
            && let Some((_, probe)) = &self.probe
        {
            self.finished(&probe.meeting_id.clone(), state);
        }
        if labs.measure_recording_files
            && let Some(id) = id
        {
            let folder = meetings.join(id);
            if self.probe.as_ref().is_some_and(|(_, p)| p.meeting_id != id) {
                let previous = self.probe.as_ref().unwrap().1.meeting_id.clone();
                self.finished(&previous, state);
            }
            let (folder, probe) = self.probe.get_or_insert_with(|| (folder, Probe::new(id)));
            probe.samples.push(sample(folder, &mut shared_read));
            let _ = probe.save(state);
        }
        if !labs.stop_forgotten_recordings || id.is_none() {
            self.timer.baseline = None;
            self.next_silence = now;
            return;
        }
        if now < self.next_silence {
            return;
        }
        self.next_silence = now + Duration::from_secs(60);
        let id = id.unwrap();
        let counts = rolling_counts(&meetings.join(id), true);
        let minutes = labs.silence_minutes.minutes();
        if !self
            .timer
            .check(counts.ok().map(|c| (id, c.words)), now, minutes)
        {
            return;
        }
        let marker = state.join("local/auto-stop").join(format!("{id}.json"));
        if self.attempted.insert(id.into()) && !marker.exists()
            // Persist the attempt before invoking Hex, including across listener restarts.
            && super::store_io::write(&marker, &true).is_ok() && stop(id)
        {
            log(&format!(
                "auto-stopped meeting {id} after {minutes} min without new words"
            ));
        }
    }
}

pub(crate) fn start(log: fn(&str)) -> Sender<String> {
    start_managed(log).0
}
#[cfg(windows)]
fn dispatch_completions(
    source: &mut Result<
        super::recording_source::Session<super::recording_source::folders::Discovery>,
        super::recording_source::Error,
    >,
    log: fn(&str),
) {
    let result = match source {
        Ok(source) => source
            .drain(|meeting| crate::listener::completion::handoff(meeting.id())?.acknowledge()),
        Err(error) => {
            log(&format!("recording source: {error:?}"));
            return;
        }
    };
    if let Err(error) = result {
        log(&format!("recording source: {error:?}"));
    }
}
pub(crate) fn start_managed(log: fn(&str)) -> (Sender<String>, std::thread::JoinHandle<()>) {
    let (send, receive) = mpsc::channel::<String>();
    let worker = std::thread::spawn(move || {
        let mut watch = Watch::default();
        let engine = crate::hex_engine::HexEngine::new();
        let start = Instant::now();
        let state = super::state_dir();
        let meetings = super::meetings_dir();
        #[cfg(windows)]
        let mut source = super::recording_source::Session::open(
            super::recording_source::folders::Discovery::configured(meetings.clone()),
            state.join("local/recording-source.json"),
        );
        loop {
            let pass = super::idle_pass::Span::begin("watch_pass");
            let labs = super::store_io::defer_busy(super::config::Config::load_in(&state));
            if let Ok(Some(config)) = &labs {
                let labs = &config.labs;
                let status = (labs.stop_forgotten_recordings || labs.measure_recording_files)
                    .then(|| engine.status().ok())
                    .flatten();
                watch.tick(
                    labs,
                    status.as_ref(),
                    (&state, &meetings),
                    start.elapsed(),
                    |id| engine.stop_forgotten(id).is_ok(),
                    log,
                );
            } else if labs.is_err() {
                watch.timer.baseline = None;
            }
            #[cfg(windows)]
            dispatch_completions(&mut source, log);
            drop(pass);
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(id) => {
                        watch.finished(&id, &state);
                        #[cfg(windows)]
                        dispatch_completions(&mut source, log);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
                if Instant::now() >= deadline {
                    break;
                }
            }
            if crate::updates::listener_stopping() {
                return;
            }
        }
    });
    (send, worker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn fixture() -> PathBuf {
        let root = tempfile::tempdir().unwrap().keep();
        fs::create_dir_all(root.join("meetings/100-1")).unwrap();
        root
    }
    fn status(meeting: &str, dictating: Option<bool>) -> crate::hex_engine::EngineState {
        crate::hex_engine::EngineState {
            running: true,
            version: "synthetic".into(),
            protocol: 4,
            recording: Some(crate::hex_engine::Recording {
                id: Some("100-1".into()),
                title: None,
                since: None,
                elapsed_seconds: Some(1),
            }),
            meeting_state: meeting.into(),
            dictating,
            capture_state: "idle".into(),
            model: serde_json::from_value(serde_json::json!({})).unwrap(),
            labs: Box::default(),
        }
    }
    fn wav(data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend(36u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(64000u32.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(data);
        bytes
    }
    fn tick(
        watch: &mut Watch,
        labs: &Labs,
        status: Option<&crate::hex_engine::EngineState>,
        root: &Path,
        seconds: u64,
        calls: &mut usize,
    ) {
        watch.tick(
            labs,
            status,
            (root, &root.join("meetings")),
            Duration::from_secs(seconds),
            |_| {
                *calls += 1;
                true
            },
            |_| (),
        );
    }
    #[test]
    fn defaults_and_only_four_durations_round_trip() {
        let defaults: Labs = serde_json::from_str("{}").unwrap();
        assert!(!defaults.stop_forgotten_recordings && !defaults.measure_recording_files);
        assert_eq!(defaults.silence_minutes.minutes(), 20);
        let config: super::super::config::Config =
            serde_json::from_str(r#"{"version":1,"features":{}}"#).unwrap();
        assert!(!config.labs.stop_forgotten_recordings && !config.labs.measure_recording_files);
        for n in [10, 20, 30, 60] {
            let labs: Labs =
                serde_json::from_value(serde_json::json!({"silence_minutes": n})).unwrap();
            assert_eq!(labs.silence_minutes.minutes(), n);
            assert_eq!(serde_json::to_value(labs).unwrap()["silence_minutes"], n);
            let mut timer = SilenceTimer::default();
            assert!(!timer.check(Some(("100-1", 0)), Duration::ZERO, n));
            assert!(!timer.check(Some(("100-1", 0)), Duration::from_secs(n * 60 - 1), n));
            assert!(timer.check(Some(("100-1", 0)), Duration::from_secs(n * 60), n));
        }
        assert!(serde_json::from_value::<Labs>(serde_json::json!({"silence_minutes":15})).is_err());
    }
    #[test]
    fn new_words_and_truncation_restart_the_timer() {
        let mut timer = SilenceTimer::default();
        for minute in 0..40 {
            assert!(!timer.check(
                Some(("100-1", minute)),
                Duration::from_secs(minute * 60),
                20
            ));
        }
        assert!(!timer.check(Some(("100-1", 1)), Duration::from_secs(3600), 20));
        assert!(!timer.check(Some(("100-2", 1)), Duration::from_secs(4800), 20));
        assert!(timer.check(Some(("100-2", 1)), Duration::from_secs(6000), 20));
    }
    #[test]
    fn stall_stops_once_across_disable_and_restart_and_only_logs_counts() {
        let root = fixture();
        let labs = Labs {
            stop_forgotten_recordings: true,
            ..Labs::default()
        };
        let recording = status("recording", Some(false));
        let mut watch = Watch::default();
        let mut calls = 0;
        tick(&mut watch, &labs, Some(&recording), &root, 0, &mut calls);
        tick(&mut watch, &labs, Some(&recording), &root, 1199, &mut calls);
        let mut logs = Vec::new();
        watch.tick(
            &labs,
            Some(&recording),
            (&root, &root.join("meetings")),
            Duration::from_secs(1260),
            |_| {
                calls += 1;
                true
            },
            |s| logs.push(s.to_owned()),
        );
        tick(
            &mut watch,
            &Labs::default(),
            Some(&recording),
            &root,
            1300,
            &mut calls,
        );
        for seconds in [1400, 2700, 4000] {
            tick(
                &mut watch,
                &labs,
                Some(&recording),
                &root,
                seconds,
                &mut calls,
            );
        }
        let mut restarted = Watch::default();
        for seconds in [0, 1200, 2400] {
            tick(
                &mut restarted,
                &labs,
                Some(&recording),
                &root,
                seconds,
                &mut calls,
            );
        }
        assert_eq!(calls, 1);
        assert_eq!(
            logs,
            ["auto-stopped meeting 100-1 after 20 min without new words"]
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unknown_dictation_starting_errors_and_disabled_never_stop() {
        let root = fixture();
        let labs = Labs {
            stop_forgotten_recordings: true,
            ..Labs::default()
        };
        let recording = status("recording", Some(false));
        let mut calls = 0;
        for invalid in [
            None,
            Some(status("unknown", Some(false))),
            Some(status("recording", Some(true))),
            Some(status("recording", None)),
            Some(status("starting", Some(false))),
        ] {
            let mut watch = Watch::default();
            tick(&mut watch, &labs, Some(&recording), &root, 0, &mut calls);
            tick(&mut watch, &labs, invalid.as_ref(), &root, 1200, &mut calls);
            tick(&mut watch, &labs, Some(&recording), &root, 1260, &mut calls);
            tick(&mut watch, &labs, Some(&recording), &root, 2400, &mut calls);
        }
        let mut watch = Watch::default();
        for seconds in [0, 1200, 2400] {
            tick(
                &mut watch,
                &Labs::default(),
                Some(&recording),
                &root,
                seconds,
                &mut calls,
            );
        }
        fs::write(
            root.join("meetings/100-1/transcript.rolling.ndjson"),
            b"invalid json\n",
        )
        .unwrap();
        let mut watch = Watch::default();
        for seconds in [0, 1200, 2400] {
            tick(
                &mut watch,
                &labs,
                Some(&recording),
                &root,
                seconds,
                &mut calls,
            );
        }
        assert_eq!(calls, 0);
        assert!(!root.join("local/probe").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn counts_ignore_header_and_treat_unfinished_writes_as_unknown() {
        let root = fixture();
        let folder = root.join("meetings/100-1");
        fs::write(
            folder.join("transcript.rolling.ndjson"),
            b"{\"covered_ms\":0}\n{\"text\":\"invented two words\"}\n{\"text\":\"unfinished",
        )
        .unwrap();
        assert!(rolling_counts(&folder, true).is_err());
        assert_eq!(rolling_counts(&folder, false).unwrap().lines, 2);
        fs::write(
            folder.join("transcript.rolling.ndjson"),
            b"{\"covered_ms\":0}\n{\"text\":\"invented two words\"}",
        )
        .unwrap();
        let counts = rolling_counts(&folder, true).unwrap();
        assert_eq!((counts.words, counts.lines), (3, 1));
        assert_eq!(
            serde_json::to_string(&counts).unwrap(),
            r#"{"words":3,"lines":1}"#
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_status_id_uses_the_existing_unique_active_folder_lookup() {
        let root = fixture();
        let meetings = root.join("meetings");
        let mut recording = status("recording", Some(false));
        let info = recording.recording.as_mut().unwrap();
        info.id = None;
        info.since = Some(std::time::UNIX_EPOCH + Duration::from_millis(1000));
        fs::write(
            meetings.join("100-1/manifest.json"),
            br#"{"status":"recording","created_at_ms":1000}"#,
        )
        .unwrap();
        assert_eq!(
            recording_id(&recording, &meetings).as_deref(),
            Some("100-1")
        );
        fs::create_dir_all(meetings.join("100-2")).unwrap();
        fs::write(
            meetings.join("100-2/manifest.json"),
            br#"{"status":"recording","created_at_ms":1000}"#,
        )
        .unwrap();
        assert!(recording_id(&recording, &meetings).is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_wav_header_growth_and_partial_frame_never_read_samples() {
        for (data_len, count, tail) in [(0, 0, 0), (8, 2, 0), (11, 2, 3)] {
            let bytes = wav(&vec![0xaa; data_len]);
            let mut cursor = Cursor::new(&bytes);
            let h = wav_header(&mut cursor, bytes.len() as u64).unwrap();
            assert_eq!((h.riff_bytes, h.declared_data_bytes), (36, 0));
            assert_eq!(
                (h.channels, h.sample_rate, h.bits_per_sample, h.block_align),
                (2, 16000, 16, 4)
            );
            assert_eq!((h.sample_count, h.trailing_bytes), (count, tail));
            assert_eq!(cursor.position(), 44);
        }
        let bytes = wav(&[]);
        for n in [0, 10, 20, 35, 42] {
            assert!(wav_header(&mut Cursor::new(&bytes[..n]), n as u64).is_err());
        }
    }
    #[test]
    fn shared_open_while_writer_is_alive_and_failure_does_not_stop_other_tracks() {
        let root = fixture();
        let folder = root.join("meetings/100-1");
        let path = folder.join("system.wav");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(windows)]
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
        let mut writer = options.open(&path).unwrap();
        writer.write_all(&wav(&[0; 8])).unwrap();
        writer.flush().unwrap();
        let mut first = sample(&folder, &mut shared_read);
        assert!(first.tracks["system.wav"].open_succeeded);
        assert_eq!(
            first.tracks["system.wav"]
                .header
                .as_ref()
                .unwrap()
                .sample_count,
            2
        );
        assert_eq!(
            first.tracks["microphone.wav"].open_error_kind.as_deref(),
            Some("NotFound")
        );
        writer.write_all(&[0; 4]).unwrap();
        writer.flush().unwrap();
        let next = sample(&folder, &mut |p| {
            if p.file_name().unwrap() == "microphone.wav" {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                shared_read(p)
            }
        });
        assert_eq!(
            next.tracks["microphone.wav"].open_error_kind.as_deref(),
            Some("PermissionDenied")
        );
        assert_eq!(
            next.tracks["system.wav"]
                .header
                .as_ref()
                .unwrap()
                .sample_count,
            3
        );
        first.at_ms = 1;
        let mut probe = Probe::new("100-1");
        probe.samples.push(first);
        probe.finish(next);
        probe.save(&root).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("local/probe/100-1.json")).unwrap())
                .unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["meeting_id"], "100-1");
        assert_eq!(value["samples"].as_array().unwrap().len(), 1);
        assert_eq!(value["comparison"]["system.wav"]["file_length_delta"], 4);
        assert_eq!(value["comparison"]["system.wav"]["sample_count_delta"], 1);
        assert_eq!(
            value["final_sample"]["tracks"]["microphone.wav"]["open_succeeded"],
            false
        );
        drop(writer);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn finished_event_and_known_idle_save_final_sample_once() {
        for event in [true, false] {
            let root = fixture();
            let labs = Labs {
                measure_recording_files: true,
                ..Labs::default()
            };
            let mut watch = Watch::default();
            let mut calls = 0;
            tick(
                &mut watch,
                &labs,
                Some(&status("recording", Some(false))),
                &root,
                0,
                &mut calls,
            );
            tick(&mut watch, &labs, None, &root, 30, &mut calls);
            assert!(watch.probe.is_some());
            if event {
                watch.finished("100-1", &root);
                watch.finished("100-1", &root);
            } else {
                tick(
                    &mut watch,
                    &labs,
                    Some(&status("idle", Some(false))),
                    &root,
                    60,
                    &mut calls,
                );
            }
            assert!(watch.probe.is_none());
            assert_eq!(calls, 0);
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("local/probe/100-1.json")).unwrap())
                    .unwrap();
            assert!(value["final_sample"].is_object());
            assert_eq!(value["samples"].as_array().unwrap().len(), 1);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
