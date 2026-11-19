use super::super::hex_http::Service;
use super::*;

pub(super) struct RunHex {
    hex: HexSpeech,
    service: Option<Service>,
    receipt: PathBuf,
    state: Value,
    current: Option<HexRequest>,
    current_ranges: std::collections::BTreeSet<String>,
}
impl RunHex {
    pub(super) fn new(hex: HexSpeech, work: &Path) -> Self {
        let previous: Value =
            store_io::read_bounded(&work.join("english-transport.json"), 16 << 20)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Value::Null);
        let mut ranges = serde_json::Map::new();
        if let Some(records) = previous["ranges"].as_object() {
            for (id, record) in records {
                if !matches!(record["transport"].as_str(), Some("http" | "failed")) {
                    continue;
                }
                let mut safe = json!({});
                for key in [
                    "transport",
                    "model",
                    "language",
                    "fallback",
                    "failure_reason",
                    "cleanup_reason",
                    "service_reaped",
                ] {
                    safe[key] = record[key].clone();
                }
                safe["reply"] = Value::Null;
                if record["reply"].is_object() {
                    safe["reply"] = json!({});
                    for key in ["http_status", "api_version", "service_pid", "body_sha256"] {
                        safe["reply"][key] = record["reply"][key].clone();
                    }
                }
                ranges.insert(id.clone(), safe);
            }
        }
        Self {
            hex,
            service: None,
            receipt: work.join("english-transport.json"),
            current: None,
            current_ranges: Default::default(),
            state: json!({"service_starts":0,"http_pieces":0,"command_pieces":0,
                "ranges":ranges,"cleanup_reason":null,"fallback_reason":null,"service_reaped":null,"service_start_seconds":0.0,"service_close_seconds":0.0}),
        }
    }
    fn save(&self) -> std::result::Result<(), HexFailure> {
        store_io::write(&self.receipt, &self.state).map_err(|_| HexFailure::Failed)
    }
    fn stop_service(&mut self, graceful: bool) {
        if let Some(mut service) = self.service.take() {
            let started = Instant::now();
            self.state["service_reaped"] = if graceful {
                service.close()
            } else {
                service.abort()
            }
            .into();
            self.state["service_close_seconds"] = started.elapsed().as_secs_f64().into();
            self.cleanup(if graceful {
                "english-batch-finished"
            } else {
                "helper-aborted"
            });
        }
    }
    fn cleanup(&mut self, reason: &str) {
        self.state["cleanup_reason"] = reason.into();
        let reaped = self.state["service_reaped"].clone();
        for id in &self.current_ranges {
            let range = &mut self.state["ranges"][id];
            range["cleanup_reason"] = reason.into();
            range["service_reaped"] = reaped.clone();
        }
    }
    fn fallback(&mut self, reason: &str) {
        self.stop_service(false);
        self.state["fallback_reason"] = reason.into();
        self.cleanup(reason);
    }
    fn record(&mut self, transport: &str, reply: Value) {
        if let Some(r) = &self.current {
            self.current_ranges.insert(r.range_id.clone());
            self.state["ranges"][&r.range_id] = json!({"transport":transport,
                "model":r.model_id,"language":r.language,"reply":reply,
                "fallback":if transport == "failed" { json!("captured-hex-baseline") } else { Value::Null },
                "failure_reason":self.state["fallback_reason"],"cleanup_reason":self.state["cleanup_reason"],
                "service_reaped":self.state["service_reaped"]});
        }
    }
}
impl HexCommand for RunHex {
    fn can_reuse(&self, request: &HexRequest) -> bool {
        let record = &self.state["ranges"][&request.range_id];
        record["transport"] == "http"
            && record["model"] == request.model_id
            && record["language"] == request.language
            && record["service_reaped"] == true
            && record["reply"]["http_status"] == 200
            && record["reply"]["api_version"] == "2"
            && record["reply"]["service_pid"]
                .as_u64()
                .is_some_and(|pid| pid > 0)
            && record["reply"]["body_sha256"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()))
    }
    fn begin(&mut self, request: &HexRequest) {
        self.current = Some(request.clone());
    }
    fn finish(&mut self) -> std::result::Result<(), HexFailure> {
        self.stop_service(true);
        self.save()?;
        if self.state["service_reaped"] == false {
            return Err(HexFailure::Failed);
        }
        Ok(())
    }
    fn call(
        &mut self,
        args: &[&str],
        timeout: Duration,
    ) -> std::result::Result<String, HexFailure> {
        if args.contains(&"--help")
            || args.first() != Some(&"model")
            || args.get(1) != Some(&"transcribe")
        {
            let result = self.hex.call(args, timeout);
            if args.first() == Some(&"status") {
                let idle = result
                    .as_ref()
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .is_some_and(|v| {
                        v["version"] == 4
                            && v["model"].get("preparing").is_some()
                            && v["meeting"]["state"] == "idle"
                            && v["capture"]["state"] == "idle"
                            && v["capture"]["pending_jobs"] == 0
                            && v["model"]["preparing"].is_null()
                    });
                if !idle {
                    self.fallback("owner-status-active-or-unknown");
                    self.record("failed", Value::Null);
                    self.save()?;
                }
            }
            return result;
        }
        let deadline = Instant::now() + timeout;
        if self.state["service_starts"] == 0 {
            self.state["service_starts"] = 1.into();
            self.save()?;
            let started = Instant::now();
            let service = Service::start(&self.hex, timeout / 4);
            self.state["service_start_seconds"] = started.elapsed().as_secs_f64().into();
            match service {
                Ok(service) => self.service = Some(service),
                Err("service-owner-active-or-unknown") => {
                    self.fallback("service-owner-active-or-unknown");
                    self.record("failed", Value::Null);
                    self.save()?;
                    return Err(HexFailure::Busy);
                }
                Err(reason) => self.fallback(reason),
            }
        }
        let remaining = || {
            deadline
                .checked_duration_since(Instant::now())
                .ok_or(HexFailure::Timeout)
        };
        if let Some(service) = &mut self.service {
            let model = args
                .windows(2)
                .find(|a| a[0] == "--model")
                .ok_or(HexFailure::InvalidRange)?[1];
            let outcome = service.call(&self.hex, Path::new(args[2]), model, remaining()? / 2);
            let identity = service.last_reply.clone();
            match outcome {
                Ok(reply) => {
                    self.state["http_pieces"] =
                        (self.state["http_pieces"].as_u64().unwrap() + 1).into();
                    self.record("http", reply.identity);
                    self.save()?;
                    return if args.contains(&"--json") {
                        Ok(json!({"transcript":reply.text,"model":model}).to_string())
                    } else {
                        Ok(reply.text)
                    };
                }
                Err(reason) if reason == "service-owner-active-or-unknown" => {
                    self.fallback(&reason);
                    self.record("failed", Value::Null);
                    self.save()?;
                    return Err(HexFailure::Busy);
                }
                Err(reason) => {
                    self.fallback(&reason);
                    self.record("failed", identity);
                    self.save()?;
                    return Err(HexFailure::HelperFailed);
                }
            }
        }
        self.record("failed", Value::Null);
        self.save()?;
        Err(HexFailure::HelperFailed)
    }
}
impl Drop for RunHex {
    fn drop(&mut self) {
        self.stop_service(false);
        let _ = self.save();
    }
}
pub(super) fn bridge(hex: HexSpeech, work: &Path, timeout: Duration) -> English<RunHex> {
    English(HexBridge::new(RunHex::new(hex, work)).with_timeout(timeout))
}
