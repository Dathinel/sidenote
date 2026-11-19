//! Anuvani language ID on Linux through the production worker, on the catalogue's 169 MB INT8 pack.
//! Ignored by default because of the download; run with `-- --ignored`. The pack is hash checked.
use super::*;
use crate::core::{
    asr::fixtures::{fetch, fixtures, library, test_wav},
    model_catalog,
};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
};

fn settings(model: PathBuf) -> Settings {
    Settings {
        model,
        runtime_dll: library(),
        ..Settings::default()
    }
}

fn identify(settings: Settings, samples: Vec<f32>) -> Result<Guess, String> {
    let cache = tempfile::tempdir().unwrap();
    let path = cache.path().to_path_buf();
    let languages = ["en".to_string(), "hi".to_string()];
    worker::spawn(settings, path, false, move |w| {
        w.identify(&samples, &languages)
    })
    .unwrap()
    .join()
    .unwrap()
    .map_err(|e| e.to_string())
}

#[test]
#[ignore = "downloads the 169 MB Anuvani pack; run with --ignored"]
fn anuvani_pack_loads_and_identifies_and_a_changed_pack_is_refused() {
    let file = &model_catalog::model("anuvani-lid-int8").unwrap().files[0];
    // Fetched and hash checked, then dropped so the test holds no second copy while the loader runs.
    assert_eq!(
        fetch(file.name, file.url, file.sha256).len() as u64,
        file.bytes
    );
    let started = std::time::Instant::now();
    let samples = test_wav()[..5 * 16000].to_vec();
    let guess = identify(settings(fixtures().join(file.name)), samples.clone());
    eprintln!("guess {guess:?} in {:?}", started.elapsed());
    let status = fs::read_to_string("/proc/self/status").unwrap();
    eprintln!(
        "{}",
        status.lines().find(|l| l.starts_with("VmHWM")).unwrap()
    );
    assert!(guess.is_ok(), "{guess:?}");

    // Kernel copy and a one byte write in place: no second 169 MB buffer in the test process.
    let copy = tempfile::tempdir().unwrap();
    let changed = copy.path().join(file.name);
    fs::copy(fixtures().join(file.name), &changed).unwrap();
    let mut open = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&changed)
        .unwrap();
    open.seek(SeekFrom::Start(file.bytes / 2)).unwrap();
    let mut byte = [0];
    open.read_exact(&mut byte).unwrap();
    open.seek(SeekFrom::Start(file.bytes / 2)).unwrap();
    open.write_all(&[byte[0] ^ 1]).unwrap();
    drop(open);
    let error = identify(settings(changed), samples).unwrap_err();
    assert!(error.contains("integrity check failed"), "{error}");
}
