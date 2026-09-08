use arb_adapters::store::Store;
use arb_core::types::*;
use rusqlite::Connection;

#[test]
fn complete_schema_matches_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("migrated.db");
    Store::open(&path).unwrap();
    let migrated = Connection::open(path).unwrap();
    let snapshot = Connection::open_in_memory().unwrap();
    snapshot
        .execute_batch(include_str!("../schema/schema_v015.sql"))
        .unwrap();

    let schema = |connection: &Connection| {
        connection
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|sql| {
                // These schema files use -- line comments and no -- inside SQL literals.
                sql.unwrap()
                    .lines()
                    .map(|line| line.split_once("--").map_or(line, |(sql, _)| sql))
                    .flat_map(str::split_whitespace)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(schema(&snapshot), schema(&migrated));
    let version = |connection: &Connection| {
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap()
    };
    assert_eq!(version(&snapshot), version(&migrated));
}

fn raw(n: u64, source: &str) -> RawRecord {
    RawRecord {
        version: 1,
        chain_id: 4663,
        source: source.into(),
        run_id: "test".into(),
        sequence: n,
        received_at_ms: n,
        request_elapsed_ns: None,
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

#[test]
fn integer_block_migration_preserves_data_and_rejects_overflow() {
    for height in [
        "10",
        "9223372036854775807",
        "9223372036854775808",
        "bad",
        "-1",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let mut sql = Connection::open(&path).unwrap();
        sql.execute_batch(include_str!("../schema/schema_v012.sql"))
            .unwrap();
        sql.execute("INSERT INTO raw_records(id,chain_id,source,run_id,sequence,data,block_number) VALUES(7,'4663','rpc','test','0',X'7B7D',?1)", [height]).unwrap();
        sql.execute("INSERT INTO ingest_gaps(chain_id,source,block_number,reason) VALUES('4663','rpc',?1,'missing')", [height]).unwrap();
        sql.execute("INSERT INTO derived_blocks(id,run_id,block_hash,block_number,data) VALUES(9,'test','hash',?1,X'7B7D')", [height]).unwrap();
        sql.execute_batch(
            "UPDATE sqlite_sequence SET seq=100 WHERE name IN ('raw_records','derived_blocks');",
        )
        .unwrap();
        // Isolate the v13 type conversion; payload validity is covered by v15 migration tests.
        let upgraded = {
            let tx = sql.transaction().unwrap();
            match tx.execute_batch(include_str!(
                "../migrations/schema_v013_use_integer_block_numbers.sql"
            )) {
                Ok(()) => tx.commit(),
                Err(error) => Err(error),
            }
        };
        if let Ok(expected) = height.parse::<i64>()
            && expected >= 0
        {
            upgraded.unwrap();
            for table in ["raw_records", "ingest_gaps", "derived_blocks"] {
                let (value, kind): (i64, String) = sql
                    .query_row(
                        &format!("SELECT block_number,typeof(block_number) FROM {table}"),
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!((value, kind), (expected, "integer".into()));
            }
            assert_eq!(
                sql.query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='raw_records'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                100
            );
            assert_eq!(
                sql.query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='derived_blocks'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                100
            );
            assert_eq!(
                sql.query_row("SELECT hex(data) FROM raw_records WHERE id=7", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "7B7D"
            );
        } else {
            assert!(upgraded.is_err());
            assert_eq!(
                sql.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                12
            );
            assert_eq!(
                sql.query_row("SELECT block_number FROM raw_records", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                height
            );
        }
    }
}

#[test]
fn cursor_columns_migrate_and_validate_progress() {
    for next in [
        serde_json::json!(42),
        serde_json::json!(i64::MAX),
        serde_json::json!(u64::MAX),
        serde_json::json!(-1),
        serde_json::json!("42"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v13.db");
        let sql = Connection::open(&path).unwrap();
        sql.execute_batch(include_str!("../schema/schema_v013.sql"))
            .unwrap();
        let hash = format!("0x{}", "ab".repeat(32));
        let data = serde_json::to_vec(&serde_json::json!({"chain_id":4663,"source":"rpc","next_block":next,"last_block_hash":hash})).unwrap();
        sql.execute(
            "INSERT INTO source_cursors VALUES('4663','rpc',?1)",
            [&data],
        )
        .unwrap();
        let empty = SourceCursor {
            chain_id: 4663,
            source: "feed".into(),
            next_block: 0,
            last_block_hash: None,
        };
        sql.execute(
            "INSERT INTO source_cursors VALUES('4663','feed',?1)",
            [serde_json::to_vec(&empty).unwrap()],
        )
        .unwrap();
        let opened = Store::open(&path);
        if let Some(n) = next.as_i64().filter(|n| *n >= 0) {
            let mut store = opened.unwrap();
            assert_eq!(store.cursor(4663, "feed").unwrap(), Some(empty));
            let cursor = store.cursor(4663, "rpc").unwrap().unwrap();
            assert_eq!(cursor.next_block, n as u64);
            assert_eq!(cursor.last_block_hash.unwrap().to_string(), hash);
            let (stored, stored_hash): (i64, String) = sql
                .query_row(
                    "SELECT next_block,last_block_hash FROM source_cursors WHERE source='rpc'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!((stored, stored_hash), (n, hash));
            assert!(sql.prepare("SELECT data FROM source_cursors").is_err());
            let overflow = SourceCursor {
                next_block: u64::MAX,
                ..cursor.clone()
            };
            assert!(store.append_raw(&[raw(1, "rpc")], &overflow).is_err());
            assert_eq!(store.raw_count().unwrap(), 0);
            assert_eq!(store.cursor(4663, "rpc").unwrap(), Some(cursor.clone()));
            let cleared = SourceCursor {
                last_block_hash: None,
                ..cursor
            };
            store.append_raw(&[raw(1, "rpc")], &cleared).unwrap();
            assert_eq!(store.cursor(4663, "rpc").unwrap(), Some(cleared));
        } else {
            assert!(opened.is_err());
            assert_eq!(
                sql.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                13
            );
            assert_eq!(
                sql.query_row(
                    "SELECT data FROM source_cursors WHERE source='rpc'",
                    [],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
                data
            );
        }
    }
}
