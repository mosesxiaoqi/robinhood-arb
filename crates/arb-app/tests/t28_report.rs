#[path = "../../arb-adapters/tests/support/simulation.rs"]
mod fixture;
use alloy_primitives::{Address, B256, U256};
use arb_adapters::store::Store;
use arb_app::report::export_report;
use arb_core::{opportunity::CostEstimate, research::RunSpec};
#[test]
fn report_preserves_unknown_and_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("report.db");
    let mut store = Store::open(&database).unwrap();
    let run = RunSpec {
        run_id: "zero,\"quoted\"".into(),
        config_hash: B256::repeat_byte(7),
        algorithm_version: "test".into(),
        registry_version: 1,
        quote_asset: Address::ZERO,
        amounts: vec![U256::from(100)],
        min_depth: U256::ZERO,
        min_profit: U256::from(1),
        costs: CostEstimate {
            asset: Address::ZERO,
            amount: None,
            conversion: None,
            basis: "unavailable".into(),
        },
    };
    store.register_run(&run).unwrap();
    use arb_core::{
        simulation::*,
        types::{ChainPosition, Offset},
    };
    let request = fixture::request();
    store
        .insert_simulation(&SimulationRecord {
            id: B256::repeat_byte(90),
            queue_id: "full".into(),
            candidate_id: B256::repeat_byte(91),
            run_id: run.run_id.clone(),
            view_id: B256::repeat_byte(92),
            expected_position: ChainPosition {
                block_number: 11,
                block_hash: B256::repeat_byte(11),
                offset: Offset::BlockEnd,
            },
            request,
            phase: SimulationPhase::QueueFull,
            outcome: SimulationOutcome::Unavailable,
            submitted_at_ms: 1,
            queued_at_ms: None,
            started_at_ms: None,
            ended_at_ms: Some(1),
            queue_wait_ns: None,
            elapsed_ns: None,
            result: None,
            error: Some("queue full".into()),
            error_evidence: vec![],
            canonical: false,
            validates_original_candidate: false,
        })
        .unwrap();
    let out = dir.path().join("out");
    export_report(&database, &run.run_id, &out, Some((10, 12))).unwrap();
    let markdown = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(markdown.contains("只读模拟，非真实成交"));
    assert!(markdown.contains("数据缺口"));
    assert!(markdown.contains("未知"));
    assert!(markdown.contains("QueueFull/Unavailable"));
    assert!(markdown.contains("10–12"));
    let csv = std::fs::read_to_string(out.join("summary.csv")).unwrap();
    assert!(csv.contains("\"zero,\"\"quoted\"\"\""));
    // Existing completed output cannot be partially overwritten.
    assert!(export_report(&database, &run.run_id, &out, None).is_err());
    assert_eq!(
        markdown,
        std::fs::read_to_string(out.join("report.md")).unwrap()
    );
    let file_parent = dir.path().join("file-parent");
    std::fs::write(&file_parent, b"existing").unwrap();
    assert!(
        export_report(
            &database,
            &run.run_id,
            &file_parent.join("bad"),
            Some((10, 12))
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&file_parent).unwrap(), b"existing");
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100001)
        INSERT INTO wallet_facts(run_id,chain_id,block_number,block_hash,offset_kind,transaction_hash,wallet,sender,wallet_is_contract,execution_status,changes_json,transfers_json,swap_count,liquidity_count,balance_scope,receipt_complete,transaction_balances_complete,evidence_json)
        SELECT ?1,4663,1,printf('0x%064x',1),'BlockEnd',printf('0x%064x',x),printf('0x%040x',1),printf('0x%040x',1),0,'Unknown','[]','[]',0,0,'BlockBoundary',0,0,'[]' FROM n", [&run.run_id]).unwrap();
    export_report(
        &database,
        &run.run_id,
        &dir.path().join("small-window"),
        Some((10, 12)),
    )
    .unwrap();
    store
        .begin_recovery(
            B256::repeat_byte(8),
            &run.run_id,
            &serde_json::to_vec(&serde_json::json!({
                "id": B256::repeat_byte(8), "run_id":run.run_id,"checkpoint_id":0,
                "common_ancestor":{"block_number":0,"block_hash":B256::ZERO,"offset":"BlockEnd"},
                "required_fetch":null,"orphan_hashes":[],"replay_blocks":[]
            }))
            .unwrap(),
            &[],
        )
        .unwrap();
    let blocked = dir.path().join("blocked");
    assert!(export_report(&database, &run.run_id, &blocked, None).is_err());
    assert!(!blocked.exists());
}
