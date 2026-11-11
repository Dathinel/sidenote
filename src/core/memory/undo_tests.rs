use super::*;

fn fixture() -> MemoryStore {
    fixture_at(Local::now().to_rfc3339())
}

fn fixture_at(stamp: String) -> MemoryStore {
    let root = fixture_root(Local::now().timestamp_nanos_opt().unwrap());
    let store = MemoryStore::new(root);
    let source = MemorySource::Meeting {
        reference: "[[1800000000000-1@00:05]]".into(),
    };
    let record = MemoryRecord {
        assignment: None,
        id: "demo-date".into(),
        record_type: RecordType::Decision,
        text: Some("The demo is Thursday.".into()),
        project: None,
        people: vec![],
        sources: vec![source.clone()],
        first_seen: Some(stamp.clone()),
        last_confirmed: Some(stamp.clone()),
        status: RecordStatus::Active,
        basis: Basis::SourceSupported,
        supersedes: None,
        updated_at: stamp.clone(),
        source_fingerprint: source_fingerprint(std::slice::from_ref(&source)),
        source_fingerprints: vec![],
    };
    let file = MemoryFile {
        records: vec![record],
        migration: Some(MigrationInfo {
            version: 1,
            source_path: "fixture".into(),
            fingerprint: "fixture".into(),
            accepted_at: stamp,
        }),
        ..Default::default()
    };
    store.save(&MemoryFile::default(), &file).unwrap();
    let mut proposal = MemoryProposal {
        id: String::new(),
        op: ProposalOp::Supersede,
        target_id: Some("demo-date".into()),
        text: "The demo is Friday.".into(),
        record_type: RecordType::Decision,
        project: None,
        people: vec![],
        sources: vec![source],
        rationale: "The latest meeting changed the date.".into(),
        target_updated_at: Some(file.records[0].updated_at.clone()),
    };
    proposal.id = proposal_id(&proposal);
    store.stage_proposals(&[proposal]).unwrap();
    store
}

fn fixture_root(tick: i64) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sidenote-memory-undo-{}-{tick}-{sequence}",
        std::process::id()
    ))
}

#[test]
fn fixture_roots_are_distinct_within_the_same_clock_tick() {
    assert_ne!(
        fixture_root(1794386244294324100),
        fixture_root(1794386244294324100)
    );
}

#[test]
fn proposal_acceptance_compares_timestamp_instants_after_comment_and_restart() {
    let stamp = "2026-11-11T12:00:00.123456700+05:30";
    for equivalent in [
        "2026-11-11T12:00:00.1234567+05:30",
        "2026-11-11T06:30:00.123456700Z",
    ] {
        let store = fixture_at(stamp.into());
        let before = store.load().unwrap();
        assert_eq!(before.records[0].updated_at, stamp);
        assert_eq!(
            before.proposals[0].proposal.target_updated_at.as_deref(),
            Some(stamp)
        );
        let id = before.proposals[0].proposal.id.clone();
        let mut file = before.clone();
        file.records[0].updated_at = equivalent.into();
        store.save(&before, &file).unwrap();
        store
            .comment_proposal(&id, "Keep the venue booking.")
            .unwrap();
        let restarted = MemoryStore::new(store.root.clone());
        let loaded = restarted.load().unwrap();
        assert_eq!(loaded.records[0].updated_at, equivalent);
        assert_eq!(
            loaded.proposals[0].proposal.target_updated_at.as_deref(),
            Some(stamp)
        );
        restarted.accept_proposal(&id).unwrap();
        let accepted = restarted.load().unwrap();
        assert_eq!(accepted.records[0].status, RecordStatus::Superseded);
        assert_eq!(accepted.proposals[0].status, ProposalStatus::Accepted);
    }
}

#[test]
fn proposal_acceptance_refuses_changed_missing_and_invalid_timestamp_guards() {
    let stamp = "2026-11-11T12:00:00.123456700+05:30";
    for (target, current) in [
        (Some(stamp), "2026-11-11T12:00:00.123456701+05:30"),
        (None, stamp),
        (Some("invalid"), stamp),
        (Some(stamp), "invalid"),
        (Some("invalid"), "invalid"),
    ] {
        let store = fixture_at(stamp.into());
        let before = store.load().unwrap();
        let mut file = before.clone();
        file.records[0].updated_at = current.into();
        file.proposals[0].proposal.target_updated_at = target.map(str::to_owned);
        file.proposals[0].proposal.id = proposal_id(&file.proposals[0].proposal);
        let id = file.proposals[0].proposal.id.clone();
        store.save(&before, &file).unwrap();
        let error = store.accept_proposal(&id).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Record changed since this proposal; review a new proposal"
        );
        let unchanged = store.load().unwrap();
        assert_eq!(
            serde_json::to_value(unchanged).unwrap(),
            serde_json::to_value(file).unwrap()
        );
    }
}

