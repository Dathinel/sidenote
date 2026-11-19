use super::{Upload, remote_error};
use crate::core::speech_host::{
    Request,
    journal::{Change, Journal},
    request,
};
use serde_json::Value;
use std::{collections::BTreeSet, io};

const FRAME_LIMIT: usize = 128 << 10;

impl Upload<'_> {
    pub(super) fn consume(
        &self,
        path: &str,
        piece_path: &str,
        id: &str,
        journal: &mut Journal,
        latest: u64,
    ) -> io::Result<()> {
        if latest < journal.cursor() {
            return self.snapshot(piece_path, id, journal, None, true);
        }
        if !self.still_allowed() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut stream = Stream {
            id,
            bytes: Vec::new(),
            received: BTreeSet::new(),
            terminal: None,
            failed: false,
        };
        let headers = format!(
            "Accept: text/event-stream\r\nLast-Event-ID: {}\r\n",
            journal.cursor()
        );
        let events = format!("{path}/events");
        let _ = request(
            &self.host.endpoint,
            &self.host.token,
            Request {
                method: "GET",
                path: &events,
                headers: &headers,
                body: &[],
            },
            |_, data| stream.feed(data, journal).is_ok() && !journal.done() && !stream.failed,
        );
        if !journal.done() {
            // Timeouts, malformed frames and disconnects all leave the same recovery path.
            self.snapshot(
                piece_path,
                id,
                journal,
                stream.terminal.map(|(_, count, _)| count),
                false,
            )?;
        }
        Ok(())
    }

    fn snapshot(
        &self,
        path: &str,
        id: &str,
        journal: &mut Journal,
        expected: Option<usize>,
        reset: bool,
    ) -> io::Result<()> {
        let (status, data) = self.send(path, "GET", "", &[])?;
        if status == 404 {
            return journal.record(vec![Change::Ack(false), Change::Cursor(0)]);
        }
        if status != 200 {
            return Err(remote_error());
        }
        let ranges = data["ranges"].as_array().ok_or_else(remote_error)?;
        let failures = data["failed"].as_array().ok_or_else(remote_error)?;
        let mut seen = BTreeSet::new();
        let mut changes = Vec::new();
        for value in ranges {
            if value["piece"] != id {
                return Err(remote_error());
            }
            let key = journal.range(value)?;
            if seen.insert(key.clone()) && !journal.ranges().contains_key(&key) {
                changes.push(Change::Range(value.clone()));
            }
        }
        if matches!(data["state"].as_str(), Some("done" | "failed"))
            && seen.len() == journal.ranges().len() + changes.len()
            && expected.is_none_or(|count| count == seen.len())
        {
            changes.push(Change::Done(failures.clone()));
        }
        if reset {
            changes.push(Change::Cursor(0));
        }
        if data["state"] == "lost" {
            changes.push(Change::Lost);
        }
        journal.record(changes)
    }
}

struct Stream<'a> {
    id: &'a str,
    bytes: Vec<u8>,
    received: BTreeSet<u64>,
    terminal: Option<(u64, usize, Vec<Value>)>,
    failed: bool,
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
        let seq: u64 = lines
            .next()
            .and_then(|line| line.strip_prefix("id: "))
            .and_then(|n| n.parse().ok())
            .filter(|seq| *seq > 0)
            .ok_or_else(remote_error)?;
        let event = lines
            .next()
            .and_then(|line| line.strip_prefix("event: "))
            .ok_or_else(remote_error)?;
        let value: Value = serde_json::from_str(
            lines
                .next()
                .and_then(|line| line.strip_prefix("data: "))
                .ok_or_else(remote_error)?,
        )?;
        let cursor = journal.cursor();
        if seq <= cursor || !self.received.insert(seq) {
            return Ok(());
        }
        if self.received.len() > 4096 {
            return Err(remote_error());
        }
        let mut changes = Vec::new();
        let count = journal.ranges().len();
        if value["piece"] == self.id {
            match event {
                "range.result" => {
                    let key = journal.range(&value)?;
                    if !journal.ranges().contains_key(&key) {
                        changes.push(Change::Range(value));
                    }
                }
                "piece.done" => {
                    let ok = value["ranges_ok"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(remote_error)?;
                    let failures = value["failed"].as_array().ok_or_else(remote_error)?;
                    if value["ranges_total"].as_u64() != Some(ok as u64 + failures.len() as u64) {
                        return Err(remote_error());
                    }
                    self.terminal = Some((seq, ok, failures.clone()));
                }
                "piece.failed" => self.failed = true,
                _ => {}
            }
        }
        let mut next = cursor;
        while next
            .checked_add(1)
            .is_some_and(|seq| self.received.remove(&seq))
        {
            next += 1;
        }
        if let Some((seq, wanted, failures)) = &self.terminal
            && next >= *seq
            && *wanted == count + changes.len()
            && !journal.done()
        {
            changes.push(Change::Done(failures.clone()));
        }
        if next != cursor {
            changes.push(Change::Cursor(next));
        }
        journal.record(changes)
    }
}
