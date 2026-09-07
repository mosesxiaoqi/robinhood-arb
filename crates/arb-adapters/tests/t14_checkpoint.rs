#[path = "../../arb-core/tests/support/mod.rs"]
mod support;
use alloy_primitives::B256;
use arb_adapters::store::Store;
use arb_core::{checkpoint::*, state::*};
#[test]
fn restore_checkpoint_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("checkpoints.db");
    let mut store = Store::open(&path).unwrap();
    let saved = Checkpoint {
        research_run_id: None,
        version: 1,
        state: State::from_bootstrap(support::bootstrap()).unwrap(),
        registry_version: 1,
        config_hash: B256::repeat_byte(7),
        algorithm_version: "v1".into(),
        processing_cursor: ProcessingCursor {
            last_raw_id: 33,
            next_block: 2,
        },
    };
    let id = store.save_checkpoint(&saved).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let restored = store.load_checkpoint(id).unwrap();
    assert_eq!(restored, saved);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_checkpoint BEFORE INSERT ON checkpoints BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
    assert!(store.save_checkpoint(&saved).is_err());
    assert_eq!(store.load_checkpoint(id).unwrap(), saved);
    let count: i64 = sql
        .query_row("SELECT count(*) FROM checkpoints", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let mut unknown = saved.clone();
    unknown.version = 999;
    sql.execute(
        "UPDATE checkpoints SET data=?1 WHERE id=?2",
        rusqlite::params![serde_json::to_vec(&unknown).unwrap(), id as i64],
    )
    .unwrap();
    assert!(store.load_checkpoint(id).is_err());
    let mut wrong = saved.clone();
    wrong.processing_cursor.next_block = 3;
    assert!(wrong.validate().is_err());
    // Replay starts strictly after the saved block; re-applying it cannot mutate state.
    let mut state = saved.state.clone();
    let view = state.view();
    let duplicate = BlockBatch {
        position: view.position.clone(),
        parent_hash: B256::ZERO,
        covered_pools: view.pools.iter().map(|p| p.descriptor.id.clone()).collect(),
        observations: vec![],
        raw_refs: vec![],
    };
    assert!(state.apply_block(&duplicate).is_err());
    assert_eq!(state, saved.state);
}