#[test]
fn undo_restores_records_and_pending_review_after_restart_and_refuses_replay() {
    let store = fixture();
    let original = store.load().unwrap();
    let id = original.proposals[0].proposal.id.clone();
    store
        .comment_proposal(&id, "Check the venue booking too.")
        .unwrap();
    store.accept_proposal(&id).unwrap();
    let changed = store.load().unwrap();
    assert_eq!(changed.records.len(), 2);
    assert_eq!(changed.records[0].status, RecordStatus::Superseded);
    let event = changed.events.last().unwrap().clone();
    let restarted = MemoryStore::new(store.root.clone());
    restarted.undo_event(&event.record_id, &event.at).unwrap();
    let restored = restarted.load().unwrap();
    assert_eq!(
        serde_json::to_value(&restored.records).unwrap(),
        serde_json::to_value(&original.records).unwrap()
    );
    assert_eq!(restored.proposals[0].status, ProposalStatus::Pending);
    assert_eq!(
        restored.proposals[0].comment.as_deref(),
        Some("Check the venue booking too.")
    );
    assert!(restarted.undo_event(&event.record_id, &event.at).is_err());
    assert!(!restarted.company_memory().unwrap().contains("Friday"));
}

#[test]
fn undo_refuses_later_edits_and_forgetting_removes_restorable_history() {
    let store = fixture();
    let id = store.load().unwrap().proposals[0].proposal.id.clone();
    store.accept_proposal(&id).unwrap();
    let file = store.load().unwrap();
    let accepted = file.events.last().unwrap().clone();
    let record = format!("memory-{id}");
    store
        .edit_record(&record, "The demo needs a venue.")
        .unwrap();
    assert!(store.undo_event(&accepted.record_id, &accepted.at).is_err());
    assert_eq!(
        store
            .load()
            .unwrap()
            .records
            .iter()
            .find(|r| r.id == record)
            .unwrap()
            .text
            .as_deref(),
        Some("The demo needs a venue.")
    );
    store.forget_record(&record).unwrap();
    let file = store.load().unwrap();
    assert!(file.events.iter().all(|e| e.undo.is_none()));
    assert!(store.undo_event(&accepted.record_id, &accepted.at).is_err());
}

#[test]
fn undo_restores_add_confirm_retire_and_rejected_proposals() {
    for (op, accept) in [
        (ProposalOp::Add, true),
        (ProposalOp::Confirm, true),
        (ProposalOp::Retire, true),
        (ProposalOp::Supersede, false),
    ] {
        let store = fixture();
        let before = store.load().unwrap();
        let mut file = before.clone();
        let mut p = file.proposals[0].proposal.clone();
        p.op = op;
        p.sources = vec![MemorySource::Meeting {
            reference: "[[1800000000001-1@00:05]]".into(),
        }];
        if op == ProposalOp::Add {
            p.target_id = None;
            p.target_updated_at = None;
        }
        p.id = proposal_id(&p);
        file.proposals.clear();
        store.save(&before, &file).unwrap();
        store.stage_proposals(std::slice::from_ref(&p)).unwrap();
        let original = store.load().unwrap();
        if accept {
            store.accept_proposal(&p.id).unwrap();
        } else {
            store.reject_proposal(&p.id).unwrap();
        }
        let event = store.load().unwrap().events.last().unwrap().clone();
        store.undo_event(&event.record_id, &event.at).unwrap();
        let restored = store.load().unwrap();
        assert_eq!(
            serde_json::to_value(&restored.records).unwrap(),
            serde_json::to_value(&original.records).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&restored.proposals).unwrap(),
            serde_json::to_value(&original.proposals).unwrap()
        );
    }
}

#[test]
fn forgetting_clears_rejected_proposal_receipts_too() {
    let store = fixture();
    let id = store.load().unwrap().proposals[0].proposal.id.clone();
    store.reject_proposal(&id).unwrap();
    let event = store.load().unwrap().events.last().unwrap().clone();
    assert!(event.undo.is_some());
    store.forget_record("demo-date").unwrap();
    assert!(
        store
            .load()
            .unwrap()
            .events
            .iter()
            .all(|e| e.undo.is_none())
    );
    assert!(store.undo_event(&event.record_id, &event.at).is_err());
}
