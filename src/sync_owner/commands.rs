use super::*;
use serde_json::{Value, json};
use sidenote_sync::pairing::SecretCode;
use std::sync::mpsc;
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
enum Request {
    Status {},
    Host {},
    Join {
        code: String,
    },
    PairState {},
    Confirm {
        attempt: u64,
        phrase: String,
        accept: bool,
    },
    Cancel {},
    Inventory {
        cursor: Option<pages::Cursor>,
    },
    List {
        cursor: Option<pages::Cursor>,
    },
    Conflicts {
        cursor: Option<pages::Cursor>,
    },
    Choices {
        conflict: pages::ConflictRef,
        cursor: Option<pages::Cursor>,
    },
    ResolveChoice {
        conflict: pages::ConflictRef,
        choice: usize,
    },
    Resolve {
        conflict: relay::Conflict,
        chosen: relay::Choice,
    },
    Offline {},
    Online {},
}
pub(super) struct Pair {
    job: relay::pairing::Pairing,
    updates: mpsc::Receiver<relay::pairing::Update>,
    phase: &'static str,
    code: Option<SecretCode>,
    expires: Instant,
    attempt: u64,
    phrase: SecretCode,
}

impl Controller {
    fn conflict(&self, key: &pages::ConflictRef) -> io::Result<relay::Conflict> {
        let current = self
            .preview()?
            .conflicts()?
            .into_iter()
            .find(|c| c.domain == key.domain && c.task_id == key.task_id && c.field == key.field)
            .ok_or_else(invalid)?;
        if pages::revision(&current)? != key.revision {
            return Err(io::Error::other("SYNC_CONTROL_CURSOR_STALE"));
        }
        Ok(current)
    }
    pub(super) fn poll_pair(&mut self) {
        let Some(pair) = &mut self.pair else { return };
        for update in pair.updates.try_iter() {
            match update {
                relay::pairing::Update::Code { code, expires } => {
                    pair.code = Some(code);
                    pair.expires = expires;
                    pair.phase = "code";
                }
                relay::pairing::Update::Confirm {
                    attempt, phrase, ..
                } => {
                    pair.code = None;
                    pair.attempt = attempt;
                    pair.phrase = SecretCode::new(phrase);
                    pair.phase = "confirm";
                }
                terminal => {
                    pair.phase = match terminal {
                        relay::pairing::Update::Ready => "paired",
                        relay::pairing::Update::TimedOut => "timed-out",
                        relay::pairing::Update::Unconfirmed => "unconfirmed",
                        relay::pairing::Update::Failed => "failed",
                        _ => unreachable!(),
                    };
                    pair.code = None;
                    pair.phrase.clear();
                }
            }
        }
        if Instant::now() >= pair.expires && matches!(pair.phase, "starting" | "code" | "confirm") {
            pair.job.cancel();
            pair.code = None;
            pair.phrase.clear();
            pair.phase = "timed-out";
        }
    }
    fn preview(&self) -> io::Result<Arc<relay::Preview>> {
        relay::get(&self.root)?.ok_or_else(|| io::Error::other("SYNC_RELAY_DISABLED"))
    }
    fn begin(&mut self, code: Option<SecretCode>) -> io::Result<Value> {
        self.pair.take();
        let (job, updates) = relay::pairing::start(&self.root, code)?;
        self.pair = Some(Pair {
            job,
            updates,
            phase: "starting",
            code: None,
            expires: Instant::now() + Duration::from_secs(600),
            attempt: 0,
            phrase: SecretCode::new(String::new()),
        });
        Ok(json!({"phase":"starting"}))
    }
    fn process(&mut self, bytes: &[u8]) -> io::Result<Value> {
        let request: Request = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        self.poll_pair();
        match request {
            Request::Status {} => Ok(pages::status(&self.state)),
            Request::Inventory { cursor } => pages::page(&self.state.meetings, cursor),
            Request::Host {} => self.begin(None),
            Request::Join { code } => self.begin(Some(SecretCode::new(code))),
            Request::PairState {} => {
                let Some(pair) = &self.pair else {
                    return Ok(json!({"phase":"idle"}));
                };
                // Only this explicit private control response contains pairing material.
                Ok(json!({"phase":pair.phase,"code":pair.code.as_deref(),
                    "attempt":pair.attempt,"phrase":pair.phrase.as_str()}))
            }
            Request::Confirm {
                attempt,
                phrase,
                accept,
            } => {
                let pair = self.pair.as_mut().ok_or_else(invalid)?;
                if pair.phase != "confirm" || pair.attempt != attempt || *pair.phrase != phrase {
                    return Err(io::Error::other("SYNC_CONFIRMATION_MISMATCH"));
                }
                pair.job.confirm(attempt, accept);
                pair.phrase.clear();
                pair.phase = "confirming";
                Ok(Value::Null)
            }
            Request::Cancel {} => {
                self.pair.take();
                Ok(Value::Null)
            }
            Request::List { cursor } => pages::page(&self.preview()?.tasks()?, cursor),
            Request::Conflicts { cursor } => {
                let rows = self
                    .preview()?
                    .conflicts()?
                    .iter()
                    .map(pages::conflict_ref)
                    .collect::<io::Result<Vec<_>>>()?;
                pages::page(&rows, cursor)
            }
            Request::Choices { conflict, cursor } => {
                pages::page(&self.conflict(&conflict)?.choices, cursor)
            }
            Request::ResolveChoice { conflict, choice } => {
                let current = self.conflict(&conflict)?;
                let chosen = current.choices.get(choice).ok_or_else(invalid)?;
                self.preview()?.resolve_conflict(&current, chosen)?;
                Ok(Value::Null)
            }
            Request::Resolve { conflict, chosen } => {
                self.preview()?.resolve_conflict(&conflict, &chosen)?;
                Ok(Value::Null)
            }
            Request::Offline {} => {
                self.preview()?.online(false)?;
                Ok(Value::Null)
            }
            Request::Online {} => {
                self.preview()?.online(true)?;
                Ok(Value::Null)
            }
        }
    }
    fn reply(&mut self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        envelope(self.process(bytes))
    }
}

fn envelope(result: io::Result<Value>) -> io::Result<Vec<u8>> {
    let value = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":match error.to_string().as_str() {
            "SYNC_CONFIRMATION_MISMATCH" => "SYNC_CONFIRMATION_MISMATCH",
            "SYNC_CONTROL_CURSOR_STALE" => "SYNC_CONTROL_CURSOR_STALE",
            "SYNC_CONTROL_ITEM_TOO_LARGE" => "SYNC_CONTROL_ITEM_TOO_LARGE",
            _ => "SYNC_CONTROL_REJECTED"
        }}),
    };
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > MAX_REPLY {
        return Ok(br#"{"ok":false,"error":"SYNC_CONTROL_ITEM_TOO_LARGE"}"#.to_vec());
    }
    Ok(bytes)
}
pub(super) fn respond(control: &Mutex<Controller>, bytes: &[u8]) -> io::Result<Vec<u8>> {
    match serde_json::from_slice::<Request>(bytes) {
        Ok(Request::Status {}) => envelope(Ok(pages::status(&snapshot()))),
        Ok(Request::Inventory { cursor }) => envelope(pages::page(&snapshot().meetings, cursor)),
        _ => control.lock().map_err(|_| invalid())?.reply(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_fields_and_operations_are_rejected() {
        for bytes in [
            br#"{"op":"status","code":"synthetic"}"#.as_slice(),
            br#"{"op":"unknown"}"#,
        ] {
            assert!(serde_json::from_slice::<Request>(bytes).is_err());
        }
    }
}
