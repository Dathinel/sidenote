//! Private headless proof control. Invitations and safety phrases only cross DPAPI files.
use super::*;
use sidenote_sync::{
    identity::{protect, unprotect},
    pairing::SecretCode,
};

pub(super) struct Driver {
    root: PathBuf,
    job: Option<pairing::Pairing>,
    events: Option<mpsc::Receiver<pairing::Update>>,
    phase: &'static str,
}
impl Driver {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            job: None,
            events: None,
            phase: "idle",
        }
    }
    fn cancel(&mut self) {
        self.job.take();
        self.events = None;
        self.phase = "idle";
    }
    fn poll(&mut self) -> io::Result<()> {
        let updates: Vec<_> = self
            .events
            .as_ref()
            .map(|events| events.try_iter().collect())
            .unwrap_or_default();
        for update in updates {
            match update {
                pairing::Update::Code { code, .. } => {
                    let protected = protect(code.as_bytes()).map_err(failed)?;
                    store_io::atomic_write(
                        &self.root.join("local/relay/test-code.dpapi"),
                        &protected,
                    )?;
                    self.phase = "code";
                }
                pairing::Update::Confirm {
                    local_name,
                    peer_name,
                    phrase,
                } => {
                    let confirmation = SecretCode::new(serde_json::to_string(
                        &serde_json::json!({"local":local_name,"peer":peer_name,"phrase":phrase}),
                    )?);
                    let protected = protect(confirmation.as_bytes()).map_err(failed)?;
                    store_io::atomic_write(
                        &self.root.join("local/relay/test-confirm.dpapi"),
                        &protected,
                    )?;
                    self.phase = "confirm";
                }
                pairing::Update::Ready => {
                    self.job.take();
                    self.phase = "paired";
                }
                pairing::Update::Failed => {
                    self.job.take();
                    self.phase = "failed";
                }
            }
        }
        Ok(())
    }
    pub fn process(&mut self, command: &Value) -> io::Result<Value> {
        self.poll()?;
        let root = &self.root;
        match command["op"].as_str().unwrap_or_default() {
            "host" | "join" => {
                let code = if command["op"] == "join" {
                    let path = PathBuf::from(
                        command["from"]
                            .as_str()
                            .ok_or_else(|| failed("missing proof path"))?,
                    )
                    .join("local/relay/test-code.dpapi");
                    if !super::super::is_proof_path(&path, "sidenote-relay-preview") {
                        return Err(failed("invalid proof path"));
                    }
                    let encrypted = store_io::read_bounded(&path, 4096)?;
                    let mut bytes = unprotect(&encrypted).map_err(failed)?;
                    if command["wrong"] == true
                        && let Some(last) = bytes.last_mut()
                    {
                        *last = if *last == b'9' { b'0' } else { *last + 1 };
                    }
                    Some(SecretCode::new(
                        String::from_utf8(bytes.to_vec()).map_err(failed)?,
                    ))
                } else {
                    None
                };
                self.cancel();
                let (job, events) = pairing::start(&self.root, code)?;
                self.job = Some(job);
                self.events = Some(events);
                self.phase = "starting";
                Ok(Value::Null)
            }
            "status" => {
                let peers = pairing::devices(root)?;
                Ok(
                    serde_json::json!({"phase":self.phase,"devices":peers.iter().map(|p|&p.name).collect::<Vec<_>>() }),
                )
            }
            "confirm" => {
                self.job
                    .as_ref()
                    .ok_or_else(|| failed("no pairing"))?
                    .confirm(command["accept"] == true);
                self.phase = "waiting";
                Ok(Value::Null)
            }
            "cancel" => {
                self.cancel();
                Ok(Value::Null)
            }
            "conflicts" => Ok(serde_json::to_value(
                get(root)?
                    .ok_or_else(|| failed("preview off"))?
                    .conflicts()?,
            )?),
            "resolutions" => Ok(serde_json::to_value(
                get(root)?
                    .ok_or_else(|| failed("preview off"))?
                    .resolutions()?,
            )?),
            "resolve" => {
                let preview = get(root)?.ok_or_else(|| failed("preview off"))?;
                let conflicts = preview.conflicts()?;
                let conflict = conflicts
                    .iter()
                    .find(|c| {
                        Some(c.task_id.as_str()) == command["id"].as_str()
                            && Some(c.field.as_str()) == command["field"].as_str()
                    })
                    .ok_or_else(|| failed("no conflict"))?;
                let choice = conflict
                    .choices
                    .get(command["choice"].as_u64().unwrap_or(u64::MAX) as usize)
                    .ok_or_else(|| failed("no choice"))?;
                preview.resolve(&conflict.task_id, &conflict.field, choice)?;
                Ok(Value::Null)
            }
            "disable" | "enable" => {
                let enabled = command["op"] == "enable";
                self.cancel();
                let mut settings = Settings::load(&self.root)?;
                settings.enabled = enabled;
                settings.save(&self.root)?;
                if enabled {
                    let _ = get(&self.root)?;
                }
                self.phase = if enabled { "idle" } else { "disabled" };
                Ok(Value::Null)
            }
            "remove" => {
                let peers = pairing::devices(root)?;
                let targets: Vec<_> = peers
                    .iter()
                    .filter(|p| !p.local && Some(p.name.as_str()) == command["name"].as_str())
                    .collect();
                if targets.len() != 1 {
                    return Err(failed("ambiguous device"));
                }
                get(root)?
                    .ok_or_else(|| failed("preview off"))?
                    .remove_device(&targets[0].token)?;
                Ok(Value::Null)
            }
            _ => Err(failed("unknown pairing operation")),
        }
    }
}
