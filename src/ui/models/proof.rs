use super::{
    super::State, CPU_DOWNLOAD_KEY, Msg, Phase, click_model_control, settings_models_runtime,
};
use sidenote::core::{self, model_catalog::CPU_LANGUAGE_PACKS};
use std::{
    collections::HashSet,
    io, thread,
    time::{Duration, Instant},
};

pub(crate) fn run(mode: &str) -> io::Result<()> {
    let root = core::state_dir();
    if ![root.clone(), core::meetings_dir(), core::peers_dir()]
        .iter()
        .all(|path| core::is_proof_path(path, "dev40-settings-proof"))
        || sidenote::updates::installed()
        || !core::Config::load_in(&root)?
            .pipeline
            .model_state_dir
            .as_os_str()
            .is_empty()
    {
        return Err(io::Error::other(
            "Use an extracted app and temporary proof folders",
        ));
    }
    let mut state = State::new();
    if matches!(mode, "settings-download" | "settings-cancel")
        && !state.config.features.indic_engine
    {
        state.toggle_feature(core::Feature::IndicEngine);
    }
    let mut runtime = settings_models_runtime(state);
    match mode {
        "startup" => {
            runtime.poll = Some(super::super::poll);
            for _ in 0..50 {
                runtime.frame(0.1);
                if !runtime.state.models.jobs.is_empty() || root.join("models").exists() {
                    return Err(io::Error::other("Startup triggered a model download"));
                }
                thread::sleep(Duration::from_millis(100));
            }
            println!(
                "Startup ran the desktop poll for five seconds with no model jobs or model files"
            );
        }
        "settings-download" | "settings-cancel" => {
            let cancel_requested = mode == "settings-cancel";
            let mut cancelled = false;
            let start = Instant::now();
            if !click_model_control(&mut runtime, CPU_DOWNLOAD_KEY)
                || runtime.state.models.pack_queue.is_empty()
            {
                return Err(io::Error::other(
                    "The Settings Models button did not start a download",
                ));
            }
            println!("Clicked the Settings Models download button");
            let mut downloading = HashSet::new();
            let mut verifying = HashSet::new();
            while !runtime.state.models.pack_queue.is_empty() {
                if runtime.state.models.jobs.len() != 1 {
                    return Err(io::Error::other("Expected one manual model job"));
                }
                let event = runtime
                    .state
                    .events_rx
                    .recv_timeout(Duration::from_secs(60))
                    .map_err(io::Error::other)?;
                let mut cancel_now = false;
                if let super::super::Event::Models(msg) = &event {
                    match msg {
                        Msg::Progress(id, p) => {
                            let first = match p.phase {
                                Phase::Downloading => downloading.insert(*id),
                                Phase::Verifying => verifying.insert(*id),
                            };
                            cancel_now = cancel_requested
                                && !cancelled
                                && *id == "anuvani-lid-int8"
                                && matches!(p.phase, Phase::Downloading)
                                && p.downloaded >= 262_144;
                            if first {
                                println!(
                                    "{id} {:?} {}/{} at {} ms",
                                    p.phase,
                                    p.downloaded,
                                    p.total,
                                    start.elapsed().as_millis()
                                );
                            }
                        }
                        Msg::Finished(id, result) => {
                            if let Err(error) = result {
                                if !cancelled || error.kind() != io::ErrorKind::Interrupted {
                                    return Err(io::Error::new(
                                        error.kind(),
                                        format!("{id}: {error}"),
                                    ));
                                }
                                println!("{id} cancelled at {} ms", start.elapsed().as_millis());
                            } else {
                                println!(
                                    "{id} receipt written at {} ms",
                                    start.elapsed().as_millis()
                                );
                            }
                        }
                    }
                }
                runtime.state.apply_event(event);
                if cancel_now {
                    runtime.invalidate();
                    runtime.frame(0.01);
                    cancelled = click_model_control(&mut runtime, "model-cancel-anuvani-lid-int8");
                    if !cancelled {
                        return Err(io::Error::other(
                            "Could not click the Settings Cancel button",
                        ));
                    }
                }
            }
            if cancel_requested {
                if !cancelled
                    || runtime.state.models.installed.len() != 1
                    || !runtime.state.models.installed.contains("silero-vad")
                {
                    return Err(io::Error::other(
                        "Cancel did not stop the queue after one pack",
                    ));
                }
            } else if CPU_LANGUAGE_PACKS
                .iter()
                .any(|id| !runtime.state.models.installed.contains(id))
            {
                return Err(io::Error::other(
                    "A Settings model pack has no verified receipt",
                ));
            }
        }
        _ => return Err(io::Error::other("Unknown Settings model proof mode")),
    }
    Ok(())
}
