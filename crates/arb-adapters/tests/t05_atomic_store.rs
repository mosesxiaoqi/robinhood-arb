use arb_adapters::store::Store;
use arb_core::types::*;
use rusqlite::Connection;
fn raw(n: u64, source: &str) -> RawRecord {
    RawRecord {
        version: 1,
        chain_id: 4663,
        source: source.into(),
        run_id: "test".into(),
        sequence: n,
        received_at_ms: n,
        kind: "block".into(),
        position: None,
        transaction_hash: None,
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Unknown,
        payload: vec![n as u8],
    }
}
#[test]
fn rollback_record_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("raw.db");
    let mut store = Store::open(&path).unwrap();
    let first = SourceCursor {
        chain_id: 4663,
        source: "rpc".into(),
        next_block: 1,
        last_block_hash: None,
    };
    store.append_raw(&[raw(0, "rpc")], &first).unwrap();
    assert_eq!(store.cursor(4663, "rpc").unwrap(), Some(first.clone()));
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_three BEFORE INSERT ON raw_records WHEN NEW.sequence='3' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    let second = SourceCursor {
        next_block: 4,
        ..first.clone()
    };
    assert!(
        store
            .append_raw(&[raw(2, "rpc"), raw(3, "rpc")], &second)
            .is_err()
    );
    assert_eq!(store.raw_count().unwrap(), 1);
    assert_eq!(store.cursor(4663, "rpc").unwrap(), Some(first.clone()));
    store.append_raw(&[raw(0, "rpc")], &first).unwrap();
    assert_eq!(store.raw_count().unwrap(), 1);
    let feed = SourceCursor {
        source: "feed".into(),
        ..first.clone()
    };
    store.append_raw(&[raw(0, "feed")], &feed).unwrap();
    assert_eq!(store.raw_count().unwrap(), 2);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.cursor(4663, "rpc").unwrap(), Some(first));
    sql.pragma_update(None, "user_version", 999).unwrap();
    assert!(Store::open(&path).is_err());
}
