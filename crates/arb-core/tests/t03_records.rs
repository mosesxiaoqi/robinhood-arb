use alloy_primitives::B256;
use arb_core::types::*;
fn sample() -> RawRecord {
    RawRecord {
        version: 1,
        chain_id: 4663,
        source: "feed".into(),
        run_id: "test".into(),
        sequence: 1,
        received_at_ms: 10,
        request_elapsed_ns: None,
        kind: "message".into(),
        position: None,
        transaction_hash: Some(B256::repeat_byte(1)),
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Unknown,
        payload: b"{\"message\":1}".to_vec(),
    }
}
#[test]
fn preserve_unknown_and_position() {
    for offset in [
        Offset::BlockEnd,
        Offset::Transaction {
            index: 3,
            log_index: Some(5),
        },
    ] {
        let mut raw = sample();
        raw.position = Some(ChainPosition {
            block_number: 4,
            block_hash: B256::repeat_byte(2),
            offset,
        });
        let decoded: RawRecord =
            serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(decoded, raw);
        assert_eq!(decoded.execution_status, ExecutionStatus::Unknown);
    }
    let mut raw = sample();
    assert!(raw.validate().is_ok());
    raw.version = 2;
    assert!(raw.validate().is_err());
    raw.version = 1;
    raw.payload = vec![0; 64 * 1024 * 1024 + 1];
    assert!(raw.validate().is_err());
    let negative = serde_json::to_string(&sample())
        .unwrap()
        .replace("\"sequence\":1", "\"sequence\":-1");
    assert!(serde_json::from_str::<RawRecord>(&negative).is_err());
}
