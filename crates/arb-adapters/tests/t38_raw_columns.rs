use arb_adapters::store::Store;
use arb_core::types::*;
use rusqlite::Connection;

#[test]
fn scalar_raw_roundtrip_and_collision_are_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("raw.db");
    let mut store = Store::open(&path).unwrap();
    let cursor = SourceCursor {
        chain_id: u64::MAX,
        source: "rpc".into(),
        next_block: 8,
        last_block_hash: None,
    };
    let mut record = RawRecord {
        version: 1,
        chain_id: u64::MAX,
        source: "rpc".into(),
        run_id: "columns".into(),
        sequence: u64::MAX,
        received_at_ms: i64::MAX as u64,
        request_elapsed_ns: Some(i64::MAX as u64),
        kind: "receipt".into(),
        position: Some(ChainPosition {
            block_number: 7,
            block_hash: [7; 32].into(),
            offset: Offset::Transaction {
                index: 3,
                log_index: Some(2),
            },
        }),
        transaction_hash: Some([8; 32].into()),
        execution_status: ExecutionStatus::Succeeded,
        confirmation: Confirmation::Finalized,
        payload: vec![0, 255, 128, b'{'],
    };
    for (sequence, offset, status, confirmation) in [
        (
            u64::MAX,
            Offset::Transaction {
                index: 3,
                log_index: Some(2),
            },
            ExecutionStatus::Succeeded,
            Confirmation::Finalized,
        ),
        (
            1,
            Offset::Transaction {
                index: 4,
                log_index: None,
            },
            ExecutionStatus::Reverted,
            Confirmation::Safe,
        ),
        (
            2,
            Offset::BlockEnd,
            ExecutionStatus::Unknown,
            Confirmation::Included,
        ),
    ] {
        record.sequence = sequence;
        record.position.as_mut().unwrap().offset = offset;
        record.execution_status = status;
        record.confirmation = confirmation;
        store
            .append_raw(std::slice::from_ref(&record), &cursor)
            .unwrap();
        assert_eq!(
            store
                .read_raw_block(u64::MAX, 7, 0)
                .unwrap()
                .last()
                .unwrap()
                .record,
            record
        );
    }
    record.sequence = 3;
    record.position = None;
    record.transaction_hash = None;
    record.confirmation = Confirmation::Unknown;
    record.request_elapsed_ns = None;
    store
        .append_raw(std::slice::from_ref(&record), &cursor)
        .unwrap();
    store
        .append_raw(std::slice::from_ref(&record), &cursor)
        .unwrap();
    assert_eq!(store.raw_count().unwrap(), 4);
    assert_eq!(store.read_raw_after(3, 1).unwrap()[0].record, record);
    record.payload.push(1);
    assert!(
        store
            .append_raw(std::slice::from_ref(&record), &cursor)
            .is_err()
    );
    record.sequence = 4;
    record.received_at_ms = u64::MAX;
    assert!(store.append_raw(&[record], &cursor).is_err());
    assert_eq!(store.raw_count().unwrap(), 4);
    let sql = Connection::open(path).unwrap();
    let values: (String, String, Vec<u8>) = sql
        .query_row(
            "SELECT execution_status,confirmation,payload FROM raw_records WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        values,
        (
            "succeeded".into(),
            "finalized".into(),
            vec![0, 255, 128, b'{']
        )
    );
    assert!(sql.prepare("SELECT data FROM raw_records").is_err());
    let duplicate = store.read_raw_after(0, 1).unwrap().remove(0).record;
    sql.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE raw_records SET payload=zeroblob(67108865) WHERE id=1;").unwrap();
    let error = store.append_raw(&[duplicate], &cursor).unwrap_err();
    assert!(error.to_string().contains("read budget"));
}

