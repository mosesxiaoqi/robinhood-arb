#[path = "support/replay.rs"]
mod support;
use alloy_primitives::B256;
use arb_adapters::store::StoredRaw;
use arb_app::replay::ObservedReplay;
use arb_core::types::ExecutionStatus;
#[test]
fn late_receipt_stays_unknown() {
    let records = support::block(2, B256::repeat_byte(1));
    let tx = B256::repeat_byte(102);
    let mut call = records[0].clone();
    call.kind = "call".into();
    call.received_at_ms = 10;
    call.transaction_hash = Some(tx);
    let mut receipt = records[1].clone();
    receipt.received_at_ms = 20;
    let mut replay = ObservedReplay::new(vec![
        StoredRaw {
            id: 1,
            record: call.clone(),
        },
        StoredRaw {
            id: 2,
            record: receipt.clone(),
        },
    ])
    .unwrap();
    replay.advance(15).unwrap();
    assert_eq!(replay.status(tx), ExecutionStatus::Unknown);
    replay.advance(20).unwrap();
    assert_eq!(replay.status(tx), ExecutionStatus::Succeeded);
    assert!(replay.advance(19).is_err());
    call.received_at_ms = 5;
    let replay = ObservedReplay::new(vec![
        StoredRaw {
            id: 1,
            record: receipt,
        },
        StoredRaw {
            id: 2,
            record: call,
        },
    ])
    .unwrap();
    assert_eq!(replay.clock_regressions(), 1);
}
