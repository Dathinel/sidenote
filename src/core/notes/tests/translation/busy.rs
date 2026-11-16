use super::*;

struct BusyNotes {
    notes: Fake,
    translations: Vec<serde_json::Value>,
}
impl Run for BusyNotes {
    fn installed(&self, helper: Helper) -> bool {
        self.notes.installed(helper)
    }
    fn run(
        &mut self,
        helper: Helper,
        model: &str,
        prompt: String,
        timeout: Duration,
    ) -> Result<String, Failure> {
        self.notes.run(helper, model, prompt, timeout)
    }
    fn translate(
        &mut self,
        snapshot: &super::super::super::translation::Snapshot,
        _: String,
    ) -> Result<String, &'static str> {
        self.translations
            .push(serde_json::to_value(snapshot).unwrap());
        Err("helper_unqualified")
    }
}

#[test]
fn busy_retry_preserves_translation_snapshot_and_notice() {
    for change in [
        "model",
        "helper",
        "location",
        "payment",
        "activation",
        "off",
        "sorting",
    ] {
        let (root, folder, settings) = routing::setup();
        routing::selected(&root.0, &folder);
        let mut config = crate::core::Config::load_in(&root.0).unwrap();
        config.engine_settings.steps.translation = true;
        config.engine_settings.stages.translation_enabled_at_ms = Some(1);
        config.engine_settings.stages.translation_model = "translation-original".into();
        config.engine_settings.translation_prompt = Some("Approved instructions".into());
        config.save(&root.0).unwrap();
        enqueue(&root.0, &settings, None);
        let sorting = change == "sorting";
        let mut run = BusyNotes {
            notes: Fake::new(if sorting {
                vec![Ok(summary()), Err(Failure::Busy)]
            } else {
                vec![Err(Failure::Busy)]
            }),
            translations: vec![],
        };
        queue::drain(&root.0, &root.0.join("meetings"), &mut run, || {
            Ok(settings.clone())
        })
        .unwrap();
        assert_eq!(run.translations.len(), 1, "{change}");
        let first = queue::read(&root.0, ID).unwrap().unwrap();
        let disposition = first
            .translation
            .expect("Busy must retain translation disposition");
        assert_eq!(disposition.reason, "helper_unqualified");
        let snapshot = serde_json::to_value(disposition.snapshot.unwrap()).unwrap();
        assert_eq!(snapshot, run.translations[0]);
        assert_eq!(first.sorting_pending, sorting);
        let summary_bytes = fs::read(folder.join("english-summary.json")).ok();
        match change {
            "helper" => config.engine_settings.stages.translation_helper = Helper::Codex,
            "location" => config.engine_settings.stages.translation_location = "local_only".into(),
            "payment" => config.engine_settings.stages.translation_payment = "no_paid".into(),
            "activation" => config.engine_settings.stages.translation_enabled_at_ms = Some(2),
            "off" => config.engine_settings.steps.translation = false,
            _ => {
                config.engine_settings.stages.translation_model = "translation-edited".into();
                config.engine_settings.stages.translation_deadline_seconds = 23;
                config.engine_settings.translation_prompt = Some("Edited instructions".into());
            }
        }
        config.save(&root.0).unwrap();
        run.notes = Fake::new(if sorting {
            vec![Ok(sorted())]
        } else {
            vec![Ok(summary()), Ok(sorted())]
        });
        queue::drain(&root.0, &root.0.join("meetings"), &mut run, || {
            Ok(settings.clone())
        })
        .unwrap();
        let again = queue::read(&root.0, ID).unwrap().unwrap();
        assert!(matches!(again.state, JobState::Done { .. }), "{change}");
        let disposition = again.translation.unwrap();
        assert_eq!(
            serde_json::to_value(disposition.snapshot.unwrap()).unwrap(),
            snapshot,
            "{change}"
        );
        let expected = match change {
            "model" | "sorting" => "helper_unqualified",
            "off" => "translation_off",
            _ => "translation_consent_changed",
        };
        assert_eq!(disposition.reason, expected, "{change}");
        assert_eq!(
            run.translations.len(),
            if change == "model" { 2 } else { 1 },
            "{change}"
        );
        assert!(run.translations.iter().all(|s| s == &snapshot));
        if sorting {
            assert_eq!(
                fs::read(folder.join("english-summary.json")).unwrap(),
                summary_bytes.unwrap()
            );
        } else {
            assert!(run.notes.calls[0].0.contains("selected revision"));
        }
    }
}
