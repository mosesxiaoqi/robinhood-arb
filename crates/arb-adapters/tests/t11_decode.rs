#[path = "support/verified.rs"]
mod verified;
use arb_adapters::{decode::decode, discovery::discover};
use arb_core::types::*;
use serde_json::Value;
#[test]
fn decode_verified_event() {
    let raw = verified::receipt();
    let pool = discover(std::slice::from_ref(&raw)).unwrap().remove(0);
    let observations = decode(&raw, &pool).unwrap();
    assert_eq!(observations.len(), 4);
    assert!(
        observations
            .iter()
            .all(|o| o.execution_status == ExecutionStatus::Succeeded
                && o.raw_ref.sequence == raw.sequence)
    );
    let mut bad = raw.clone();
    let mut value: Value = serde_json::from_slice(&bad.payload).unwrap();
    let swap = value["result"][0]["logs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|l| l["topics"][0].as_str().unwrap().starts_with("0x40e9"))
        .unwrap();
    swap["data"] = "0x01".into();
    bad.payload = serde_json::to_vec(&value).unwrap();
    assert!(decode(&bad, &pool).is_err());
    let mut failed = raw.clone();
    let mut value: Value = serde_json::from_slice(&failed.payload).unwrap();
    value["result"][0]["status"] = "0x0".into();
    value["result"][0]["logs"] = serde_json::json!([]);
    failed.payload = serde_json::to_vec(&value).unwrap();
    assert!(decode(&failed, &pool).unwrap().is_empty());
    let mut fee_raw = raw.clone();
    let mut value: Value = serde_json::from_slice(&fee_raw.payload).unwrap();
    let mut log = value["result"][0]["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["topics"][0].as_str().unwrap().starts_with("0x40e9"))
        .unwrap()
        .clone();
    let pool_id = log["topics"][1].clone();
    log["topics"] = serde_json::json!([
        alloy_primitives::keccak256("ProtocolFeeUpdated(bytes32,uint24)"),
        pool_id
    ]);
    log["data"] = serde_json::json!(format!("0x{:064x}", 100));
    value["result"][0]["logs"] = serde_json::json!([log]);
    fee_raw.payload = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        decode(&fee_raw, &pool).unwrap()[0].event,
        PoolEvent::ProtocolFeeUpdated { fee: 100 }
    );
    let mut feed = raw;
    feed.kind = "message".into();
    assert!(decode(&feed, &pool).unwrap().is_empty());
}
