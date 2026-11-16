//! Replay frozen public receipts without opening audio or loading a speech model.
#[cfg(all(windows, feature = "asr-runtime"))]
fn main() -> sidenote::core::asr::Result<()> {
    use sidenote::core::meeting_router::{Chunk, batches::Plan, production, revision};
    use std::{fs, path::PathBuf, time::Instant};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("Usage: assembly_replay SAVED_PUBLIC_INPUTS OUTPUT".into());
    }
    let saved = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    let temp = tempfile::tempdir()?;
    for file in fs::read_dir(saved)? {
        let file = file?;
        fs::copy(file.path(), temp.path().join(file.file_name()))?;
    }
    let read = |name: &str| fs::read(temp.path().join(name));
    let plan: Plan = serde_json::from_slice(&read("batch-plan.json")?)?;
    fs::write(
        temp.path().join(format!("{}.plan.json", plan.revision)),
        serde_json::to_vec(&plan)?,
    )?;
    let pieces = serde_json::from_slice(&read("pieces.json")?)?;
    let baseline: Vec<Chunk> = serde_json::from_slice(&read("baseline.json")?)?;
    let started = Instant::now();
    let data = production::assemble_saved(temp.path(), &plan, &pieces, &baseline, 1)?;
    let assembly_seconds = started.elapsed().as_secs_f64();
    let dest = temp.path().join("published");
    revision::begin(&dest, 1)?;
    let started = Instant::now();
    let chosen = revision::commit(&dest, &data)?;
    let publication_seconds = started.elapsed().as_secs_f64();
    let (readback_id, readback) = revision::load(&dest, None)?.ok_or("No selected revision")?;
    assert_eq!(chosen, readback_id);
    assert_eq!(readback.evidence.len(), 291);
    fs::create_dir_all(&out)?;
    fs::copy(
        dest.join("transcript.segments.v1.json"),
        out.join("selected.json"),
    )?;
    let summary = serde_json::json!({"revision":chosen,"assembly_seconds":assembly_seconds,
        "publication_seconds":publication_seconds,"audio_seconds":1800,"profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "planned":plan.batches.values().map(|b| b.len()).sum::<usize>(),"evidence":readback.evidence.len(),
        "segments":readback.transcript.segments.len(),"no_speech":readback.provenance["no_speech"],
        "coverage":readback.provenance["assembly_coverage"],"omitted":"all audio inference, physical Stop, normalization, planning, notes"});
    fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!("{summary}");
    Ok(())
}

#[cfg(not(all(windows, feature = "asr-runtime")))]
fn main() {
    eprintln!("assembly_replay requires Windows and asr-runtime");
    std::process::exit(2);
}
