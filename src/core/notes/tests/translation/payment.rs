use super::*;

#[test]
fn translation_requires_the_selected_subscription_route() {
    for case in [
        "subscription",
        "metered",
        "unknown",
        "automatic_fallback",
        "wrong_auth",
    ] {
        let (root, folder, settings) = fixture();
        let mut config = crate::core::Config::load_in(&root.0).unwrap();
        config.engine_settings.stages.translation_enabled_at_ms = Some(1);
        config.engine_settings.stages.translation_model = "translation-fixture".into();
        config.save(&root.0).unwrap();
        fs::write(root.0.join("capability-case"), case).unwrap();
        enqueue(&root.0, &settings, None);
        process(&root.0, &settings);
        let allowed = case == "subscription";
        assert_eq!(root.0.join("translation-input").exists(), allowed);
        let job = queue::read(&root.0, ID).unwrap().unwrap();
        assert!(matches!(job.state, JobState::Done { .. }));
        assert_eq!(job.helper, Some(Helper::Claude));
        assert_eq!(
            job.translation.unwrap().reason,
            if allowed {
                "translation_reply_invalid"
            } else {
                "translation_payment_route_unavailable"
            }
        );
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
        assert!(!folder.join("english-transcript.md").exists());
        assert!(
            !crate::core::notes::status_record(&root.0, ID)
                .unwrap()
                .translation_message()
                .is_empty()
        );
        if allowed {
            let record: serde_json::Value =
                store_io::read(&root.0.join("translation-invocation.json")).unwrap();
            let binding = &record["config"]["sidenote_route"];
            assert_eq!(binding["provider"], "fixture");
            assert_eq!(binding["id"], "subscription");
            assert_eq!(binding["auth_mode"], "subscription");
            assert_eq!(binding["automatic_fallback"], false);
            assert_eq!(binding["model"], "translation-fixture");
        }
    }
}
