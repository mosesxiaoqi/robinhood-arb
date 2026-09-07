use alloy_primitives::{B256, U256, address, keccak256};
use arb_core::types::*;
use serde_json::json;
pub fn block(number: u64, parent: B256) -> Vec<RawRecord> {
    let hash = B256::repeat_byte(number as u8);
    let tx = B256::repeat_byte((number + 100) as u8);
    let tick = 4000 + number as i32 * 100;
    let values = [
        U256::from(1),
        U256::MAX,
        arb_core::protocol::verified::sqrt_at_tick(tick).unwrap(),
        U256::from(1000000),
        U256::from(tick as u32),
        U256::ZERO,
    ];
    let data = format!(
        "0x{}",
        values
            .iter()
            .map(|v| format!("{v:064x}"))
            .collect::<String>()
    );
    let log = json!({"address":address!("8366a39cc670b4001a1121b8f6a443a643e40951"),"topics":[keccak256("Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)"),B256::repeat_byte(1),B256::ZERO],"data":data,"blockNumber":format!("0x{number:x}"),"blockHash":hash,"transactionHash":tx,"transactionIndex":"0x0","logIndex":"0x0","removed":false});
    let block = json!({"number":format!("0x{number:x}"),"hash":hash,"parentHash":parent,"transactions":[tx]});
    let receipts = json!([{"blockNumber":format!("0x{number:x}"),"blockHash":hash,"transactionHash":tx,"transactionIndex":"0x0","status":"0x1","logs":[log.clone()]}]);
    [
        ("block", block),
        ("receipts", receipts),
        ("logs", json!([log])),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (kind, value))| RawRecord {
        version: 1,
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "fixed".into(),
        sequence: number * 10 + i as u64,
        received_at_ms: number * 100 + i as u64,
        kind: kind.into(),
        position: Some(ChainPosition {
            block_number: number,
            block_hash: hash,
            offset: Offset::BlockEnd,
        }),
        transaction_hash: None,
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Included,
        payload: serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":number*10+i as u64,"result":value}),
        )
        .unwrap(),
    })
    .collect()
}
