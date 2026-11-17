//! Opt-in daily checks use the service's replica and a durable retry window.
use crate::core::{self, store_io};
use chrono::{Local, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

const RETRY_MS: i64 = 15 * 60 * 1000;
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Schedule {
    enabled: bool,
    hour: u32,
    minute: u32,
    runner: String,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            hour: 8,
            minute: 30,
            runner: "Codex Sol".into(),
        }
    }
}
impl Schedule {
    fn load(root: &Path) -> io::Result<Self> {
        let bytes = match store_io::read_bounded(&root.join("service-schedule.json"), 4096) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let schedule: Self = serde_json::from_slice(&bytes)?;
        if schedule.hour > 23 || schedule.minute > 59 || schedule.runner.trim().is_empty() {
            return Err(store_io::invalid("INVALID_SERVICE_SCHEDULE"));
        }
        Ok(schedule)
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Attempt {
    day: Option<NaiveDate>,
    attempted_at_ms: i64,
    completed: bool,
}

pub(crate) fn tick() -> io::Result<()> {
    if crate::service::role() != crate::service::DeviceRole::Server {
        return Ok(());
    }
    let root = core::state_dir();
    let schedule = Schedule::load(&root)?;
    let now = Local::now();
    if !schedule.enabled
        || (now.hour(), now.minute()) < (schedule.hour, schedule.minute)
        || !core::Config::load()?.features.daily_check
        || !core::DeviceSettings::load()?.daily_check
    {
        return Ok(());
    }
    super::workers::start("daily-check", move || {
        let result = attempt(&root, now.date_naive(), now.timestamp_millis(), || {
            core::daily_check::run_daily_check(&schedule.runner, false, false)
                .map(|_| ())
                .map_err(|_| io::Error::other("SCHEDULED_CHECK_FAILED"))
        });
        if result.is_err() {
            let _ = crate::operations::record(&root, crate::operations::EventKind::WorkerChanged);
        }
    })
}

fn attempt(
    root: &Path,
    day: NaiveDate,
    now_ms: i64,
    run: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let _lock = match store_io::lock(&root.join("local/scheduled-check.lock")) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
        Err(error) => return Err(error),
    };
    let path = root.join("local/scheduled-check.json");
    let prior: Attempt = store_io::read(&path)?;
    let report = root.join("local/checks").join(format!("{day}.json"));
    if report.is_file()
        || (prior.day == Some(day)
            && (prior.completed || now_ms.saturating_sub(prior.attempted_at_ms) < RETRY_MS))
    {
        return Ok(());
    }
    let mut receipt = Attempt {
        day: Some(day),
        attempted_at_ms: now_ms,
        completed: false,
    };
    // A killed worker leaves an incomplete durable attempt and becomes eligible again.
    store_io::write(&path, &receipt)?;
    run()?;
    receipt.completed = report.is_file();
    store_io::write(&path, &receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_attempt_retries_after_backoff_and_published_report_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let day = NaiveDate::from_ymd_opt(2030, 1, 2).unwrap();
        assert!(!Schedule::load(root.path()).unwrap().enabled);
        assert!(
            attempt(root.path(), day, RETRY_MS, || Err(io::Error::other(
                "killed"
            )))
            .is_err()
        );
        attempt(root.path(), day, RETRY_MS + 1, || panic!("retry storm")).unwrap();
        attempt(root.path(), day, RETRY_MS * 2, || {
            std::fs::create_dir_all(root.path().join("local/checks"))?;
            std::fs::write(
                root.path().join("local/checks/2030-01-02.json"),
                b"synthetic report",
            )
        })
        .unwrap();
        attempt(root.path(), day, RETRY_MS * 3, || {
            panic!("duplicate report")
        })
        .unwrap();
        let saved: Attempt =
            store_io::read(&root.path().join("local/scheduled-check.json")).unwrap();
        assert!(saved.completed);
        let next = day.succ_opt().unwrap();
        let mut called = false;
        attempt(root.path(), next, RETRY_MS * 4, || {
            called = true;
            Ok(())
        })
        .unwrap();
        assert!(called);
    }
    #[test]
    fn invalid_schedule_and_corrupt_receipt_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        for bytes in [
            br#"{"enabled":true,"hour":24}"#.as_slice(),
            br#"{"enabled":true,"minute":60}"#,
            br#"{"enabled":true,"runner":" "}"#,
            br#"{"unknown":true}"#,
        ] {
            std::fs::write(root.path().join("service-schedule.json"), bytes).unwrap();
            assert!(Schedule::load(root.path()).is_err());
        }
        std::fs::create_dir_all(root.path().join("local")).unwrap();
        std::fs::write(root.path().join("local/scheduled-check.json"), b"{").unwrap();
        assert!(
            attempt(root.path(), Local::now().date_naive(), RETRY_MS, || panic!(
                "corrupt receipt"
            ))
            .is_err()
        );
    }
}
