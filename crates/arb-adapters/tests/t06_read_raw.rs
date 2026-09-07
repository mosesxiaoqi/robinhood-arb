use arb_adapters::store::Store;
use arb_core::types::*;
use rusqlite::{Connection, params};
#[test]
fn paginate_without_skip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("read.db");
    let mut store = Store::open(&path).unwrap();
    for n in 1..=3 {
        let source = if n == 3 { "feed" } else { "rpc" };
        let raw = RawRecord {
            version: 1,
            chain_id: 4663,
            source: source.into(),
            run_id: "test".into(),
            sequence: n,
            received_at_ms: 10,
            request_elapsed_ns: None,
            kind: "message".into(),
            position: None,
            transaction_hash: None,
            execution_status: ExecutionStatus::Unknown,
            confirmation: Confirmation::Unknown,
            payload: vec![1],
        };
        store
            .append_raw(
                &[raw],
                &SourceCursor {
                    chain_id: 4663,
                    source: source.into(),
                    next_block: n,
                    last_block_hash: None,
                },
            )
            .unwrap();
    }
    let first = store.read_raw_after(0, 2).unwrap();
    assert_eq!(first.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2]);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.read_raw_after(2, 2).unwrap()[0].id, 3);
    assert_eq!(
        store
            .read_raw_filtered_after(0, 10, Some(4663), Some("feed"))
            .unwrap()[0]
            .id,
        3
    );
    assert!(store.read_raw_after(0, 1001).is_err());
    let mut value = serde_json::to_value(&first[0].record).unwrap();
    value["version"] = 2.into();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE raw_records SET data=?1 WHERE id=1",
            params![serde_json::to_vec(&value).unwrap()],
        )
        .unwrap();
    assert!(store.read_raw_after(0, 2).is_err());
}
