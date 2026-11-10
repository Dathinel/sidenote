//! Synthetic live-transport probe. It never loads a meeting or sends private text.
use sidenote::core::{self, AskProgress, Runner};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

fn main() {
    let runner = Runner {
        name: "Codex configured default".into(),
        program: "codex".into(),
        model: None,
        args: [
            "exec",
            "--sandbox",
            "read-only",
            "--json",
            "-c",
            "model_reasoning_effort=low",
            "-",
        ]
        .map(str::to_owned)
        .to_vec(),
        stdin: true,
        tools: true,
    };
    let question = "In two lines, what is a stand-up meeting? Answer directly. Do not use tools.";
    core::ask_session::warm(runner.clone(), "synthetic-warm-probe".into());
    let priming = core::ask_session::run(&runner, question.into(), "synthetic-warm-probe", |_| {})
        .expect("Priming turn");
    assert!(!priming.is_empty());
    let mut rows = Vec::new();
    for run in 1..=3 {
        let start = Instant::now();
        let first = Cell::new(None);
        let answer =
            core::ask_session::run(&runner, question.into(), "synthetic-warm-probe", |event| {
                if matches!(event, AskProgress::Text { .. }) && first.get().is_none() {
                    first.set(Some(start.elapsed().as_secs_f64()));
                }
            })
            .expect("Warm turn");
        rows.push(serde_json::json!({"run":run,"first_text_s":first.get(),"total_s":start.elapsed().as_secs_f64(),"answer":answer}));
    }
    assert!(
        core::ask::cancel_all(Duration::from_secs(5)),
        "Warm process did not stop"
    );
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/ask-models-round1/codex-warm-core.json");
    std::fs::write(out, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    for row in rows {
        println!(
            "run={} first_text_s={} total_s={}",
            row["run"], row["first_text_s"], row["total_s"]
        );
    }
}
