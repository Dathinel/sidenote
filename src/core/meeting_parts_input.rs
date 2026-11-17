use super::*;
impl Meeting {
    /// Snapshot all finished parts, retaining the previous notes until replacement succeeds.
    pub fn prepare_notes(&mut self, root: &Path, hex: &Path) -> io::Result<PathBuf> {
        if self.parts.is_empty() {
            return Err(store_io::invalid("No recording part yet"));
        }
        if super::super::notes::queue::active(root,&self.id)? {
            return Err(io::Error::new(io::ErrorKind::WouldBlock,"notes_busy"));
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
            "notes_requested_at_ms": chrono::Utc::now().timestamp_millis(),
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
    pub fn view_text(&self,hex:&Path) -> io::Result<String> {
        let mut out=String::new(); let mut offset=0u64;
        for (i,part) in self.parts.iter().enumerate() {
            let text=part_text(&hex.join(&part.id),self.phase==Phase::Recording && i+1==self.parts.len())?;
            out.push_str(&super::super::notes::shift_transcript(&text,offset)?);
            offset=offset.saturating_add(part.duration_ms);
            if out.len() as u64>super::super::live::ROLLING_MAX_BYTES { return Err(store_io::invalid("Meeting transcript is too large")); }
        }
        Ok(out)
    }
    pub fn live_lines(&self,hex:&Path) -> Vec<super::super::live::LiveLine> {
        let mut out=Vec::new(); let mut offset=0u64;
        for (i,part) in self.parts.iter().enumerate() {
            if i>0 { out.push(super::super::live::LiveLine {seconds:offset/1000,you:false,text:"Recording resumed".into()}); }
            if let Ok(text)=part_text(&hex.join(&part.id),self.phase==Phase::Recording && i+1==self.parts.len()) {
                out.extend(super::super::parse_transcript(&text).into_iter().map(|line|super::super::live::LiveLine {
                    seconds:line.seconds.unwrap_or(0).saturating_add(offset/1000),you:line.who.eq_ignore_ascii_case("You"),text:line.text }));
            }
            offset=offset.saturating_add(part.duration_ms);
        }
        out
    }
}
fn part_text(folder:&Path,active:bool) -> io::Result<String> {
    let read=|name:&str| match file_reads::text(&folder.join(name),super::super::live::ROLLING_MAX_BYTES) {
        Ok(text)=>Ok(text),Err(e) if e.kind()==io::ErrorKind::NotFound=>Ok(String::new()),Err(e)=>Err(e)
    };
    if !active { let english=read("english-transcript.md")?; if !english.trim().is_empty() {return Ok(english);} }
    let mut raw=if active {read("transcript.rolling.ndjson")?} else {read("transcript.ndjson")?};
    if raw.trim().is_empty() { raw=read(if active {"transcript.ndjson"} else {"transcript.rolling.ndjson"})?; }
    Ok(super::super::notes::render_joined_transcript(&raw))
}
