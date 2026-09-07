use alloy_primitives::B256;
use arb_adapters::discovery::discover;
use arb_core::types::*;
use serde_json::{Value, json};
#[test]
fn register_verified_pool_once() {
    let source: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/pons-observations.json"
    ))
    .unwrap();
    let receipt = source["requests"][4]["response"]["result"].clone();
    let raw = RawRecord {
        version: 1,
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "test".into(),
        sequence: 1,
        received_at_ms: 1,
        request_elapsed_ns: None,
        kind: "receipts".into(),
        position: Some(ChainPosition {
            block_number: u64::from_str_radix(
                receipt["blockNumber"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
                16,
            )
            .unwrap(),
            block_hash: receipt["blockHash"]
                .as_str()
                .unwrap()
                .parse::<B256>()
                .unwrap(),
            offset: Offset::BlockEnd,
        }),
        transaction_hash: None,
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Included,
        payload: serde_json::to_vec(&json!({"result":[receipt]})).unwrap(),
    };
    let pools = discover(&[raw.clone(), raw.clone()]).unwrap();
    assert_eq!(pools.len(), 1);
    assert!(!pools[0].is_quoteable());
    assert_eq!(pools[0].tick_spacing, 200);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pools.db");
    let mut store = arb_adapters::store::Store::open(&path).unwrap();
    store.register_pools(&pools).unwrap();
    store.register_pools(&pools).unwrap();
    assert_eq!(store.read_pools(None, 10).unwrap(), pools);
    assert!(store.read_pools(Some(&pools[0].id), 10).unwrap().is_empty());
    let mut forged = raw.clone();
    let text = String::from_utf8(forged.payload).unwrap().replace(
        "0x7ed598bcef8bd9edd8c97a195c6d13f40801ec7e",
        "0x1111111111111111111111111111111111111111",
    );
    forged.payload = text.into_bytes();
    assert!(discover(&[forged]).unwrap().is_empty());
    let mut wrong_chain = raw;
    wrong_chain.chain_id = 1;
    assert!(discover(&[wrong_chain]).is_err());
}
