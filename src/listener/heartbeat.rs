//! Device-local recorder availability; contains timestamps only.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{fs, io, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub from: i64,
    pub to: i64,
}
impl Gap {
    pub fn notice(&self) -> String {
        let local = |seconds| {
            chrono::DateTime::from_timestamp(seconds, 0)
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%b %d, %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "unknown time".into())
        };
        format!(
            "Recorder was off from {} to {}.",
            local(self.from),
            local(self.to)
        )
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    started: i64,
    last_seen: i64,
    stopped: Option<i64>,
    gap: Option<Gap>,
}
impl Record {
    fn restart(previous: Option<Self>, now: i64) -> Self {
        let mut gap = previous.as_ref().and_then(|record| record.gap.clone());
        if let Some(previous) = previous {
            let from = previous.stopped.unwrap_or(previous.last_seen);
            // Ignore corrupt or future clock values rather than inventing downtime.
            if previous.started <= previous.last_seen
                && previous.last_seen <= from
                && from < now - 120
            {
                gap = Some(Gap { from, to: now });
            }
        }
        Self {
            started: now,
            last_seen: now,
            stopped: None,
            gap,
        }
    }
}

fn directory() -> std::path::PathBuf {
    crate::core::state_dir().join("local")
}
fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn write(path: &Path, value: &impl Serialize) -> io::Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec(value)?)?;
    crate::platform::replace_file(&temp, path)
}
fn pending_in(root: &Path) -> Option<Gap> {
    let record: Record = read(&root.join("recorder-heartbeat.json"))?;
    let gap = record.gap?;
    let dismissed: Option<Gap> = read(&root.join("recorder-gap-dismissed.json"));
    (dismissed.as_ref() != Some(&gap)).then_some(gap)
}
pub fn pending_gap() -> Option<Gap> {
    pending_in(&directory())
}
pub fn dismiss(gap: &Gap) -> io::Result<()> {
    write(&directory().join("recorder-gap-dismissed.json"), gap)
}

pub(super) struct Recorder {
    record: Record,
    path: std::path::PathBuf,
    last_tick: std::time::Instant,
    write_failed: bool,
}
impl Recorder {
    pub(super) fn start() -> Self {
        let path = directory().join("recorder-heartbeat.json");
        let previous: Option<Record> = read(&path);
        let previous_gap = previous.as_ref().and_then(|record| record.gap.clone());
        let record = Record::restart(previous, chrono::Utc::now().timestamp());
        if record.gap != previous_gap
            && let Some(gap) = &record.gap
        {
            super::log(&format!("recorder gap from={} to={}", gap.from, gap.to));
        }
        super::log(&format!("recorder started at={}", record.started));
        let mut recorder = Self {
            record,
            path,
            last_tick: std::time::Instant::now(),
            write_failed: false,
        };
        recorder.save();
        recorder
    }
    fn save(&mut self) {
        let failed = write(&self.path, &self.record).is_err();
        if failed && !self.write_failed {
            super::log("recorder heartbeat write failed");
        }
        self.write_failed = failed;
    }
    pub(super) fn tick(&mut self) {
        if self.last_tick.elapsed() >= std::time::Duration::from_secs(30) {
            self.record.last_seen = chrono::Utc::now().timestamp();
            self.last_tick = std::time::Instant::now();
            self.save();
        }
    }
    pub(super) fn stop(&mut self) {
        let now = chrono::Utc::now().timestamp();
        self.record.last_seen = now;
        self.record.stopped = Some(now);
        self.save();
        super::log(&format!("recorder stopped at={now}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_uses_stop_or_last_heartbeat_and_requires_more_than_two_minutes() {
        for stopped in [None, Some(110)] {
            let from = stopped.unwrap_or(100);
            for seconds in [120, 121] {
                let previous = Record {
                    started: 90,
                    last_seen: 100,
                    stopped,
                    gap: None,
                };
                let restarted = Record::restart(Some(previous), from + seconds);
                assert_eq!(
                    restarted.gap,
                    (seconds > 120).then_some(Gap {
                        from,
                        to: from + seconds
                    })
                );
            }
        }
        assert!(Record::restart(None, 100).gap.is_none());
        let future = Record {
            started: 90,
            last_seen: 200,
            stopped: None,
            gap: None,
        };
        assert!(Record::restart(Some(future), 100).gap.is_none());
    }
    #[test]
    fn dismissal_survives_restart_and_does_not_hide_a_new_gap() {
        let root = tempfile::tempdir().unwrap();
        let record = Record::restart(
            Some(Record {
                started: 10,
                last_seen: 20,
                stopped: None,
                gap: None,
            }),
            200,
        );
        let gap = record.gap.clone().unwrap();
        let path = root.path().join("recorder-heartbeat.json");
        write(&path, &record).unwrap();
        assert_eq!(pending_in(root.path()), Some(gap.clone()));
        write(&root.path().join("recorder-gap-dismissed.json"), &gap).unwrap();
        write(&path, &Record::restart(Some(record), 210)).unwrap();
        assert!(pending_in(root.path()).is_none());
        let previous = read(&path);
        write(&path, &Record::restart(previous, 400)).unwrap();
        assert_eq!(pending_in(root.path()), Some(Gap { from: 210, to: 400 }));
    }
}
