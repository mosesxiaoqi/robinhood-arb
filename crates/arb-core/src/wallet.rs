use crate::{
    simulation::AssetChange,
    types::{ChainPosition, ExecutionStatus},
};
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BalanceScope {
    BlockBoundary,
    Transaction,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferFact {
    pub asset: Address,
    pub from: Address,
    pub to: Address,
    pub amount: U256,
    pub log_index: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Valuation {
    pub quote_asset: Address,
    pub before: U256,
    pub after: U256,
    pub external_in: U256,
    pub external_out: U256,
    pub basis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletFacts {
    pub chain_id: u64,
    pub position: ChainPosition,
    pub transaction_hash: B256,
    pub wallet: Address,
    pub sender: Address,
    pub recipient: Option<Address>,
    pub wallet_is_contract: bool,
    pub execution_status: ExecutionStatus,
    /// RPC before/after-block balances include every transaction in that block.
    pub changes: Vec<AssetChange>,
    pub transfers: Vec<TransferFact>,
    /// Counts cover this transaction's decoded, registered pools, not a trader identity.
    pub swap_count: usize,
    pub liquidity_count: usize,
    pub gas_cost: Option<U256>,
    pub balance_scope: BalanceScope,
    pub receipt_complete: bool,
    pub transaction_balances_complete: bool,
    pub valuation: Option<Valuation>,
    pub evidence: Vec<Vec<u8>>,
}
