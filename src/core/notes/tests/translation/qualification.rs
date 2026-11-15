use super::*;

#[test]
fn unqualified_provider_never_receives_transcript() {
    for case in [
        "identity",
        "binary",
        "invocation",
        "version",
        "tools",
        "isolation",
        "retention",
    ] {
        let (root, folder, settings) = fixture();
        let mut config = crate::core::Config::load_in(&root.0).unwrap();
        config.engine_settings.stages.translation_enabled_at_ms = Some(1);
        let model = match case {
            "identity" => "unqualified-fixture",
            "binary" => "translation-stale-binary",
            "invocation" => "translation-stale-invocation",
            _ => "translation-fixture",
        };
        config.engine_settings.stages.translation_model = model.into();
        config.save(&root.0).unwrap();
        fs::write(root.0.join("capability-case"), case).unwrap();
        enqueue(&root.0, &settings, None);
        process(&root.0, &settings);
        assert!(!root.0.join("translation-input").exists());
        if ["identity", "binary", "invocation"].contains(&case) {
            assert!(!root.0.join("capability-input").exists());
        }
        let job = queue::read(&root.0, ID).unwrap().unwrap();
        assert!(matches!(job.state, JobState::Done { .. }));
        assert_eq!(job.helper, Some(Helper::Claude));
        assert_eq!(job.translation.unwrap().reason, "helper_unqualified");
        let selection = transcript::chosen(
            &root.0,
            ID,
            &folder,
            job.transcript_revision.as_deref(),
            0,
            1,
        )
        .unwrap()
        .unwrap();
        assert!(
            fs::read_to_string(root.0.join("notes-input"))
                .unwrap()
                .contains(&selection.render())
        );
        let status = crate::core::notes::status_record(&root.0, ID).unwrap();
        assert!(!status.translation_message().is_empty());
        if !["identity", "binary", "invocation"].contains(&case) {
            let sent = fs::read_to_string(root.0.join("capability-input")).unwrap();
            assert!(!sent.contains("selected") && sent.contains(model));
        }
    }
}

pub(super) fn capability_child(root: &Path, input: &str) -> ! {
    fs::write(root.join("capability-input"), input).unwrap();
    let request: serde_json::Value = serde_json::from_str(input).unwrap();
    let case = fs::read_to_string(root.join("capability-case")).unwrap_or_default();
    let mut cap = serde_json::json!({"version":"native-fixture-v1","model":request["model"],
        "tools":[],"isolated":true,"retains_content":false});
    match case.as_str() {
        "version" => cap["version"] = "unknown".into(),
        "tools" => cap["tools"] = serde_json::json!(["read"]),
        "isolation" => cap["isolated"] = false.into(),
        "retention" => cap["retains_content"] = true.into(),
        _ => {}
    }
    println!(
        "{}",
        serde_json::json!({"type":"text","part":{"text":cap.to_string()}})
    );
    std::process::exit(0);
}