#[test]
fn populated_v14_raw_migrates_losslessly_or_rolls_back() {
    for corruption in [None, Some("json"), Some("identity"), Some("oversized")] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v14.db");
        let sql = Connection::open(&path).unwrap();
        sql.execute_batch(include_str!("../schema/schema_v014.sql"))
            .unwrap();
        let positioned = RawRecord {
            version: 1,
            chain_id: u64::MAX,
            source: "rpc".into(),
            run_id: "legacy".into(),
            sequence: u64::MAX,
            received_at_ms: 123,
            request_elapsed_ns: Some(456),
            kind: "receipt".into(),
            position: Some(ChainPosition {
                block_number: 7,
                block_hash: [7; 32].into(),
                offset: Offset::Transaction {
                    index: 3,
                    log_index: Some(2),
                },
            }),
            transaction_hash: Some([8; 32].into()),
            execution_status: ExecutionStatus::Reverted,
            confirmation: Confirmation::Safe,
            payload: vec![0, 255, 128, b'{'],
        };
        let unpositioned = RawRecord {
            sequence: 0,
            position: None,
            transaction_hash: None,
            execution_status: ExecutionStatus::Unknown,
            confirmation: Confirmation::Unknown,
            request_elapsed_ns: None,
            ..positioned.clone()
        };
        for (id, record) in [(7, &positioned), (8, &unpositioned)] {
            sql.execute(
                "INSERT INTO raw_records(id,chain_id,source,run_id,sequence,block_number,data) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![id,record.chain_id.to_string(),record.source,record.run_id,record.sequence.to_string(),record.position.as_ref().map(|p| p.block_number as i64),serde_json::to_vec(record).unwrap()],
            ).unwrap();
        }
        sql.execute_batch("UPDATE sqlite_sequence SET seq=100 WHERE name='raw_records'")
            .unwrap();
        match corruption {
            Some("json") => {
                sql.execute("UPDATE raw_records SET data=X'7B' WHERE id=8", [])
                    .unwrap();
            }
            Some("oversized") => {
                sql.execute(
                    "UPDATE raw_records SET data=zeroblob(67108865) WHERE id=8",
                    [],
                )
                .unwrap();
            }
            Some("identity") => {
                sql.execute("UPDATE raw_records SET sequence='1' WHERE id=8", [])
                    .unwrap();
            }
            _ => {}
        }
        let legacy_data: (i64, Vec<u8>) = sql
            .query_row(
                "SELECT length(data),substr(data,1,1024) FROM raw_records WHERE id=8",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(sql);
        let opened = Store::open(&path);
        if corruption.is_some() {
            assert!(opened.is_err());
            let sql = Connection::open(&path).unwrap();
            assert_eq!(
                sql.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
                    .unwrap(),
                14
            );
            assert_eq!(
                sql.query_row::<(i64, Vec<u8>), _, _>(
                    "SELECT length(data),substr(data,1,1024) FROM raw_records WHERE id=8",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?))
                )
                .unwrap(),
                legacy_data
            );
            assert_eq!(
                sql.query_row::<i64, _, _>("SELECT count(*) FROM raw_records", [], |r| r.get(0))
                    .unwrap(),
                2
            );
            assert!(sql.prepare("SELECT payload FROM raw_records").is_err());
            assert!(sql.prepare("SELECT * FROM raw_records_v014").is_err());
            continue;
        }
        let mut store = opened.unwrap();
        let rows = store.read_raw_after(0, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].id, &rows[0].record), (7, &positioned));
        assert_eq!((rows[1].id, &rows[1].record), (8, &unpositioned));
        let new_record = RawRecord {
            sequence: 1,
            ..unpositioned
        };
        store
            .append_raw(
                &[new_record],
                &SourceCursor {
                    chain_id: u64::MAX,
                    source: "rpc".into(),
                    next_block: 8,
                    last_block_hash: None,
                },
            )
            .unwrap();
        assert_eq!(store.read_raw_after(8, 1).unwrap()[0].id, 101);
        let sql = Connection::open(&path).unwrap();
        assert_eq!(
            sql.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            15
        );
        let columns: (String, i64, i64, Vec<u8>) = sql.query_row(
            "SELECT offset_kind,transaction_index,log_index,payload FROM raw_records WHERE id=7", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        assert_eq!(columns, ("transaction".into(), 3, 2, positioned.payload));
        assert!(sql.prepare("SELECT data FROM raw_records").is_err());
        assert!(sql.prepare("SELECT * FROM raw_records_v014").is_err());
    }
}

#[test]
fn legacy_json_expansion_does_not_reject_a_valid_payload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large-v14.db");
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(include_str!("../schema/schema_v014.sql"))
        .unwrap();
    let record = RawRecord {
        version: 1,
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "legacy-large".into(),
        sequence: 0,
        received_at_ms: 123,
        request_elapsed_ns: None,
        kind: "response".into(),
        position: None,
        transaction_hash: None,
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Unknown,
        payload: vec![255; 17 * 1024 * 1024],
    };
    let data = serde_json::to_vec(&record).unwrap();
    assert!(data.len() > 64 * 1024 * 1024);
    sql.execute(
        "INSERT INTO raw_records(id,chain_id,source,run_id,sequence,data) VALUES(7,'4663','rpc','legacy-large','0',?1)",
        [data],
    ).unwrap();
    drop(sql);
    let store = Store::open(&path).unwrap();
    let migrated = store.read_raw_after(0, 1).unwrap().remove(0);
    assert_eq!(migrated.id, 7);
    assert_eq!(migrated.record, record);
}
