use super::super::hex_http::Service;
use super::*;

pub(super) struct RunHex {
    hex: HexSpeech,
    service: Option<Service>,
    receipt: PathBuf,
    state: Value,
}
impl RunHex {
    pub(super) fn new(hex: HexSpeech, work: &Path) -> Self {
        Self {
            hex,
            service: None,
            receipt: work.join("english-transport.json"),
            state: json!({"service_starts":0,"http_pieces":0,"command_pieces":0,
                "fallback_reason":null,"service_reaped":null,"service_start_seconds":0.0}),
        }
    }
    fn save(&self) -> std::result::Result<(), HexFailure> {
        store_io::write(&self.receipt, &self.state).map_err(|_| HexFailure::Failed)
    }
    fn stop_service(&mut self) {
        if let Some(mut service) = self.service.take() {
            self.state["service_reaped"] = service.close().into();
        }
    }
    fn fallback(&mut self, reason: &str) {
        self.stop_service();
        self.state["fallback_reason"] = reason.into();
    }
}
impl HexCommand for RunHex {
    fn finish(&mut self) -> std::result::Result<(), HexFailure> {
        self.stop_service();
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
            return self.hex.call(args, timeout);
        }
        let deadline = Instant::now() + timeout;
        if self.state["service_starts"] == 0 {
            self.state["service_starts"] = 1.into();
            self.save()?;
            // Preserve time for the command if startup hangs or HTTP exhausts its allowance.
            let started = Instant::now();
            let service = Service::start(&self.hex, timeout / 4);
            self.state["service_start_seconds"] = started.elapsed().as_secs_f64().into();
            match service {
                Ok(service) => self.service = Some(service),
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
            match service.call(Path::new(args[2]), model, remaining()? / 2) {
                Ok(text) => {
                    self.state["http_pieces"] =
                        (self.state["http_pieces"].as_u64().unwrap() + 1).into();
                    self.save()?;
                    return if args.contains(&"--json") {
                        Ok(json!({"transcript":text,"model":model}).to_string())
                    } else {
                        Ok(text)
                    };
                }
                Err(reason) => self.fallback(reason),
            }
        }
        // Retry this failed piece, then keep every later piece on the top-level command.
        self.state["command_pieces"] = (self.state["command_pieces"].as_u64().unwrap() + 1).into();
        self.save()?;
        let mut fallback = vec!["transcribe"];
        fallback.extend_from_slice(&args[2..]);
        if !fallback.contains(&"--json") {
            fallback.push("--json");
        }
        let output = self.hex.call(&fallback, remaining()?)?;
        if args.contains(&"--json") {
            return Ok(output);
        }
        let value: Value = serde_json::from_str(&output).map_err(|_| HexFailure::Failed)?;
        value["transcript"]
            .as_str()
            .or_else(|| value["text"].as_str())
            .map(str::to_owned)
            .ok_or(HexFailure::Failed)
    }
}
impl Drop for RunHex {
    fn drop(&mut self) {
        self.stop_service();
        let _ = self.save();
    }
}
pub(super) fn bridge(hex: HexSpeech, work: &Path, timeout: Duration) -> English<RunHex> {
    English(HexBridge::new(RunHex::new(hex, work)).with_timeout(timeout))
}
