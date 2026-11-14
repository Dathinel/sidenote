//! Public-audio runtime proof, not a test suite. No app state is opened.
#[cfg(feature = "asr-runtime")]
fn main() -> sidenote::core::asr::Result<()> {
    use sidenote::core::asr::{self, LanguageId, SpeechEngine};
    use std::{path::Path, time::Instant};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 {
        return Err("Usage: asr_spike <dll> <lid.onnx> <hi.onnx> <tokens.txt> <wav>".into());
    }
    asr::initialize(Path::new(&args[0]))?;
    let mut lid = asr::Anuvani::load(Path::new(&args[1]))?;
    let mut hi = asr::IndicConformer::load(Path::new(&args[2]), Path::new(&args[3]))?;
    let start = Instant::now();
    let path = Path::new(&args[4]);
    let mut files = if path.is_dir() {
        std::fs::read_dir(path)?
            .map(|p| p.map(|p| p.path()))
            .collect::<std::io::Result<Vec<_>>>()?
    } else {
        vec![path.to_owned()]
    };
    files.retain(|p| p.extension().is_some_and(|e| e == "wav"));
    files.sort();
    let mut readings = Vec::new();
    for file in files.into_iter().take(50) {
        let audio = asr::read_wav(&file)?;
        let scores = lid.identify(&audio, &["en".into(), "hi".into()])?;
        let reading = hi.transcribe(&audio, asr::SAMPLE_RATE, "hi")?;
        readings
            .push(serde_json::json!({"id":file.file_stem().and_then(|s| s.to_str()), "scores":scores, "reading":reading}));
    }
    println!(
        "{}",
        serde_json::json!({"lid_load_seconds": lid.load_seconds, "hi_load_seconds": hi.load_seconds, "inference_seconds": start.elapsed().as_secs_f64(), "readings": readings})
    );
    Ok(())
}
#[cfg(not(feature = "asr-runtime"))]
fn main() {
    eprintln!("Build with --features asr-runtime");
}
