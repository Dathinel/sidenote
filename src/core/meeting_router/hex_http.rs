//! An embedded endpoint belongs only to this run; never discover the owner's service.
use super::{HexSpeech, inputs::Snapshot};
use crate::platform::{JobProcess, ProcessJob};
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Read},
    os::windows::io::AsRawHandle,
    path::Path,
    process::Command,
    sync::{Arc, mpsc, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Endpoint {
    url: String,
    token: String,
    pid: u32,
    api_version: String,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
pub(super) struct Service {
    process: Option<JobProcess>,
    job: Arc<ProcessJob>,
    endpoint: Option<Endpoint>,
    pub(super) last_reply: serde_json::Value,
}
pub(super) struct Reply {
    pub text: String,
    pub identity: serde_json::Value,
}
impl Service {
    pub(super) fn start(hex: &HexSpeech, timeout: Duration) -> Result<Self, &'static str> {
        let job = Arc::new(ProcessJob::new().map_err(|_| "service-containment")?);
        let (output, writer) = std::io::pipe().map_err(|_| "service-pipe")?;
        let mut command = Command::new(&hex.executable);
        command
            .args(["service", "--embedded"])
            .envs(std::env::vars_os());
        // The process is placed in the kill-on-close job before its first instruction.
        let process = job
            .spawn_output(&command, Some(&writer))
            .map_err(|_| "service-spawn")?;
        drop(writer);
        let mut service = Self {
            process: Some(process),
            job,
            endpoint: None,
            last_reply: serde_json::Value::Null,
        };
        let watch = OwnerWatch::start(hex, &service.job);
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let mut line = Zeroizing::new(Vec::new());
            let result = BufReader::new(output)
                .take(16385)
                .read_until(b'\n', &mut line)
                .ok()
                .filter(|_| line.len() <= 16384)
                .and_then(|_| serde_json::from_slice::<Endpoint>(&line).ok());
            let _ = tx.send(result);
        });
        let endpoint = rx
            .recv_timeout(timeout.min(Duration::from_secs(30)))
            .ok()
            .flatten();
        if endpoint.is_none() {
            service.abort();
        }
        // Closing the job also closes descendant copies of stdout on a failed handshake.
        let _ = reader.join();
        if watch.finish() {
            service.abort();
            return Err("service-owner-active-or-unknown");
        }
        let endpoint = endpoint.ok_or("service-handshake")?;
        let url = url::Url::parse(&endpoint.url).map_err(|_| "service-endpoint")?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || endpoint.api_version != "2"
            || endpoint.token.is_empty()
            || !endpoint
                .token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            || !service.job.contains_pid(endpoint.pid).unwrap_or(false)
            || !owns_port(endpoint.pid, url.port().unwrap())
        {
            return Err("service-owner-or-endpoint");
        }
        service.endpoint = Some(endpoint);
        Ok(service)
    }
    pub(super) fn call(
        &mut self,
        hex: &HexSpeech,
        wav: &Path,
        model: &str,
        timeout: Duration,
    ) -> Result<Reply, String> {
        self.last_reply = serde_json::Value::Null;
        let endpoint = self.endpoint.as_ref().ok_or("service-closed")?;
        let mut url = url::Url::parse(&endpoint.url).map_err(|_| "service-endpoint")?;
        if !self.job.contains_pid(endpoint.pid).unwrap_or(false)
            || !owns_port(endpoint.pid, url.port().unwrap())
        {
            return Err("service-owner-changed".into());
        }
        url.set_path("/transcriptions");
        url.query_pairs_mut()
            .append_pair("model", model)
            .append_pair("language", "en");
        let audio = Snapshot::read_file(wav, 1 << 20).map_err(|_| "service-audio")?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build()
            .into();
        let mut authorization = ureq::http::HeaderValue::from_str(&Zeroizing::new(format!(
            "Bearer {}",
            endpoint.token
        )))
        .map_err(|_| "service-auth")?;
        authorization.set_sensitive(true);
        let watch = OwnerWatch::start(hex, &self.job);
        let response = agent
            .post(url.as_str())
            .header("Authorization", authorization)
            .header("Content-Type", "audio/wav")
            .send(&audio);
        // Keep observing the owner through body reads as well as model inference.
        let result = (|| {
        let mut response = response.map_err(|_| "service-request")?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(8 << 20)
            .read_to_string()
            .map_err(|_| "service-response")?;
        use sha2::{Digest, Sha256};
        self.last_reply = serde_json::json!({"http_status":status,"api_version":endpoint.api_version,
                "service_pid":endpoint.pid,"body_sha256":hex::encode(Sha256::digest(text.as_bytes()))});
        if status != 200 {
            return Err(format!("service-http-status-{status}"));
        }
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| "service-json")?;
        let transcript = value["transcript"].as_str().ok_or("service-transcript")?;
        if !self.job.contains_pid(endpoint.pid).unwrap_or(false)
            || !owns_port(endpoint.pid, url.port().unwrap())
        {
            return Err("service-owner-changed".into());
        }
        // Empty success retains the existing NoSpeech disposition without restarting inference.
        Ok(Reply {
            text: transcript.to_owned(),
            identity: self.last_reply.clone(),
        })
        })();
        if watch.finish() {
            self.abort();
            return Err("service-owner-active-or-unknown".into());
        }
        result
    }
    pub(super) fn close(&mut self) -> bool {
        self.stop(Duration::from_secs(3))
    }
    pub(super) fn abort(&mut self) -> bool {
        self.stop(Duration::ZERO)
    }
    fn stop(&mut self, grace: Duration) -> bool {
        if let Some(process) = self.process.take() {
            let handle = process.handle;
            drop(process.input);
            // Normal exits honor the SDK's stdin lease; faults and cancellation skip the grace.
            if !grace.is_zero() {
                let _ = JobProcess::wait_handle(&handle, grace.as_millis() as u32);
            }
            if self.job.terminate().is_err() {
                unsafe {
                    windows_sys::Win32::System::Threading::TerminateProcess(
                        handle.as_raw_handle(),
                        1,
                    );
                }
            }
            if !matches!(JobProcess::wait_handle(&handle, 5000), Ok(Some(_))) {
                return false;
            }
            let started = Instant::now();
            while !self.job.empty().unwrap_or(false) && started.elapsed() < Duration::from_secs(5) {
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.job.empty().unwrap_or(false)
    }
}
// The private model has no shared admission queue with the owner's app.
// Unknown status therefore releases the model just like known owner activity.
struct OwnerWatch {
    stop: mpsc::Sender<()>,
    active: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl OwnerWatch {
    fn start(hex: &HexSpeech, job: &Arc<ProcessJob>) -> Self {
        let hex = HexSpeech { executable: hex.executable.clone() };
        let job = Arc::clone(job);
        let active = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&active);
        let (stop, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut status = super::hex_bridge::HexBridge::new(hex)
                .with_timeout(Duration::from_millis(250));
            loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                    _ => break,
                }
                if status.probe_idle() != Ok(true) {
                    observed.store(true, Ordering::Release);
                    let _ = job.terminate();
                    break;
                }
            }
        });
        Self { stop, active, thread: Some(thread) }
    }
    fn finish(mut self) -> bool {
        self.join();
        self.active.load(Ordering::Acquire)
    }
    fn join(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for OwnerWatch {
    fn drop(&mut self) { self.join(); }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.abort();
    }
}
fn owns_port(pid: u32, port: u16) -> bool {
    use windows_sys::Win32::{NetworkManagement::IpHelper::*, Networking::WinSock::AF_INET};
    let mut size = 0;
    unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
    }
    if size == 0 || size > 16 << 20 {
        return false;
    }
    let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
    if unsafe {
        GetExtendedTcpTable(
            buffer.as_mut_ptr().cast(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    } != 0
    {
        return false;
    }
    let count = buffer[0] as usize;
    let rows = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().add(1).cast::<MIB_TCPROW_OWNER_PID>(),
            (buffer.len() - 1) * 4 / std::mem::size_of::<MIB_TCPROW_OWNER_PID>(),
        )
    };
    rows.iter().take(count).any(|row| {
        row.dwOwningPid == pid
            && row.dwLocalAddr == u32::from_ne_bytes([127, 0, 0, 1])
            && u16::from_be(row.dwLocalPort as u16) == port
    })
}
