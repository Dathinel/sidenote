use sidenote::core::{self, AskProgress, Runner};
use std::{cell::RefCell, path::PathBuf, time::Instant};
fn main() {
    let candidates: Vec<Runner> = serde_json::from_str(include_str!("../default-runners.json"))
        .expect("Candidate runner config");
    let dir = core::meetings_dir().join("1800000000000-1");
    let filter = std::env::args().nth(1);
    let mut results = Vec::new();
    for runner in candidates {
        if filter.as_ref().is_some_and(|f| !runner.name.contains(f)) {
            continue;
        }
        let tools = RefCell::new(Vec::new());
        let started = Instant::now();
        let prompt = if runner.tools {
            "Use a tool to list the filenames in the current working directory. Do not read file contents, write files, change settings, or call external services. Report the filenames briefly."
        } else {
            "Say only: pong"
        };
        let result = core::run_runner(&runner, prompt.into(), &PathBuf::from(&dir), |event| {
            if let AskProgress::Tool { name } = event {
                tools.borrow_mut().push(name);
            }
        });
        let ok = result.as_ref().is_ok_and(|s| {
            if runner.tools {
                !s.trim().is_empty() && !tools.borrow().is_empty()
            } else {
                s.trim().eq_ignore_ascii_case("pong")
            }
        });
        let entry = serde_json::json!({"name":runner.name,"success":ok,"seconds":started.elapsed().as_secs_f64(),"tools":tools.into_inner(),"answer_chars":result.as_ref().map(|s|s.chars().count()).unwrap_or(0),"error":result.err()});
        println!("{entry}");
        results.push(entry);
    }
    let suffix = filter.unwrap_or_else(|| "all".into()).replace(' ', "-");
    std::fs::write(
        format!("artifacts/runner-probe-{suffix}.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
