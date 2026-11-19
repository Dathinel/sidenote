use sha2::{Digest, Sha256};
use sidenote::core::{
    asr,
    meeting_router::{
        batches::{
            Range, Recognizer,
            worker::{Worker, WorkerFailure},
        },
        ranges::AsrRange,
    },
    model_catalog,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
const DATA: &str = "https://huggingface.co/datasets/dhruvkys/11k-asr/resolve/dc161f4fcf07cdfcd3852d9467e66c81964c5e52/test/clips";
const CLIPS: [(&str, &str); 2] = [
    (
        "common_voice_hi_26988540.mp3",
        "5f461e5f6ad6e5d7dfd9fd735e1a9f48b6365fc6bed0037d671ff970451d9dcc",
    ),
    (
        "common_voice_hi_24976572.mp3",
        "dea22a7b1e8f324ce3e0720bd2e41b0b189ba6e8225657f89d6f510b7e46526b",
    ),
];
fn fetch(path: &Path, url: &str, sha: &str) {
    if !path.exists() {
        let bytes = ureq::get(url)
            .call()
            .unwrap()
            .into_body()
            .with_config()
            .limit(256 << 20)
            .read_to_vec()
            .unwrap();
        assert_eq!(hex::encode(Sha256::digest(&bytes)), sha);
        fs::write(path, bytes).unwrap();
    }
    assert_eq!(hex::encode(Sha256::digest(fs::read(path).unwrap())), sha);
}
fn fixtures() -> (PathBuf, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/l4-fixtures");
    fs::create_dir_all(&root).unwrap();
    let model = model_catalog::model("indicconformer-hi-ctc").unwrap();
    for file in model.files {
        let path = root.join(file.name);
        fetch(&path, file.url, file.sha256);
        assert_eq!(fs::metadata(path).unwrap().len(), file.bytes);
    }
    let runtime =
        PathBuf::from(std::env::var_os("SIDENOTE_L4_RUNTIME").expect("absolute pinned L1 runtime"));
    assert!(runtime.is_absolute());
    assert_eq!(
        hex::encode(Sha256::digest(fs::read(&runtime).unwrap())),
        "4b3607aebd1784b26b6f9b20e4bd974c7ab8287043e4d095cb7d2cb40b5e566e"
    );
    (root, runtime)
}
fn clip(root: &Path, index: usize, temp: &Path) -> PathBuf {
    let (name, sha) = CLIPS[index];
    let source = root.join(name);
    fetch(&source, &format!("{DATA}/{name}"), sha);
    let wav = temp.join(format!("{index}.wav"));
    // Conversion is test setup only and uses the same contained launcher.
    let mut command =
        sidenote_sync::tree_worker::speech_command(Path::new("/usr/bin/ffmpeg").as_os_str())
            .unwrap();
    command
        .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .args(["-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .env_clear();
    let mut child = sidenote_sync::tree_worker::spawn_speech(&mut command).unwrap();
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success(), "public MP3 conversion");
    wav
}
fn worker(root: &Path, runtime: &Path) -> Worker {
    let mut command = sidenote_sync::tree_worker::speech_command(
        Path::new(env!("CARGO_BIN_EXE_sidenote")).as_os_str(),
    )
    .unwrap();
    command
        .args(["--routing-native", "hi", "--graph"])
        .arg(root.join("model.int8.onnx"))
        .arg("--tokens")
        .arg(root.join("tokens.txt"))
        .arg("--runtime-dll")
        .arg(runtime);
    Worker::spawn_with_threads(
        command,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(120),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
}
fn range(wav: PathBuf) -> Range {
    let samples = asr::read_wav(&wav).unwrap().len() as u64;
    Range {
        track: "public".into(),
        piece: "l4".into(),
        wav,
        range: AsrRange {
            context: (0, samples),
            members: vec![(0, (0, samples))],
            language: Some("hi".into()),
        },
        model_id: "indicconformer-hi-ctc".into(),
        model_sha256: model_catalog::model("indicconformer-hi-ctc").unwrap().files[0]
            .sha256
            .into(),
    }
}
fn descendants(pid: u32) -> Vec<u32> {
    let mut result = vec![pid];
    let mut i = 0;
    while i < result.len() {
        let children =
            fs::read_to_string(format!("/proc/{0}/task/{0}/children", result[i])).unwrap();
        result.extend(
            children
                .split_whitespace()
                .map(|p| p.parse::<u32>().unwrap()),
        );
        i += 1;
    }
    result
}
fn gone(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        if pids
            .iter()
            .all(|pid| !Path::new(&format!("/proc/{pid}")).exists())
        {
            break;
        }
        assert!(Instant::now() < deadline, "leftover process in {pids:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    eprintln!("reaped all PIDs {pids:?}");
}
#[test]
#[ignore = "downloads the public 198 MB Hindi pack; run explicitly on the Linux box"]
fn public_hindi_native_entry_returns_two_ranges() {
    let (root, runtime) = fixtures();
    let temp = tempfile::tempdir().unwrap();
    let ranges = [
        range(clip(&root, 0, temp.path())),
        range(clip(&root, 1, temp.path())),
    ];
    let mut worker = worker(&root, &runtime);
    for range in &ranges {
        let value = worker
            .recognize(range)
            .expect("real native_worker transcription");
        eprintln!("native reply {value}");
        assert!(!value["text"].as_str().unwrap().trim().is_empty());
        assert_eq!(value["timing"], "range");
        assert_eq!(value["start_sample"], 0);
        assert_eq!(value["end_sample"], range.range.context.1);
    }
    let pids = descendants(worker.pid());
    assert_eq!(pids.len(), 3, "launcher, namespace init, recognizer");
    for pid in &pids {
        assert!(unsafe { libc::getpriority(libc::PRIO_PROCESS, *pid) } >= 10);
    }
    eprintln!(
        "native {}",
        fs::read_to_string(format!("/proc/{}/status", pids[2]))
            .unwrap()
            .lines()
            .find(|l| l.starts_with("VmHWM:"))
            .unwrap()
    );
    worker.finish().unwrap();
    gone(&pids);
}
fn cpu_ticks(pid: u32) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}
#[test]
#[ignore = "downloads the public 198 MB Hindi pack; run explicitly on the Linux box"]
fn killed_native_entry_reports_exited_and_reaps_the_tree() {
    let (root, runtime) = fixtures();
    let temp = tempfile::tempdir().unwrap();
    let wav = clip(&root, 0, temp.path());
    let mut worker = worker(&root, &runtime);
    worker.recognize(&range(wav.clone())).unwrap();
    let pids = descendants(worker.pid());
    assert_eq!(pids.len(), 3);
    let native = pids[2];
    let mut samples = asr::read_wav(&wav).unwrap();
    samples = samples.iter().copied().cycle().take(480_000).collect();
    let long = temp.path().join("30s.wav");
    asr::write_wav(&long, &samples).unwrap();
    let range = range(long);
    let before = cpu_ticks(native);
    std::thread::scope(|scope| {
        let call = scope.spawn(|| worker.recognize(&range));
        let deadline = Instant::now() + Duration::from_secs(5);
        while cpu_ticks(native) <= before {
            assert!(
                Instant::now() < deadline,
                "second inference must start before kill"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(unsafe { libc::kill(native as i32, libc::SIGKILL) }, 0);
        let error = call.join().unwrap().unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<WorkerFailure>(),
                Some(WorkerFailure::Exited(_))
            ),
            "{error}"
        );
        eprintln!("caller received {error}");
    });
    worker.finish().unwrap();
    gone(&pids);
}
