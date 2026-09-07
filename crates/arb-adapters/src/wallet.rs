use crate::{
    decode::decode,
    discovery::receipt_logs,
    rpc::{RpcSource, SourceError},
};
use alloy_primitives::{Address, B256, U256, keccak256};
use arb_core::{simulation::AssetChange, types::*, wallet::*};
use serde_json::{Value, json};
fn invalid() -> SourceError {
    SourceError::Invalid("wallet evidence scope")
}
fn amount(v: &Value) -> Result<U256, SourceError> {
    v.as_str().and_then(|s| s.parse().ok()).ok_or_else(invalid)
}
impl RpcSource {
    /// Captures conservative block-boundary facts. No unsupported trace or transaction P&L inference.
    pub async fn wallet_facts(
        &self,
        hash: B256,
        wallet: Address,
        tokens: &[Address],
        pools: &[PoolDescriptor],
    ) -> Result<WalletFacts, SourceError> {
        if self.chain_id() != 4663
            || hash == B256::ZERO
            || tokens.len() > 32
            || pools.len() > 128
            || tokens.contains(&Address::ZERO)
            || tokens
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != tokens.len()
        {
            return Err(invalid());
        }
        let _permit = self.gate.acquire().await.map_err(|_| invalid())?;
        let mut evidence = vec![];
        if self
            .evidence_request("eth_chainId", json!([]), &mut evidence)
            .await?
            != json!("0x1237")
        {
            return Err(invalid());
        }
        let transaction = self
            .evidence_request("eth_getTransactionByHash", json!([hash]), &mut evidence)
            .await?;
        let receipt = self
            .evidence_request("eth_getTransactionReceipt", json!([hash]), &mut evidence)
            .await?;
        let position = ChainPosition {
            block_number: amount(&receipt["blockNumber"])?
                .try_into()
                .map_err(|_| invalid())?,
            block_hash: serde_json::from_value(receipt["blockHash"].clone())
                .map_err(|_| invalid())?,
            offset: Offset::BlockEnd,
        };
        let index = amount(&receipt["transactionIndex"])?;
        if receipt["transactionHash"] != json!(hash)
            || transaction["hash"] != json!(hash)
            || transaction["blockHash"] != receipt["blockHash"]
            || transaction["blockNumber"] != receipt["blockNumber"]
            || amount(&transaction["transactionIndex"])? != index
            || position.block_number == 0
        {
            return Err(invalid());
        }
        let tag = format!("0x{:x}", position.block_number);
        let prior = format!("0x{:x}", position.block_number - 1);
        let header = self
            .evidence_request("eth_getBlockByNumber", json!([tag, false]), &mut evidence)
            .await?;
        let parent = self
            .evidence_request("eth_getBlockByNumber", json!([prior, false]), &mut evidence)
            .await?;
        if header["hash"] != json!(position.block_hash)
            || header["parentHash"] != parent["hash"]
            || header["number"] != json!(tag)
            || parent["number"] != json!(prior)
        {
            return Err(invalid());
        }
        let status = match receipt["status"].as_str() {
            Some("0x1") => ExecutionStatus::Succeeded,
            Some("0x0") => ExecutionStatus::Reverted,
            _ => return Err(invalid()),
        };
        let raw = RawRecord {
            version: 1,
            chain_id: 4663,
            source: "wallet-rpc".into(),
            run_id: "wallet-facts".into(),
            sequence: 0,
            received_at_ms: 0,
            request_elapsed_ns: None,
            kind: "receipts".into(),
            position: Some(position.clone()),
            transaction_hash: Some(hash),
            execution_status: status.clone(),
            confirmation: Confirmation::Included,
            payload: serde_json::to_vec(&json!({"result":[receipt]})).map_err(|_| invalid())?,
        };
        let mut transfers = vec![];
        for log in receipt_logs(&raw).map_err(|_| invalid())? {
            if U256::from(log.transaction_index) != index {
                return Err(invalid());
            }
            if log.topics.first() == Some(&keccak256("Transfer(address,address,uint256)"))
                && tokens.contains(&log.address)
            {
                if log.topics.len() != 3
                    || log.data.len() != 32
                    || log.topics[1][..12] != [0; 12]
                    || log.topics[2][..12] != [0; 12]
                {
                    return Err(invalid());
                }
                transfers.push(TransferFact {
                    asset: log.address,
                    from: Address::from_slice(&log.topics[1][12..]),
                    to: Address::from_slice(&log.topics[2][12..]),
                    amount: U256::from_be_slice(&log.data),
                    log_index: log.log_index.to(),
                });
            }
        }
        let mut swaps = 0;
        let mut liquidity = 0;
        for pool in pools {
            for observation in decode(&raw, pool).map_err(|_| invalid())? {
                match observation.event {
                    PoolEvent::Swap { .. } => swaps += 1,
                    PoolEvent::LiquidityChanged { .. } => liquidity += 1,
                    _ => {}
                }
            }
        }
        let mut changes = vec![];
        for asset in std::iter::once(&Address::ZERO).chain(tokens.iter()) {
            let mut balances = vec![];
            for at in [&prior, &tag] {
                let value = if *asset == Address::ZERO {
                    self.evidence_request("eth_getBalance", json!([wallet, at]), &mut evidence)
                        .await?
                } else {
                    self.evidence_request("eth_call",json!([{"to":asset,"data":format!("0x70a08231000000000000000000000000{}",alloy_primitives::hex::encode(wallet))},at]),&mut evidence).await?
                };
                balances.push(amount(&value)?);
            }
            changes.push(AssetChange {
                account: wallet,
                asset: *asset,
                before: balances[0],
                after: balances[1],
            });
        }
        let code = self
            .evidence_request("eth_getCode", json!([wallet, tag]), &mut evidence)
            .await?;
        let code = alloy_primitives::hex::decode(code.as_str().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        for (at, expected) in [(&prior, &parent), (&tag, &header)] {
            let final_header = self
                .evidence_request("eth_getBlockByNumber", json!([at, false]), &mut evidence)
                .await?;
            if final_header["hash"] != expected["hash"] {
                return Err(invalid());
            }
        }
        Ok(WalletFacts {
            chain_id: 4663,
            position,
            transaction_hash: hash,
            wallet,
            sender: serde_json::from_value(transaction["from"].clone()).map_err(|_| invalid())?,
            recipient: serde_json::from_value(transaction["to"].clone()).map_err(|_| invalid())?,
            wallet_is_contract: !code.is_empty(),
            execution_status: status,
            changes,
            transfers,
            swap_count: swaps,
            liquidity_count: liquidity,
            gas_cost: amount(&receipt["gasUsed"])?
                .checked_mul(amount(&receipt["effectiveGasPrice"])?),
            balance_scope: BalanceScope::BlockBoundary,
            receipt_complete: true,
            transaction_balances_complete: false,
            valuation: None,
            evidence,
        })
    }
}
