#[path = "../../arb-core/tests/support/mod.rs"]
mod support;
use alloy_primitives::B256;
use arb_adapters::store::{ReportTable, Store};
use arb_core::{
    research::{DerivedBlock, StageTiming, TimeQuality},
    state::{BlockBatch, StateView},
    types::RawRef,
};
use rusqlite::{Connection, params};

#[test]
fn migrate_research_records_and_keep_scalar_columns_authoritative() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v14.db");
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch(include_str!("../schema/schema_v014.sql"))
        .unwrap();
    let bootstrap = support::bootstrap();
    let refs = vec![RawRef {
        source: "rpc".into(),
        run_id: "capture".into(),
        sequence: 1,
    }];
    let mut block = DerivedBlock {
        canonical: true,
        time_quality: Some(TimeQuality {
            clock_regressions: 2,
            multiple_clock_domains: true,
        }),
        timings: vec![StageTiming {
            run_id: "run".into(),
            stage: "quote".into(),
            recorded_at_ms: 10,
            elapsed_ns: 20,
        }],
        run_id: "run".into(),
        view_id: B256::repeat_byte(4),
        batch: BlockBatch {
            position: bootstrap.position.clone(),
            parent_hash: B256::ZERO,
            covered_pools: vec![bootstrap.pools[0].descriptor.id.clone()],
            observations: vec![],
            raw_refs: refs.clone(),
        },
        view: StateView {
            position: bootstrap.position,
            pools: bootstrap.pools,
            raw_refs: refs,
        },
        candidates: vec![],
        exclusions: vec![],
        excluded_pools: 3,
    };
    sql.execute("INSERT INTO derived_blocks(id,run_id,block_hash,block_number,canonical,data) VALUES(9,'run',?1,1,0,?2)",params![block.view.position.block_hash.to_string(),serde_json::to_vec(&block).unwrap()]).unwrap();
    let plan = serde_json::json!({"id":B256::repeat_byte(8),"run_id":"run","checkpoint_id":7,"common_ancestor":block.view.position,"required_fetch":[2,3],"orphan_hashes":[B256::repeat_byte(9)],"replay_blocks":[block.batch]});
    sql.execute(
        "INSERT INTO recovery_jobs VALUES(?1,'run',?2,'pending')",
        params![
            B256::repeat_byte(8).to_string(),
            serde_json::to_vec(&plan).unwrap()
        ],
    )
    .unwrap();
    let status = serde_json::json!({"run_id":"run","status":"Stopped","started_at_ms":1,"ended_at_ms":2,"collected_blocks":3,"processed_blocks":2,"first_collected":1,"last_collected":3,"disk_start":100,"disk_end":200,"max_processing_ns":1000,"error":null});
    sql.execute(
        "INSERT INTO runtime_status VALUES('run',?1)",
        [serde_json::to_vec(&status).unwrap()],
    )
    .unwrap();
    let mut store = Store::open(&path).unwrap();
    block.canonical = false;
    assert_eq!(
        store
            .find_derived("run", block.view.position.block_hash)
            .unwrap(),
        Some(block.clone())
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &store.recovery_data(B256::repeat_byte(8)).unwrap()
        )
        .unwrap(),
        plan
    );
    assert_eq!(store.runtime_status("run").unwrap(), Some(status.clone()));
    assert!(store.has_pending_recovery("run").unwrap());
    sql.execute_batch(
        "UPDATE derived_blocks SET excluded_pools=5; UPDATE runtime_status SET processed_blocks=4;",
    )
    .unwrap();
    assert_eq!(
        store
            .find_derived("run", block.view.position.block_hash)
            .unwrap()
            .unwrap()
            .excluded_pools,
        5
    );
    assert_eq!(
        store.runtime_status("run").unwrap().unwrap()["processed_blocks"],
        4
    );
    store.save_runtime_status("run", &status).unwrap();
    assert_eq!(store.runtime_status("run").unwrap(), Some(status));
    assert_eq!(
        store
            .read_report_page(ReportTable::Blocks, "run", 0, 1, 1)
            .unwrap()[0]
            .0,
        9
    );
    assert!(
        store
            .read_report_page(ReportTable::Blocks, "run", 0, 2, 3)
            .unwrap()
            .is_empty()
    );
    // Every generic data column is gone from the current schema, including empty tables.
    let count:i64=sql.query_row("SELECT count(*) FROM sqlite_master m JOIN pragma_table_info(m.name) p WHERE m.type='table' AND p.name='data'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 0);

    // Scalar strings count toward the read budget before model reconstruction too.
    sql.execute_batch("UPDATE derived_blocks SET view_id=CAST(zeroblob(67108865) AS TEXT)")
        .unwrap();
    assert!(matches!(
        store.find_derived("run", block.view.position.block_hash),
        Err(arb_adapters::store::StoreError::Invalid(
            "record read budget"
        ))
    ));
    sql.execute_batch("UPDATE recovery_jobs SET run_id=CAST(zeroblob(67108865) AS TEXT)")
        .unwrap();
    assert!(matches!(
        store.recovery_data(B256::repeat_byte(8)),
        Err(arb_adapters::store::StoreError::Invalid(
            "record read budget"
        ))
    ));

    // Failure in the last converted table must roll back earlier table conversions as well.
    let bad_path = dir.path().join("bad-v14.db");
    let bad = Connection::open(&bad_path).unwrap();
    bad.execute_batch(include_str!("../schema/schema_v014.sql"))
        .unwrap();
    let legacy_block = serde_json::to_vec(&block).unwrap();
    bad.execute("INSERT INTO derived_blocks(id,run_id,block_hash,block_number,data) VALUES(9,'run',?1,1,?2)", params![block.view.position.block_hash.to_string(),legacy_block]).unwrap();
    bad.execute(
        "INSERT INTO runtime_status VALUES('run',?1)",
        [b"{".as_slice()],
    )
    .unwrap();
    assert!(Store::open(&bad_path).is_err());
    assert_eq!(
        bad.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        14
    );
    assert_eq!(
        bad.query_row("SELECT data FROM derived_blocks WHERE id=9", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        legacy_block
    );
    assert_eq!(
        bad.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name LIKE '%_v014'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
