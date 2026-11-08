use hex_meetings::core::{self, AskProgress, Message};
fn main() {
    let runners = core::list_runners().expect("Runner configuration");
    println!("Configured {} verified runners", runners.len());
    let history=vec![Message{role:"user".into(),text:"Use a tool to list only the filenames in this folder. Do not read contents, change files, or contact an external service.".into(), ..Default::default()}];
    let receiver = core::ask::spawn(
        "1800000000000-1".into(),
        history,
        "Claude Sonnet".into(),
        core::ask::Cancel::new(),
        |event| match event {
            AskProgress::Tool { name } => println!("Tool: {name}"),
            AskProgress::Started { runner } => println!("Runner: {runner}"),
            _ => {}
        },
    );
    let answer = receiver
        .recv()
        .expect("Ask worker exited")
        .expect("Ask failed");
    assert!(!answer.trim().is_empty());
    assert_eq!(
        core::last_used_runner().unwrap().as_deref(),
        Some("Claude Sonnet")
    );
    println!(
        "Core Ask completed, {} answer characters; last runner reloaded",
        answer.chars().count()
    );
}
