use super::*;

pub(super) fn fixture() -> MemoryStore {
    fixture_at(Local::now().to_rfc3339())
}

fn fixture_at(stamp: String) -> MemoryStore {
    let root = fixture_root(Local::now().timestamp_nanos_opt().unwrap());
    let store = MemoryStore::new(root);
    super::super::recording_guard::keep_in(&store.root, "1800000000000-1").unwrap();
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
fn plain_accept_refuses_a_commented_proposal() {
    let store = fixture();
    let id = store.load().unwrap().proposals[0].proposal.id.clone();
    store.comment_proposal(&id, "Friday afternoon.").unwrap();
    assert!(store.accept_proposal(&id).is_err());
    assert_eq!(
        store.load().unwrap().proposals[0].status,
        ProposalStatus::Pending
    );
}

#[test]
fn plain_accept_refuses_a_source_hidden_since_the_card_loaded() {
    let store = fixture();
    let shown = store.list_pending_proposals().unwrap().remove(0);
    let receipt = store
        .root
        .join("local/recording-guard/1800000000000-1.json");
    fs::create_dir_all(receipt.parent().unwrap()).unwrap();
    fs::write(receipt, "unreadable synthetic receipt").unwrap();
    let before = serde_json::to_vec(&store.load().unwrap()).unwrap();
    assert!(store.accept_proposal(&shown.proposal.id).is_err());
    assert_eq!(serde_json::to_vec(&store.load().unwrap()).unwrap(), before);
}

#[test]
fn revision_preserves_the_first_reply_for_the_current_pending_comment() {
    let store = fixture();
    let id = store.load().unwrap().proposals[0].proposal.id.clone();
    let first = store
        .comment_proposal(&id, "Friday morning.")
        .unwrap()
        .unwrap();
    let current = store
        .comment_proposal(&id, "Friday afternoon.")
        .unwrap()
        .unwrap();
    assert!(
        !store
            .save_revision(&first, "The demo is Friday morning.")
            .unwrap()
    );
    assert!(
        store
            .save_revision(&current, "The demo is Friday afternoon.")
            .unwrap()
    );
    assert!(
        !store
            .save_revision(&current, "The demo is at 3 pm.")
            .unwrap()
    );
    assert_eq!(
        store.load().unwrap().proposals[0].revision_text(),
        Some("The demo is Friday afternoon.")
    );
    store.reject_proposal(&id).unwrap();
    assert!(
        !store
            .save_revision(&current, "The demo is at 4 pm.")
            .unwrap()
    );
}

#[test]
fn accept_binds_the_choice_to_the_wording_on_the_card() {
    for choice in [AcceptChoice::Shown, AcceptChoice::Original] {
        let store = fixture();
        let id = store.load().unwrap().proposals[0].proposal.id.clone();
        let request = store
            .comment_proposal(&id, "Friday afternoon.")
            .unwrap()
            .unwrap();
        let original = store.list_pending_proposals().unwrap().remove(0);
        assert!(store.accept_shown(&original, AcceptChoice::Shown).is_err());
        store
            .save_revision(&request, "The demo is Friday afternoon.")
            .unwrap();
        assert!(store.accept_shown(&original, AcceptChoice::Shown).is_err());
        let revised = store.list_pending_proposals().unwrap().remove(0);
        let (shown, wording) = match choice {
            AcceptChoice::Shown => (&revised, "The demo is Friday afternoon."),
            AcceptChoice::Original => (&original, "The demo is Friday."),
        };
        store.accept_shown(shown, choice).unwrap();
        assert!(
            store
                .load()
                .unwrap()
                .records
                .iter()
                .any(|r| r.text.as_deref() == Some(wording))
        );
    }
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
        restarted
            .accept_shown(&loaded.proposals[0], AcceptChoice::Original)
            .unwrap();
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
    let shown = store.load().unwrap().proposals.remove(0);
    store.accept_shown(&shown, AcceptChoice::Original).unwrap();
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

fn stage_add(store: &MemoryStore) -> String {
    let mut proposal = store.load().unwrap().proposals[0].proposal.clone();
    proposal.op = ProposalOp::Add;
    proposal.target_id = None;
    proposal.target_updated_at = None;
    proposal.id = proposal_id(&proposal);
    store
        .stage_proposals(std::slice::from_ref(&proposal))
        .unwrap();
    proposal.id
}

fn shown_proposal(store: &MemoryStore, id: &str) -> PendingProposal {
    store
        .list_pending_proposals()
        .unwrap()
        .into_iter()
        .find(|p| p.proposal.id == id)
        .unwrap()
}

#[test]
fn decision_history_keeps_visible_wording_until_the_record_is_forgotten() {
    let store = fixture();
    let id = stage_add(&store);
    let request = store
        .comment_proposal(&id, "Friday afternoon.")
        .unwrap()
        .unwrap();
    let accepted_text = "The demo is Friday afternoon.";
    store.save_revision(&request, accepted_text).unwrap();
    store
        .accept_shown(&shown_proposal(&store, &id), AcceptChoice::Shown)
        .unwrap();
    let record_id = format!("memory-{id}");
    let file = store.load().unwrap();
    let record = file.records.iter().find(|r| r.id == record_id).unwrap();
    let mut confirm = file
        .proposals
        .iter()
        .find(|p| p.proposal.id == id)
        .unwrap()
        .proposal
        .clone();
    confirm.op = ProposalOp::Confirm;
    confirm.target_id = Some(record_id.clone());
    confirm.target_updated_at = Some(record.updated_at.clone());
    confirm.sources = vec![MemorySource::Meeting {
        reference: "[[1800000000001-1@00:05]]".into(),
    }];
    confirm.id = proposal_id(&confirm);
    super::super::recording_guard::keep_in(&store.root, "1800000000001-1").unwrap();
    store
        .stage_proposals(std::slice::from_ref(&confirm))
        .unwrap();
    assert!(
        store
            .comment_proposal(&confirm.id, "Keep this as a note.")
            .unwrap()
            .is_none()
    );
    store
        .accept_shown(&shown_proposal(&store, &confirm.id), AcceptChoice::Shown)
        .unwrap();
    fs::write(
        store
            .root
            .join("local/recording-guard/1800000000001-1.json"),
        "unreadable receipt",
    )
    .unwrap();
    let file = store.load().unwrap();
    assert!(
        store
            .visible_file(file.clone())
            .records
            .iter()
            .all(|r| r.id != record_id)
    );
    let rows = store.history_rows(&file);
    assert!(rows.iter().any(|row| row.record_id == record_id
        && row.comment_label == Some("Your comment was used")
        && row.decision.as_ref().and_then(|d| d.text.as_deref()) == Some(accepted_text)));
    assert_eq!(
        rows.iter()
            .filter(|r| r.record_id == record_id && r.decision.is_some())
            .count(),
        1
    );
    store.forget_record(&record_id).unwrap();
    let file = store.load().unwrap();
    assert!(
        file.events
            .iter()
            .filter(|e| e.record_id == record_id)
            .all(|e| e.decision.is_none())
    );
    assert!(
        store
            .history_rows(&file)
            .iter()
            .filter(|r| r.record_id == record_id)
            .all(|r| r.decision.is_none())
    );
}

#[test]
fn legacy_comment_appears_until_the_owner_confirms_the_record() {
    let store = fixture();
    let id = stage_add(&store);
    let comment = "Trying to keep Friday afternoon.";
    store.comment_proposal(&id, comment).unwrap();
    store
        .accept_shown(&shown_proposal(&store, &id), AcceptChoice::Original)
        .unwrap();
    let record = format!("memory-{id}");
    let before = store.load().unwrap();
    let mut legacy = before.clone();
    legacy
        .events
        .iter_mut()
        .find(|e| e.record_id == record)
        .unwrap()
        .decision = None;
    store.save(&before, &legacy).unwrap();
    let file = store.load().unwrap();
    assert!(store.history_rows(&file).iter().any(
        |row| row.record_id == record && row.comment_label == Some("Your comment was not used")
    ));
    assert_eq!(
        store.comments_not_used(&file),
        [(record.clone(), comment.into())]
    );
    store.confirm_record(&record).unwrap();
    assert!(store.comments_not_used(&store.load().unwrap()).is_empty());
}
