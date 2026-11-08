//! Manual downloader proof. Always creates an isolated OS temporary state folder.
use sidenote::core::model_catalog::{ModelStore, catalog};
use std::sync::atomic::AtomicBool;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id = std::env::args().nth(1).ok_or("Pass one catalogue id")?;
    let model = catalog()
        .iter()
        .find(|m| m.id == id)
        .ok_or("Unknown model")?;
    if model.bytes() > 300_000_000 {
        return Err("Proof is limited to one small model".into());
    }
    let root = std::env::temp_dir().join(format!("sidenote-model-proof-{}", std::process::id()));
    let store = ModelStore::new(root.clone());
    println!("Temporary state: {}", root.display());
    let cancel = AtomicBool::new(false);
    let interrupted = store
        .install(&id, &cancel, |p| {
            if p.downloaded > 1_000_000 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .unwrap_err();
    assert_eq!(interrupted.kind(), std::io::ErrorKind::Interrupted);
    let partial = root
        .join("models")
        .join(&id)
        .join(format!("{}.part", model.files[0].name));
    let offset = std::fs::metadata(&partial)?.len();
    assert!(offset > 0);
    println!("Cancelled partial retained: {offset} bytes. Resuming.");
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    store.install(&id, &cancel, |p| {
        println!("{:?}: {} / {}", p.phase, p.downloaded, p.total)
    })?;
    let paths = store
        .installed_paths(&id)?
        .ok_or("Install receipt or files missing")?;
    assert_eq!(paths.len(), model.files.len());
    for path in &paths {
        println!("Installed: {}", path.display());
    }
    // Exercise a completed corrupt partial without another network download.
    std::fs::remove_file(root.join("models").join(&id).join("installed.json"))?;
    let path = &paths[0];
    let partial = path.with_file_name(format!("{}.part", model.files[0].name));
    std::fs::rename(path, &partial)?;
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).open(&partial)?;
        file.write_all(b"BAD!")?;
    }
    let failure = store
        .install(&id, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert!(failure.to_string().contains("Checksum verification failed"));
    assert!(!partial.exists());
    assert!(store.installed_paths(&id)?.is_none());
    let cancelled = store
        .install(&id, &AtomicBool::new(true), |_| {})
        .unwrap_err();
    assert_eq!(cancelled.kind(), std::io::ErrorKind::Interrupted);
    store.remove(&id)?;
    assert!(!root.join("models").join(&id).exists());
    println!(
        "Verified install paths, checksum rejection/deletion, cancellation and removal. Temporary root: {}",
        root.display()
    );
    Ok(())
}
