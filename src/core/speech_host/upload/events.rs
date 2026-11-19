use super::{Upload, remote_error};
use crate::core::speech_host::{
    Request,
    journal::{Change, Journal},
    request,
};
use serde_json::Value;
use std::{collections::BTreeSet, io, time::Instant};

const FRAME_LIMIT: usize = 128 << 10;

impl Upload<'_> {
    pub(super) fn consume(
        &self,
        path: &str,
        piece_path: &str,
        id: &str,
        journal: &mut Journal,
        latest: u64,
        deadline: Instant,
    ) -> io::Result<()> {
        if latest < journal.cursor() {
            return self.snapshot(piece_path, id, journal, None, true, deadline);
        }
        if !self.still_allowed() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut stream = Stream {
            id,
            bytes: Vec::new(),
            expected: None,
            terminal: false,
        };
        let headers = format!(
            "Accept: text/event-stream\r\nLast-Event-ID: {}\r\n",
            journal.cursor()
        );
        let events = format!("{path}/events");
        let host = self.host()?;
        let _ = request(
            &host.endpoint,
            &host.token,
            Request {
                method: "GET",
                path: &events,
                headers: &headers,
                body: &[],
                deadline: Some(deadline),
            },
            |_, data| stream.feed(data, journal).is_ok() && !stream.terminal,
        );
        self.snapshot(piece_path, id, journal, stream.expected, false, deadline)
    }

    fn snapshot(
        &self,
        path: &str,
        id: &str,
        journal: &mut Journal,
        expected: Option<u64>,
        reset: bool,
        deadline: Instant,
    ) -> io::Result<()> {
        let (status, data) = self.send(path, "GET", "", &[], Some(deadline))?;
        if status == 404 && !journal.replacement() {
            return journal.record(vec![Change::Ack(false), Change::Cursor(0)]);
        }
        if status != 200 {
            return Err(remote_error());
        }
        let ranges = data["ranges"].as_array().ok_or_else(remote_error)?;
        let failures = data["failed"].as_array().ok_or_else(remote_error)?;
        let saved = journal.ranges();
        let mut seen = BTreeSet::new();
        let mut changes = Vec::new();
        for value in ranges {
            if value["piece"] != id {
                return Err(remote_error());
            }
            let key = journal.range(value).map_err(|_| remote_error())?;
            if seen.insert(key.clone()) && !saved.contains_key(&key) {
                changes.push(Change::Range(value.clone()));
            }
        }
        let state = data["state"].as_str();
        let complete = seen.len() == saved.len() + changes.len()
            && expected.is_none_or(|count| count == seen.len() as u64);
        match state {
            Some("done" | "failed")
                if journal.replacement()
                    && (!complete || state == Some("failed") || !failures.is_empty()) =>
            {
                changes.push(Change::HostFailed)
            }
            Some("failed") if complete => changes.push(Change::HostFailed),
            Some("done") if complete => changes.push(Change::Done(failures.clone())),
            Some("lost") if journal.replacement() => changes.push(Change::HostFailed),
            Some("lost") => changes.push(Change::Lost),
            _ => {}
        }
        if reset {
            changes.push(Change::Cursor(0));
        }
        journal.record(changes)
    }
}

struct Stream<'a> {
    id: &'a str,
    bytes: Vec<u8>,
    expected: Option<u64>,
    terminal: bool,
}

impl Stream<'_> {
    fn feed(&mut self, data: &[u8], journal: &mut Journal) -> io::Result<()> {
        self.bytes.extend_from_slice(data);
        while let Some(end) = self.bytes.windows(2).position(|w| w == b"\n\n") {
            if end > FRAME_LIMIT {
                return Err(remote_error());
            }
            let frame: Vec<_> = self.bytes.drain(..end + 2).collect();
            self.apply(&frame, journal)?;
        }
        if self.bytes.len() > FRAME_LIMIT {
            return Err(remote_error());
        }
        Ok(())
    }

    fn apply(&mut self, frame: &[u8], journal: &mut Journal) -> io::Result<()> {
        let frame = std::str::from_utf8(frame).map_err(|_| remote_error())?;
        if frame.starts_with(':') {
            return Ok(());
        }
        let mut lines = frame.lines();
        let mut field = |prefix| {
            lines
                .next()
                .and_then(|line| line.strip_prefix(prefix))
                .ok_or_else(remote_error)
        };
        let seq: u64 = field("id: ")?.parse().map_err(|_| remote_error())?;
        if seq == 0 {
            return Err(remote_error());
        }
        let event = field("event: ")?;
        let value: Value = serde_json::from_str(field("data: ")?)?;
        let cursor = journal.cursor();
        if seq <= cursor {
            return Ok(());
        }
        let mut changes = Vec::new();
        if value["piece"] == self.id {
            match event {
                "range.result" => {
                    let key = journal.range(&value)?;
                    if !journal.ranges().contains_key(&key) {
                        changes.push(Change::Range(value));
                    }
                }
                "piece.done" | "piece.failed" => {
                    self.expected = value["ranges_ok"].as_u64();
                    self.terminal = true;
                }
                _ => {}
            }
        }
        // Replaying a gap is cheap: applying a range twice keeps its first reading.
        if cursor.checked_add(1) == Some(seq) {
            changes.push(Change::Cursor(seq));
        }
        journal.record(changes)
    }
}
