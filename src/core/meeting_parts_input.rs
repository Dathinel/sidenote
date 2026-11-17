use super::*;
impl Meeting {
    /// Snapshot all finished parts, retaining the previous notes until replacement succeeds.
    pub fn prepare_notes(&mut self, root: &Path, hex: &Path) -> io::Result<PathBuf> {
        if self.parts.is_empty() {
            return Err(store_io::invalid("No recording part yet"));
        }
        self.refresh(hex)?;
        let folder = inputs(root).join(&self.id);
        let mut transcript = String::new();
        let mut offset = 0u64;
        for part in &self.parts {
            let input = hex.join(&part.id);
            let manifest: serde_json::Value = serde_json::from_slice(&file_reads::read(
                &input.join("manifest.json"),
                1024 * 1024,
            )?)?;
            if manifest["status"] != "complete" {
                return Err(store_io::invalid("Wait for Hex to finish the transcript"));
            }
            let raw = file_reads::text(
                &input.join("transcript.ndjson"),
                super::super::live::ROLLING_MAX_BYTES,
            )?;
            for line in raw.lines().filter(|l| !l.trim().is_empty()) {
                let mut row: serde_json::Value = serde_json::from_str(line)?;
                for field in ["start_ms", "end_ms"] {
                    if let Some(ms) = row[field].as_f64() {
                        row[field] = (ms + offset as f64).into();
                    }
                }
                transcript.push_str(&serde_json::to_string(&row)?);
                transcript.push('\n');
            }
            offset = offset.saturating_add(part.duration_ms);
        }
        if transcript.len() as u64 > super::super::live::ROLLING_MAX_BYTES {
            return Err(store_io::invalid("Meeting transcript is too large"));
        }
        let manifest = serde_json::json!({"schema_version":1,"status":"complete",
            "created_at_ms":self.parts[0].started_ms,"duration_ms":self.elapsed_ms(),
            "ended_at_ms": self.parts.last().map(|p| p.started_ms.saturating_add(p.duration_ms as i64))});
        store_io::atomic_write(&folder.join("transcript.ndjson"), transcript.as_bytes())?;
        store_io::write(&folder.join("manifest.json"), &manifest)?;
        store_io::write(&folder.join("sidenote-parts.json"), &self.parts)?;
        store_io::atomic_write(
            &folder.join("english-transcript.md"),
            super::super::notes::render_joined_transcript(&transcript).as_bytes(),
        )?;
        self.phase = Phase::Ended;
        self.requested_parts = self.parts.len();
        self.save(root)?;
        Ok(folder)
    }
    pub fn live_lines(&self, hex: &Path) -> Vec<super::super::live::LiveLine> {
        let mut lines = Vec::new();
        let mut offset = 0;
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                lines.push(super::super::live::LiveLine {
                    seconds: offset / 1000,
                    you: false,
                    text: "Recording resumed".into(),
                });
            }
            let folder = hex.join(&part.id);
            let mut part_lines = match super::super::live::read_rolling(&folder, None) {
                Ok(super::super::live::Read::Lines(lines, _)) => lines,
                _ => Vec::new(),
            };
            if part_lines.is_empty() {
                if let Ok(raw) = file_reads::text(
                    &folder.join("transcript.ndjson"),
                    super::super::live::ROLLING_MAX_BYTES,
                ) {
                    for row in raw
                        .lines()
                        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                    {
                        if let (Some(ms), Some(text)) =
                            (row["start_ms"].as_f64(), row["text"].as_str())
                        {
                            part_lines.push(super::super::live::LiveLine {
                                seconds: (ms.max(0.0) / 1000.0) as u64,
                                you: row["source"] == "microphone",
                                text: text.into(),
                            });
                        }
                    }
                }
            }
            for line in &mut part_lines {
                line.seconds += offset / 1000;
            }
            lines.extend(part_lines);
            offset = offset.saturating_add(part.duration_ms);
        }
        lines
    }
}
