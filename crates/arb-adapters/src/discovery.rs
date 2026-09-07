use alloy_primitives::{Address, B256, Bytes, U64, address, keccak256};
use alloy_sol_types::{SolEvent, SolValue, sol};
use arb_core::{
    route::{PoolId, PoolLocator},
    types::{ChainPosition, Offset, PoolDescriptor, PoolVerification, RawRecord},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const FACTORY: Address = address!("7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e");
pub const HOOK: Address = address!("E5e702641Ea86F4ae6cC3cDaeD2B886f976Be044");
pub const MANAGER: Address = address!("8366a39cc670b4001a1121b8f6a443a643e40951");
pub(crate) mod abi {
    use super::*;
    sol! {
     event PoolGraduated(address indexed token,uint256 positionId,uint256 tokenAmount,uint256 pairTokenAmount);
     event PoolRegistered(bytes32 indexed poolId,address memecoin,address quoteToken,address creator);
     event Initialize(bytes32 indexed id,address indexed currency0,address indexed currency1,uint24 fee,int24 tickSpacing,address hooks,uint160 sqrtPriceX96,int24 tick);
    }
}
#[derive(Debug, Error)]
#[error("invalid discovery input: {0}")]
pub struct DiscoveryError(pub &'static str);
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChainLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
    pub block_hash: B256,
    pub block_number: U64,
    pub transaction_hash: B256,
    pub transaction_index: U64,
    pub log_index: U64,
    pub removed: bool,
}
impl ChainLog {
    pub fn event<E: SolEvent>(&self) -> Result<E, DiscoveryError> {
        E::decode_raw_log_validate(self.topics.iter().copied(), &self.data)
            .map_err(|_| DiscoveryError("event ABI"))
    }
    pub fn position(&self) -> ChainPosition {
        ChainPosition {
            block_number: self.block_number.to(),
            block_hash: self.block_hash,
            offset: Offset::Transaction {
                index: self.transaction_index.to(),
                log_index: Some(self.log_index.to()),
            },
        }
    }
}
pub(crate) fn receipt_logs(raw: &RawRecord) -> Result<Vec<ChainLog>, DiscoveryError> {
    raw.validate().map_err(|_| DiscoveryError("raw record"))?;
    if raw.kind != "receipts" {
        return Ok(vec![]);
    }
    let position = raw
        .position
        .as_ref()
        .ok_or(DiscoveryError("missing block position"))?;
    let envelope: Value =
        serde_json::from_slice(&raw.payload).map_err(|_| DiscoveryError("receipt JSON"))?;
    let receipts = envelope["result"]
        .as_array()
        .ok_or(DiscoveryError("receipt array"))?;
    let mut logs = Vec::new();
    let mut seen = BTreeSet::new();
    for receipt in receipts {
        let entries = receipt["logs"]
            .as_array()
            .ok_or(DiscoveryError("logs array"))?;
        match receipt["status"].as_str() {
            Some("0x0") => {
                if !entries.is_empty() {
                    return Err(DiscoveryError("reverted logs"));
                }
                continue;
            }
            Some("0x1") => {}
            _ => return Err(DiscoveryError("unknown receipt status")),
        }
        let tx_hash: B256 = serde_json::from_value(receipt["transactionHash"].clone())
            .map_err(|_| DiscoveryError("transaction hash"))?;
        for entry in entries {
            let log: ChainLog =
                serde_json::from_value(entry.clone()).map_err(|_| DiscoveryError("log fields"))?;
            if log.removed
                || log.block_hash != position.block_hash
                || log.block_number.to::<u64>() != position.block_number
                || log.transaction_hash != tx_hash
                || !seen.insert(log.log_index)
            {
                return Err(DiscoveryError("log identity/coverage"));
            }
            logs.push(log);
        }
    }
    logs.sort_by_key(|l| (l.transaction_index, l.log_index));
    Ok(logs)
}
pub fn discover(records: &[RawRecord]) -> Result<Vec<PoolDescriptor>, DiscoveryError> {
    let mut pools = BTreeMap::new();
    for raw in records {
        if raw.chain_id != 4663 {
            return Err(DiscoveryError("unsupported network"));
        }
        let logs = receipt_logs(raw)?;
        for initialized in logs.iter().filter(|l| {
            l.address == MANAGER && l.topics.first() == Some(&abi::Initialize::SIGNATURE_HASH)
        }) {
            let event: abi::Initialize = initialized.event()?;
            if event.hooks != HOOK {
                continue;
            }
            let key = (
                event.currency0,
                event.currency1,
                event.fee,
                event.tickSpacing,
                event.hooks,
            )
                .abi_encode();
            if event.id != keccak256(key)
                || event.currency0 >= event.currency1
                || event.tickSpacing.as_i32() <= 0
            {
                return Err(DiscoveryError("invalid pool key"));
            }
            let registered = logs.iter().find(|l| {
                l.address == HOOK
                    && l.transaction_hash == initialized.transaction_hash
                    && l.topics.first() == Some(&abi::PoolRegistered::SIGNATURE_HASH)
                    && l.topics.get(1) == Some(&event.id)
            });
            let Some(registered) = registered else {
                continue;
            };
            let registered: abi::PoolRegistered = registered.event()?;
            let graduated = logs.iter().any(|l| {
                l.address == FACTORY
                    && l.transaction_hash == initialized.transaction_hash
                    && l.topics.first() == Some(&abi::PoolGraduated::SIGNATURE_HASH)
                    && l.event::<abi::PoolGraduated>()
                        .is_ok_and(|g| g.token == registered.memecoin)
            });
            if !graduated {
                continue;
            }
            if !((registered.memecoin == event.currency0
                && registered.quoteToken == event.currency1)
                || (registered.memecoin == event.currency1
                    && registered.quoteToken == event.currency0))
            {
                return Err(DiscoveryError("registered assets mismatch"));
            }
            let id = PoolId {
                chain_id: 4663,
                locator: PoolLocator::Singleton {
                    manager: MANAGER,
                    pool_id: event.id,
                },
            };
            let pool = PoolDescriptor {
                id: id.clone(),
                protocol: "pons-v2-v4".into(),
                token: registered.memecoin,
                quote_asset: registered.quoteToken,
                currency0: event.currency0,
                currency1: event.currency1,
                hook: event.hooks,
                lp_fee: event.fee.to(),
                tick_spacing: event.tickSpacing.as_i32(),
                hook_fee_bps: None,
                creator_tax_bps: None,
                token_decimals: None,
                quote_decimals: (registered.quoteToken == Address::ZERO).then_some(18),
                initialized_at: initialized.position(),
                verification: if event.fee.is_zero() {
                    PoolVerification::Pending
                } else {
                    PoolVerification::Unsupported("nonzero LP fee".into())
                },
            };
            if let Some(previous) = pools.insert(id, pool.clone())
                && previous != pool
            {
                return Err(DiscoveryError("conflicting pool registration"));
            }
        }
    }
    Ok(pools.into_values().collect())
}
