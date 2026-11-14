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
    failure: Option<(&'static str, String)>,
    attempt: u64,
    merge: Option<super::super::project_merge::MergePreview>,
}
impl Driver {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            job: None,
            events: None,
            phase: "idle",
            failure: None,
            attempt: 0,
            merge: None,
        }
    }
    fn cancel(&mut self) {
        self.job.take();
        self.events = None;
        self.phase = "idle";
        self.failure = None;
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
                    attempt,
                    local_name,
                    peer_name,
                    phrase,
                } => {
                    self.attempt = attempt;
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
                pairing::Update::TimedOut => {
                    self.job.take();
                    self.phase = "timed-out";
                }
                pairing::Update::Unconfirmed => {
                    self.job.take();
                    self.phase = "unconfirmed";
                }
                pairing::Update::Failed { stage, category } => {
                    self.failure = Some((stage, category));
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
            "shared-state" => {
                let tasks = super::super::TaskStore::new(root.clone());
                let config = super::super::Config::load_in(root)?;
                let prefs = super::super::UiPrefs::load(root);
                Ok(
                    serde_json::json!({"overrides":tasks.load_overrides()?,"projects":tasks.load_projects()?,
                    "teams":tasks.load_teams()?,"people":prefs.people_sides,"sidebar":prefs.sidebar_collapsed,
                    "name":config.notes.as_ref().map(|n|&n.your_name),"model":config.notes.as_ref().map(|n|&n.model),
                    "memory":super::super::memory::MemoryStore::new(root.clone()).load()?,
                    "checked":super::super::StateStore::new(root.clone()).load("100-2")?.checked}),
                )
            }
            "meeting-field" => {
                let field = command["field"].as_str().unwrap_or_default();
                let text = command["value"].as_str().unwrap_or_default().to_string();
                if !matches!(field, "owner" | "due" | "project" | "title") {
                    return Err(failed("field"));
                }
                super::super::TaskStore::new(root.clone()).update_override(
                    "100-2",
                    0,
                    "Synthetic generated task",
                    |o| match field {
                        "owner" => o.owner = Some(text),
                        "due" => o.due = Some(text),
                        "project" => o.project = Some(text),
                        _ => o.title = Some(text),
                    },
                )?;
                Ok(Value::Null)
            }
            "meeting-check" => {
                super::super::StateStore::new(root.clone()).set_tick_view(
                    "100-2",
                    command["task"]
                        .as_str()
                        .unwrap_or("Synthetic generated task"),
                    command["checked"] == true,
                )?;
                Ok(Value::Null)
            }
            "project-add" => {
                super::super::TaskStore::new(root.clone())
                    .add_project(command["name"].as_str().unwrap_or("Synthetic project"))?;
                Ok(Value::Null)
            }
            "area-create" => {
                let store = super::super::TaskStore::new(root.clone());
                let preview = store.preview_area(super::super::area_ops::Action::Create {
                    name: command["name"].as_str().unwrap_or("Synthetic area").into(),
                })?;
                store.apply_area(&preview)?;
                Ok(Value::Null)
            }
            "merge-open" => {
                self.merge = Some(
                    super::super::TaskStore::new(root.clone())
                        .preview_merge("Research", "Fieldwork")?,
                );
                Ok(Value::Null)
            }
            "merge-commit" => {
                let preview = self.merge.take().ok_or_else(|| failed("missing merge"))?;
                let value = super::super::TaskStore::new(root.clone()).merge_projects(&preview)?;
                Ok(Value::String(value.merges["Research"].id.clone()))
            }
            "merge-undo" => {
                super::super::TaskStore::new(root.clone())
                    .undo_project_merge("Research", command["id"].as_str().unwrap_or_default())?;
                Ok(Value::Null)
            }
            "team-member" | "team-alias" => {
                let store = super::super::TaskStore::new(root.clone());
                let original = store
                    .load_teams()?
                    .teams
                    .into_iter()
                    .next()
                    .ok_or_else(|| failed("team"))?;
                let mut changed = original.clone();
                if command["op"] == "team-member" {
                    changed.members.push(
                        command["name"]
                            .as_str()
                            .unwrap_or("Synthetic member")
                            .into(),
                    );
                } else {
                    changed.aliases.insert("A.".into(), "Alex".into());
                }
                store.edit_team(Some(&original), Some(&changed))?;
                Ok(Value::Null)
            }
            "people-side" => {
                let mut prefs = super::super::UiPrefs::load(root);
                prefs.people_sides.insert(
                    command["name"].as_str().unwrap_or("alex").into(),
                    serde_json::from_value(command["side"].clone())?,
                );
                prefs.save(root)?;
                Ok(Value::Null)
            }
            "local-settings" => {
                let mut prefs = super::super::UiPrefs::load(root);
                prefs.sidebar_collapsed = Some(true);
                prefs.save(root)?;
                let mut notes = super::super::Config::load_in(root)?
                    .notes
                    .unwrap_or_default();
                let baseline = notes.clone();
                notes.model = "synthetic-local-model".into();
                super::super::Config::save_notes(root, &baseline, &notes)?;
                Ok(
                    serde_json::json!({"sidebar":super::super::UiPrefs::load(root).sidebar_collapsed,"model":super::super::Config::load_in(root)?.notes.unwrap().model}),
                )
            }
            "profile-name" => {
                let mut notes = super::super::Config::load_in(root)?
                    .notes
                    .unwrap_or_default();
                let baseline = notes.clone();
                notes.your_name = command["name"].as_str().unwrap_or("Synthetic owner").into();
                super::super::Config::save_notes(root, &baseline, &notes)?;
                Ok(Value::Null)
            }
            "memory-edit" => {
                super::super::memory::MemoryStore::new(root.clone()).edit_record(
                    "synthetic-memory",
                    command["text"]
                        .as_str()
                        .unwrap_or("Synthetic edited memory"),
                )?;
                Ok(Value::Null)
            }
            "memory-confirm" => {
                super::super::memory::MemoryStore::new(root.clone())
                    .confirm_record("synthetic-memory")?;
                Ok(Value::Null)
            }
            "memory-readable" => Ok(Value::Bool(
                super::super::memory::MemoryStore::new(root.clone())
                    .load()
                    .is_ok(),
            )),
            "proposal-decision" => {
                let store = super::super::memory::MemoryStore::new(root.clone());
                let id = store
                    .list_proposals()?
                    .first()
                    .ok_or_else(|| failed("proposal"))?
                    .proposal
                    .id
                    .clone();
                if command["accept"] == true {
                    store.accept_proposal(&id)?;
                } else {
                    store.reject_proposal(&id)?;
                }
                Ok(Value::Null)
            }
            "proposal-stage" => {
                let mut proposal: super::super::memory::MemoryProposal = serde_json::from_value(
                    serde_json::json!({"id":"","op":"add","target_id":null,"text":"Synthetic proposal awaiting owner review","record_type":"story","project":null,"people":[],"sources":[],"rationale":"Synthetic review"}),
                )?;
                proposal.id = super::super::memory::proposal_id(&proposal);
                super::super::memory::MemoryStore::new(root.clone())
                    .stage_agent_proposal(&proposal)?;
                Ok(Value::Null)
            }
            "binary-version" => Ok(
                serde_json::json!({"capabilities":operations::build_capabilities(),"version":env!("CARGO_PKG_VERSION"),"debug":cfg!(debug_assertions)}),
            ),
            "check-relay" => Ok(serde_json::to_value(operations::check_now(root)?)?),
            "relay-state" => Ok(serde_json::to_value(operations::status(root)?)?),
            "folder-note" => {
                super::super::StateStore::new(root.clone()).update("100-2", |s| {
                    s.messages.push(super::super::Message {
                        role: "user".into(),
                        text: "Synthetic folder note".into(),
                        ..Default::default()
                    })
                })?;
                Ok(Value::Null)
            }
            "status" => {
                let peers = pairing::devices(root)?;
                Ok(
                    serde_json::json!({"phase":self.phase,"stage":self.failure.as_ref().map(|f|f.0),"category":self.failure.as_ref().map(|f|&f.1),"device":super::super::DeviceSettings::read_in(root).map(|d|d.name).unwrap_or_else(|_| "unknown".into()),"devices":peers.iter().map(|p|&p.name).collect::<Vec<_>>() }),
                )
            }
            "confirm" => {
                self.job
                    .as_ref()
                    .ok_or_else(|| failed("no pairing"))?
                    .confirm(self.attempt, command["accept"] == true);
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
                preview.resolve_conflict(conflict, choice)?;
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
