use super::super::hex_http::Service;
use super::*;

pub(super) struct RunHex {
    hex: HexSpeech,
    service: Option<Service>,
    receipt: PathBuf,
    state: Value,
    current: Option<HexRequest>,
}
impl RunHex {
    pub(super) fn new(hex: HexSpeech, work: &Path) -> Self {
        Self {
            hex,
            service: None,
            receipt: work.join("english-transport.json"),
            current: None,
            state: json!({"service_starts":0,"http_pieces":0,"command_pieces":0,
                "ranges":{},"cleanup_reason":null,"fallback_reason":null,"service_reaped":null,"service_start_seconds":0.0,"service_close_seconds":0.0}),
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
            self.state["cleanup_reason"] = if graceful { "english-batch-finished" } else { "helper-aborted" }.into();
        }
    }
    fn fallback(&mut self, reason: &str) {
        self.stop_service(false);
        self.state["fallback_reason"] = reason.into();
        self.state["cleanup_reason"] = reason.into();
    }
    fn record(&mut self, transport: &str, reply: Value) {
        if let Some(r) = &self.current {
            self.state["ranges"][&r.range_id] = json!({"transport":transport,
                "model":r.model_id,"language":r.language,"reply":reply,
                "fallback":if transport == "failed" { json!("captured-hex-baseline") } else { Value::Null },
                "failure_reason":self.state["fallback_reason"]});
        }
    }
}
impl HexCommand for RunHex {
    fn begin(&mut self, request: &HexRequest) {
        self.current = Some(request.clone());
    }
    fn finish(&mut self) -> std::result::Result<(), HexFailure> {
        self.stop_service(true);
        let reason = self.state["cleanup_reason"].clone();
        let reaped = self.state["service_reaped"].clone();
        for range in self.state["ranges"].as_object_mut().unwrap().values_mut() {
            range["cleanup_reason"] = reason.clone();
            range["service_reaped"] = reaped.clone();
        }
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
                let idle = result.as_ref().ok().and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .is_some_and(|v| v["meeting"]["state"] == "idle"
                        && v["capture"]["state"] == "idle" && v["capture"]["pending_jobs"] == 0
                        && v["model"]["preparing"].is_null());
                if !idle { self.stop_service(false); }
            }
            return result;
        }
        let deadline = Instant::now() + timeout;
        if self.state["service_starts"] == 0 {
            self.state["service_starts"] = 1.into();
            self.save()?;
            // Bound startup within the existing range deadline.
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
