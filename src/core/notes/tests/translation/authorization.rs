use super::*;

#[test]
fn experimental_switch_suppresses_previously_authorized_translation() {
    let (root, folder, settings) = fixture();
    let mut config = crate::core::Config::load_in(&root.0).unwrap();
    config.engine_settings.stages.translation_enabled_at_ms = Some(1);
    config.engine_settings.stages.translation_model = "translation-fixture".into();
    config.features.translation = false;
    config.save(&root.0).unwrap();
    enqueue(&root.0, &settings, None);
    process(&root.0, &settings);
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    assert_eq!(job.translation.unwrap().reason, "translation_off");
    assert!(!root.0.join("capability-input").exists());
    assert!(!root.0.join("translation-input").exists());
    assert!(!folder.join("english-transcript.md").exists());
}

#[test]
fn queue_revocation_after_preparation_prevents_spawn() {
    let (root, folder, settings) = fixture();
    let mut config = crate::core::Config::load_in(&root.0).unwrap();
    config.engine_settings.stages.translation_enabled_at_ms = Some(1);
    config.engine_settings.stages.translation_model = "translation-fixture".into();
    config.save(&root.0).unwrap();
    fs::write(root.0.join("withdraw-before-spawn"), "").unwrap();
    enqueue(&root.0, &settings, None);
    process(&root.0, &settings);
    assert!(root.0.join("withdrawal-fired").exists());
    assert!(root.0.join("capability-input").exists());
    assert!(!root.0.join("translation-input").exists());
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    assert_eq!(job.helper, Some(Helper::Claude));
    assert_eq!(
        job.translation.unwrap().reason,
        "translation_consent_changed"
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
        crate::core::notes::status_record(&root.0, ID)
            .unwrap()
            .translation_message()
            .contains("permission changed")
    );
}
