use arb_core::types::*;
use serde_json::{Value, json};
pub fn receipt() -> RawRecord {
    let source: Value = serde_json::from_str(include_str!(
        "../../../../tests/data/verified/pons-observations.json"
    ))
    .unwrap();
    let receipt = source["requests"][4]["response"]["result"].clone();
    RawRecord {
        version: 1,
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "verified".into(),
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
            block_hash: receipt["blockHash"].as_str().unwrap().parse().unwrap(),
            offset: Offset::BlockEnd,
        }),
        transaction_hash: None,
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Included,
        payload: serde_json::to_vec(&json!({"result":[receipt]})).unwrap(),
    }
}
