use anyhow::{Context, Result, ensure};
use automerge::Automerge;
use serde_json::{Value, json};
use sidenote_sync::{
    client::{Device, Running, SyncClient},
    identity::Group,
    model,
    pairing::{self, PairHost},
    transport::{Frame, RelayUrl, Transport, WebSocketTransport},
};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

const DOMAINS: [&str; 5] = ["tasks", "memory", "projects", "diary", "meeting:fixture-1"];
const MARKER: &str = "SIDENOTE-PLAINTEXT-MUST-NEVER-REACH-RELAY";

fn event(name: &str) {
    println!("{}", json!({"event":name}));
}

async fn same(a: &Device, b: &Device) -> Result<()> {
    same_with_timeout(a, b, Duration::from_secs(30)).await
}

async fn same_with_timeout(a: &Device, b: &Device, deadline: Duration) -> Result<()> {
    tokio::time::timeout(deadline, async {
        loop {
            let mut equal = true;
            for domain in DOMAINS {
                if a.load(domain)?.get_heads() != b.load(domain)?.get_heads() {
                    equal = false;
                }
            }
            if equal {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("replicas did not converge")?
}

async fn fail_join(relay: &RelayUrl, code: &str, device: &Device) -> Result<()> {
    ensure!(
        pairing::join(relay, code, device, |_| async { true })
            .await
            .is_err(),
        "wrong code accepted"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    Ok(())
}

async fn pair(
    relay: &RelayUrl,
    a: Arc<Device>,
    b: Arc<Device>,
    existing: Option<Group>,
) -> Result<(Group, Group)> {
    let host = PairHost::open(relay, &a.identity).await?;
    let code = zeroize::Zeroizing::new(host.code().to_owned());
    let mut wrong = zeroize::Zeroizing::new(code.as_bytes().to_vec());
    wrong[7] = if wrong[7] == b'9' { b'0' } else { wrong[7] + 1 };
    let sas = Arc::new(Mutex::new(Vec::new()));
    let host_sas = sas.clone();
    let a_task = tokio::spawn(async move {
        host.accept(&a, existing, |s| {
            host_sas.lock().expect("SAS lock").push(s);
            async { true }
        })
        .await
    });
    fail_join(relay, std::str::from_utf8(&wrong)?, &b).await?;
    let guest_sas = sas.clone();
    let bg = pairing::join(relay, &code, &b, |s| {
        guest_sas.lock().expect("SAS lock").push(s);
        async { true }
    })
    .await?;
    let ag = tokio::time::timeout(Duration::from_secs(5), a_task).await???;
    ensure!(ag.group.room() == bg.room(), "group mismatch");
    {
        let sas = sas.lock().expect("SAS lock");
        ensure!(sas.len() == 2 && sas[0] == sas[1], "confirmation mismatch");
    }
    ensure!(
        pairing::join(relay, &code, &b, |_| async { true })
            .await
            .is_err(),
        "consumed invitation reused"
    );
    ensure!(
        ag.receipt == pairing::Receipt::Confirmed,
        "pairing receipt unknown"
    );
    Ok((ag.group, bg))
}

async fn three_failures(relay: &RelayUrl, a: Arc<Device>, b: Arc<Device>) -> Result<()> {
    let host = PairHost::open(relay, &a.identity).await?;
    let code = zeroize::Zeroizing::new(host.code().to_owned());
    let mut wrong = zeroize::Zeroizing::new(code.as_bytes().to_vec());
    wrong[7] = if wrong[7] == b'9' { b'0' } else { wrong[7] + 1 };
    let task = tokio::spawn(async move { host.accept(&a, None, |_| async { true }).await });
    for _ in 0..3 {
        fail_join(relay, std::str::from_utf8(&wrong)?, &b).await?;
    }
    ensure!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await??
            .is_err(),
        "room survived three failures"
    );
    ensure!(
        pairing::join(relay, &code, &b, |_| async { true })
            .await
            .is_err(),
        "closed invitation accepted correct code"
    );
    Ok(())
}

async fn declined_confirmation(relay: &RelayUrl, a: Arc<Device>, b: Arc<Device>) -> Result<()> {
    let host = PairHost::open(relay, &a.identity).await?;
    let code = zeroize::Zeroizing::new(host.code().to_owned());
    let task = tokio::spawn(async move { host.accept(&a, None, |_| async { false }).await });
    ensure!(
        pairing::join(relay, &code, &b, |_| async { true })
            .await
            .is_err(),
        "declined confirmation accepted"
    );
    task.abort();
    let _ = task.await;
    Ok(())
}

fn auth(identity: &sidenote_sync::identity::Identity, group: &Group, nonce: &str) -> Value {
    let enrollments:Vec<_>=group.enrollments.iter().map(|e|json!({"sponsor":hex::encode(e.sponsor),"member":hex::encode(e.member),"signature":hex::encode(&e.signature)})).collect();
    json!({"type":"auth","key":hex::encode(identity.public()),"signature":hex::encode(identity.sign(&sidenote_sync::identity::auth_bytes(&group.room(),"device",nonce))),"enrollments":enrollments})
}
async fn text(ws: &mut impl Transport) -> Result<Value> {
    match ws.recv().await? {
        Frame::Text(s) => Ok(serde_json::from_str(&s)?),
        _ => anyhow::bail!("expected control"),
    }
}
async fn rejected_auth(a: &Device, c: &Device, group: &Group) -> Result<()> {
    let endpoint = format!("ws://127.0.0.1:8787/sync/{}?role=device", group.room());
    let mut first = WebSocketTransport::connect(&endpoint).await?;
    let nonce = text(&mut first).await?;
    let replay = auth(
        &a.identity,
        group,
        nonce["nonce"].as_str().context("nonce")?,
    );
    let mut other = WebSocketTransport::connect(&endpoint).await?;
    let _ = text(&mut other).await?;
    other.send(Frame::Text(replay.to_string())).await?;
    ensure!(
        other.recv().await.is_err(),
        "replayed relay challenge accepted"
    );
    first.close().await?;
    let mut unknown = WebSocketTransport::connect(&endpoint).await?;
    let nonce = text(&mut unknown).await?;
    unknown
        .send(Frame::Text(
            auth(
                &c.identity,
                group,
                nonce["nonce"].as_str().context("nonce")?,
            )
            .to_string(),
        ))
        .await?;
    ensure!(
        unknown.recv().await.is_err(),
        "unregistered device accepted"
    );
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (mut raw, _) = tokio_tungstenite::connect_async(&endpoint).await?;
    let challenge = raw.next().await.context("missing raw challenge")??;
    let challenge: Value = serde_json::from_str(challenge.to_text()?)?;
    raw.send(Message::Text(
        auth(
            &a.identity,
            group,
            challenge["nonce"].as_str().context("nonce")?,
        )
        .to_string()
        .into(),
    ))
    .await?;
    ensure!(
        raw.next()
            .await
            .context("missing raw ready")??
            .to_text()?
            .contains("ready"),
        "raw auth rejected"
    );
    raw.send(Message::Binary(
        vec![0; sidenote_sync::transport::MAX_FRAME + 1].into(),
    ))
    .await?;
    ensure!(
        matches!(
            raw.next().await,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None
        ),
        "oversize accepted"
    );
    // The normal transport paces at 200 frames/s and cannot exceed the relay's 600-frame quota.
    let (mut limited, _) =
        tokio_tungstenite::connect_async(format!("{endpoint}&__proof_rate=1")).await?;
    let challenge = limited.next().await.context("missing rate challenge")??;
    let challenge: Value = serde_json::from_str(challenge.to_text()?)?;
    limited
        .send(Message::Text(
            auth(
                &a.identity,
                group,
                challenge["nonce"].as_str().context("nonce")?,
            )
            .to_string()
            .into(),
        ))
        .await?;
    ensure!(
        limited
            .next()
            .await
            .context("missing rate ready")??
            .to_text()?
            .contains("ready"),
        "rate fixture auth rejected"
    );
    let mut envelope = vec![1, 3];
    envelope.extend_from_slice(&a.identity.public());
    let target = group
        .members()
        .into_iter()
        .find(|p| *p != a.identity.public())
        .context("missing rate target")?;
    envelope.extend_from_slice(&target);
    envelope.extend_from_slice(&[0; 96]);
    for _ in 0..650 {
        if limited
            .feed(Message::Binary(envelope.clone().into()))
            .await
            .is_err()
        {
            break;
        }
    }
    let _ = limited.flush().await;
    ensure!(
        matches!(
            tokio::time::timeout(Duration::from_secs(5), limited.next())
                .await
                .context("relay rate quota did not close the connection")?,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None
        ),
        "relay rate quota not enforced"
    );
    Ok(())
}

async fn started(
    a: Arc<Device>,
    b: Arc<Device>,
    ag: Group,
    bg: Group,
    relay: RelayUrl,
) -> Result<(Running, Running)> {
    let ar = SyncClient::start(a.clone(), ag, relay.clone())?;
    let br = SyncClient::start(b.clone(), bg, relay)?;
    same(&a, &b).await?;
    Ok((ar, br))
}

fn edit(device: &Device, op: impl FnOnce(&mut Automerge) -> Result<()>) -> Result<()> {
    device.store(|s| s.edit("tasks", op))
}

async fn conflict_cases(
    a: Arc<Device>,
    b: Arc<Device>,
    ag: &Group,
    bg: &Group,
    relay: &RelayUrl,
) -> Result<Value> {
    edit(&a, |d| model::put(d, "task-1", "owner", "Alice"))?;
    edit(&b, |d| model::put(d, "task-1", "due", "2030-01-02"))?;
    let (ar, br) = started(a.clone(), b.clone(), ag.clone(), bg.clone(), relay.clone()).await?;
    ensure!(
        model::values(&a.load("tasks")?, "task-1", "owner")? == ["Alice"],
        "independent owner lost"
    );
    ensure!(
        model::values(&b.load("tasks")?, "task-1", "due")? == ["2030-01-02"],
        "independent due lost"
    );
    ar.stop().await?;
    br.stop().await?;
    edit(&a, |d| {
        model::put(d, "task-1", "owner", "concurrent-A")?;
        model::check(d, "task-1")?;
        model::splice(d, "task-1", 2, 0, "alpha")
    })?;
    edit(&b, |d| {
        model::put(d, "task-1", "owner", "concurrent-B")?;
        model::uncheck(d, "task-1")?;
        model::splice(d, "task-1", 2, 0, "beta")
    })?;
    let (ar, br) = started(a.clone(), b.clone(), ag.clone(), bg.clone(), relay.clone()).await?;
    for device in [&a, &b] {
        let d = device.load("tasks")?;
        ensure!(
            model::values(&d, "task-1", "owner")?.len() == 2,
            "concurrent scalar lost"
        );
        ensure!(model::checked(&d, "task-1")?, "add-wins check failed");
        let title = model::text(&d, "task-1")?;
        ensure!(
            title.contains("alpha") && title.contains("beta"),
            "character merge failed"
        );
    }
    let merged_text = model::text(&a.load("tasks")?, "task-1")?;
    ar.client.edit("tasks", |d| model::uncheck(d, "task-1"))?;
    same(&a, &b).await?;
    ensure!(
        !model::checked(&b.load("tasks")?, "task-1")?,
        "later uncheck failed"
    );
    ar.client
        .edit("tasks", |d| model::resolve(d, "task-1", "owner", "chosen"))?;
    same(&a, &b).await?;
    ensure!(
        model::values(&b.load("tasks")?, "task-1", "owner")? == ["chosen"],
        "resolution failed"
    );
    let recovered = model::recovery(&b.load("tasks")?, "task-1")?;
    ensure!(
        recovered.len() == 1
            && recovered[0].values.contains(&"concurrent-A".to_owned())
            && recovered[0].values.contains(&"concurrent-B".to_owned()),
        "conflict recovery record missing alternatives"
    );
    ar.stop().await?;
    br.stop().await?;
    edit(&a, |d| model::delete(d, "task-1"))?;
    edit(&b, |d| {
        model::put(d, "task-1", "owner", "recoverable concurrent edit")
    })?;
    let (ar, br) = started(a.clone(), b.clone(), ag.clone(), bg.clone(), relay.clone()).await?;
    for device in [&a, &b] {
        let d = device.load("tasks")?;
        ensure!(
            model::deleted(&d, "task-1")?
                && model::values(&d, "task-1", "owner")? == ["recoverable concurrent edit"],
            "delete/edit recovery failed"
        );
    }
    ar.stop().await?;
    br.stop().await?;
    Ok(
        json!({"independent_fields":true,"concurrent_scalar_values":2,"add_wins_check":true,"later_uncheck":true,"character_text":merged_text,"resolve_retains_recovery":true,"delete_edit_recoverable":true}),
    )
}

#[cfg(windows)]
fn resources() -> Result<(f64, u64)> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    };
    let mut create = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let mut memory = PROCESS_MEMORY_COUNTERS::default();
    memory.cb = std::mem::size_of_val(&memory) as u32;
    unsafe {
        ensure!(
            GetProcessTimes(
                GetCurrentProcess(),
                &mut create,
                &mut exit,
                &mut kernel,
                &mut user
            ) != 0,
            "process times failed"
        );
        ensure!(
            K32GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, memory.cb) != 0,
            "working set failed"
        );
    }
    let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
    Ok((
        (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0,
        memory.WorkingSetSize as u64,
    ))
}
#[cfg(not(windows))]
fn resources() -> Result<(f64, u64)> {
    anyhow::bail!("Windows resource API required")
}

fn crash_child(path: &Path) -> Result<()> {
    let mut store = sidenote_sync::store::Store::open(path, b"crash-fixture")?;
    store.edit("tasks", |d| {
        model::create(d, "task-1", "committed-before-kill")
    })?;
    drop(store);
    let conn = rusqlite::Connection::open(path)?;
    conn.execute_batch("PRAGMA synchronous=FULL; BEGIN IMMEDIATE; UPDATE documents SET data=zeroblob(1048576) WHERE id='tasks';")?;
    println!("uncommitted-write-ready");
    std::io::stdout().flush()?;
    std::thread::sleep(Duration::from_secs(30));
    anyhow::bail!("parent failed to terminate crash child")
}
fn crash_test(dir: &Path) -> Result<()> {
    event("crash_child_starting");
    let path = dir.join("crash.sqlite");
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("--crash-child")
        .arg(&path)
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().context("child stdout")?).read_line(&mut line)?;
    ensure!(
        line.trim() == "uncommitted-write-ready",
        "crash child failed"
    );
    child.kill()?;
    child.wait()?;
    event("crash_child_killed");
    let store = sidenote_sync::store::Store::open(&path, b"crash-fixture")?;
    ensure!(
        model::text(&store.load("tasks")?, "task-1")? == "committed-before-kill",
        "uncommitted crash lost document"
    );
    let conn = rusqlite::Connection::open(&path)?;
    event("crash_document_recovered");
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(integrity == "ok", "crash database integrity failure");
    Ok(())
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|s| s == "--crash-child") {
        return crash_child(Path::new(args.get(2).context("missing child path")?));
    }
    let output = Path::new(args.get(1).context("usage: local_e2e <evidence.json>")?);
    let quick = args.get(2).is_some_and(|s| s == "--quick");
    let start = Instant::now();
    event("harness_starting");
    let dir = tempfile::tempdir()?;
    crash_test(dir.path())?;
    event("crash_recovery_passed");
    let a = Arc::new(Device::open(&dir.path().join("a.sqlite"))?);
    let b = Arc::new(Device::open(&dir.path().join("b.sqlite"))?);
    let c = Arc::new(Device::open(&dir.path().join("c.sqlite"))?);
    let relay = RelayUrl::local("ws://127.0.0.1:8787")?;
    three_failures(&relay, a.clone(), c.clone()).await?;
    event("three_failures_close_room");
    declined_confirmation(&relay, a.clone(), c.clone()).await?;
    event("declined_confirmation_rejected");
    let (ag, bg) = pair(&relay, a.clone(), b.clone(), None).await?;
    rejected_auth(&a, &c, &ag).await?;
    a.merge_group(&ag)?;
    b.merge_group(&bg)?;
    ensure!(
        a.store(|s| Group::load(s, &ag.room()))?.members() == ag.members(),
        "protected group reload failed"
    );
    ensure!(
        a.store(|s| Ok(sidenote_sync::identity::Identity::load_or_create(s)?.public()))?
            == a.identity.public(),
        "identity changed on restart"
    );
    event("pairing_and_auth_passed");
    for domain in DOMAINS {
        a.store(|s| {
            s.edit(domain, |d| {
                model::create(d, "task-1", "Task")?;
                model::check(d, "task-1")
            })
        })?;
    }
    let (ar, br) = started(a.clone(), b.clone(), ag.clone(), bg.clone(), relay.clone()).await?;
    for domain in DOMAINS {
        ensure!(
            model::text(&b.load(domain)?, "task-1")? == "Task",
            "domain missing"
        );
    }
    ar.stop().await?;
    br.stop().await?;
    let conflicts = conflict_cases(a.clone(), b.clone(), &ag, &bg, &relay).await?;
    event("all_merge_cases_passed");
    let (ar, br) = started(a.clone(), b.clone(), ag.clone(), bg.clone(), relay.clone()).await?;
    // Benchmark a fresh, visible task; the conflict fixture remains as recovery evidence.
    ar.client
        .edit("tasks", |d| model::create(d, "bench", "Benchmark"))?;
    same(&a, &b).await?;
    let mut updates = br.client.subscribe();
    let mut samples = Vec::new();
    for index in 0..if quick { 30 } else { 1020 } {
        let value = format!(
            "{MARKER}:{index:04}:{}",
            "x".repeat(1024 - MARKER.len() - 6)
        );
        let (committed, heads) = ar.client.edit("tasks", |d| {
            model::put(d, "bench", "due", &value)?;
            Ok((Instant::now(), d.get_heads()))
        })?;
        let applied = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let current = updates.borrow_and_update().clone();
                if let Some(applied) = current
                    && applied.domain == "tasks"
                    && applied.heads == heads
                {
                    return Ok::<_, anyhow::Error>(applied.at);
                }
                updates.changed().await?;
            }
        })
        .await
        .context("benchmark delivery timeout")??;
        ensure!(
            model::values(&b.load("tasks")?, "bench", "due")? == [value],
            "benchmark value missing"
        );
        if index >= 20 {
            samples.push(applied.duration_since(committed).as_secs_f64() * 1000.0);
        }
        // Keep throughput below the explicit relay quota; pacing is outside each sample.
        tokio::time::sleep(Duration::from_millis(20)).await;
        if index % 100 == 0 {
            println!("{}", json!({"event":"latency_progress","edits":index}));
        }
    }
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let count = sorted.len();
    let latency = json!({"edits":samples.len(),"warmup":20,"payload_bytes":1024,"failed_deliveries":0,"median_ms":(sorted[(count-1)/2]+sorted[count/2])/2.0,"p95_ms":sorted[(count as f64*0.95).ceil() as usize-1],"max_ms":sorted[count-1],"samples_ms":samples});
    event("latency_passed");
    br.stop().await?;
    let offline_start = Instant::now();
    for index in 0..500 {
        ar.client.edit("tasks", |d| {
            model::put(
                d,
                "bench",
                "due",
                &format!("offline-{index:04}:{}", "x".repeat(1011)),
            )
        })?;
    }
    let offline_edits_ms = offline_start.elapsed().as_secs_f64() * 1000.0;
    drop(b);
    let b = Arc::new(Device::open(&dir.path().join("b.sqlite"))?);
    let bg = b.store(|s| Group::load(s, &bg.room()))?;
    let restart = Instant::now();
    let br = SyncClient::start(b.clone(), bg, relay.clone())?;
    same(&a, &b).await?;
    let catchup_ms = restart.elapsed().as_secs_f64() * 1000.0;
    ensure!(
        model::values(&b.load("tasks")?, "bench", "due")?[0].starts_with("offline-0499:"),
        "offline tail missing"
    );
    event("offline_restart_catchup_passed");
    // Force an authenticated connection replacement; the displaced client must reconnect.
    let before = ar.client.connections.load(Ordering::Relaxed);
    let endpoint = format!("ws://127.0.0.1:8787/sync/{}?role=device", ag.room());
    let mut replacement = WebSocketTransport::connect(&endpoint).await?;
    let challenge = text(&mut replacement).await?;
    replacement
        .send(Frame::Text(
            auth(
                &a.identity,
                &ag,
                challenge["nonce"].as_str().context("nonce")?,
            )
            .to_string(),
        ))
        .await?;
    ensure!(
        text(&mut replacement).await?["type"] == "ready",
        "replacement rejected"
    );
    replacement.close().await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while ar.client.connections.load(Ordering::Relaxed) <= before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    ar.client.edit("tasks", |d| {
        model::put(d, "bench", "owner", "after-auto-reconnect")
    })?;
    same(&a, &b).await?;
    event("automatic_reconnect_passed");
    br.stop().await?;
    let (extended, cg) = pair(&relay, a.clone(), c.clone(), Some(ar.client.group()?)).await?;
    event("third_device_paired");
    ar.client.update_membership(extended.enrollments.clone())?;
    let old_bg = b.store(|s| Group::load(s, &ag.room()))?;
    ensure!(
        old_bg.members().len() == 2,
        "offline membership fixture already updated"
    );
    let br = SyncClient::start(b.clone(), old_bg, relay.clone())?;
    let cr = SyncClient::start(c.clone(), cg, relay.clone())?;
    same(&a, &c).await?;
    event("third_device_synced");
    tokio::time::timeout(Duration::from_secs(10), async {
        while br.client.group()?.members().len() != 3 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("offline member did not learn signed roster")??;
    cr.client
        .edit("tasks", |d| model::put(d, "bench", "owner", "third-device"))?;
    same(&a, &c).await?;
    same(&a, &b).await?;
    cr.stop().await?;
    drop(c);
    event("third_device_and_offline_membership_passed");
    // CI-only bulk regression, outside the measured edit workload.
    let connections_before_bulk = (
        ar.client.connections.load(Ordering::Relaxed),
        br.client.connections.load(Ordering::Relaxed),
    );
    let mut state = 0x9e3779b9u32;
    let mut bulk = String::with_capacity(24 * 1024 * 1024);
    for _ in 0..24 * 1024 * 1024 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        bulk.push((b'!' + (state % 90) as u8) as char);
    }
    bulk.replace_range(..MARKER.len(), MARKER);
    let digest = sidenote_sync::identity::hash(bulk.as_bytes());
    ar.client
        .edit("memory", |d| model::put(d, "task-1", "bulk-fixture", &bulk))?;
    // The instrumented bulk path includes cold loads, whose worker budget is up to 120 seconds.
    same_with_timeout(&a, &b, Duration::from_secs(120))
        .await
        .with_context(|| {
            let reasons = |running: &Running| {
                running.client.peer_statuses().map(|peers| {
                    peers
                        .into_iter()
                        .map(|peer| peer.reason)
                        .collect::<Vec<_>>()
                })
            };
            format!(
                "bulk catch-up peer reasons: {:?}, {:?}; document reasons: {:?}, {:?}",
                reasons(&ar),
                reasons(&br),
                ar.client.document_status("memory"),
                br.client.document_status("memory")
            )
        })?;
    let values = model::values(&b.load("memory")?, "task-1", "bulk-fixture")?;
    ensure!(
        values.len() == 1 && sidenote_sync::identity::hash(values[0].as_bytes()) == digest,
        "bulk scalar mismatch"
    );
    drop(values);
    drop(bulk);
    ensure!(
        (
            ar.client.connections.load(Ordering::Relaxed),
            br.client.connections.load(Ordering::Relaxed)
        ) == connections_before_bulk,
        "bulk transfer triggered relay disconnect/reconnect"
    );
    event("f3_twenty_four_mib_incompressible_catchup_passed");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let idle_start = Instant::now();
    let (cpu_before, _) = resources()?;
    let mut working_sets = Vec::new();
    for _ in 0..if quick { 1 } else { 60 } {
        tokio::time::sleep(Duration::from_secs(1)).await;
        working_sets.push(resources()?.1);
    }
    let (cpu_after, _) = resources()?;
    let wall = idle_start.elapsed().as_secs_f64();
    let idle = json!({"scope":"parent process containing two clients and the harness; worker processes and relay excluded","worker_resource_measurement":"pending; parent-only figures are not total client resource use","wall_seconds":wall,"cpu_seconds":cpu_after-cpu_before,"cpu_percent_one_core":(cpu_after-cpu_before)/wall*100.0,"working_set_mean_bytes":working_sets.iter().sum::<u64>() as f64/working_sets.len() as f64,"working_set_min_bytes":working_sets.iter().min(),"working_set_max_bytes":working_sets.iter().max(),"samples":working_sets.len()});
    ar.client.edit("tasks", |d| {
        model::put(d, "bench", "due", "after-idle-wake")
    })?;
    same(&a, &b).await?;
    event("idle_wake_sync_passed");
    for domain in DOMAINS {
        ensure!(
            a.load(domain)?.get_heads() == b.load(domain)?.get_heads(),
            "final domain divergence"
        );
    }
    let a_connections = ar.client.connections.load(Ordering::Relaxed);
    let b_connections = br.client.connections.load(Ordering::Relaxed);
    ar.stop().await?;
    br.stop().await?;
    let report = json!({"relay":"local wrangler/workerd at numeric loopback","domains":DOMAINS,"crash_uncommitted_transaction_recovered":true,"dpapi_identity_and_group_reload":true,"pairing":{"right_code":true,"wrong_code":true,"three_failures_close":true,"single_use":true,"sas_matches":true,"explicit_rejection_blocks_group":true},"authentication":{"unknown_device_rejected":true,"fresh_challenge_replay_rejected":true,"server_oversize_rejected":true,"server_rate_limit_enforced":true},"conflicts":conflicts,"latency":latency,"offline":{"edits":500,"edit_time_ms":offline_edits_ms,"restart_to_exact_heads_ms":catchup_ms},"automatic_reconnect":true,"third_device_and_offline_membership":true,"bulk_sync_bytes":24*1024*1024,"connections":{"a":a_connections,"b":b_connections},"idle":idle,"harness_seconds":start.elapsed().as_secs_f64(),"plaintext_marker":MARKER});
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"event":"passed","evidence_written":true,"median_ms":report["latency"]["median_ms"],"p95_ms":report["latency"]["p95_ms"],"max_ms":report["latency"]["max_ms"],"idle_cpu_percent_one_core":report["idle"]["cpu_percent_one_core"]})
    );
    Ok(())
}
