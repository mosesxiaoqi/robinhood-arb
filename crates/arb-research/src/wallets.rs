use alloy_primitives::{B256, U256};
use arb_core::{
    opportunity::{SignedAmount, profit},
    types::ExecutionStatus,
    wallet::*,
};
use serde::Serialize;
#[derive(Debug, PartialEq, Eq, Serialize)]
pub enum Activity {
    Unknown,
    TransferOnly,
    SwapObserved,
    MultipleSwapsObserved,
    LiquidityObserved,
}
#[derive(Debug, Serialize)]
pub struct WalletAttribution {
    pub transaction_hash: B256,
    pub activity: Activity,
    pub roles: Vec<String>,
    pub complete: bool,
    pub net_asset_change: Option<SignedAmount>,
    pub realized_profit: Option<SignedAmount>,
    pub limitations: Vec<String>,
}
pub fn attribute(facts: &WalletFacts) -> WalletAttribution {
    let mut roles = vec![];
    if facts.wallet == facts.sender {
        roles.push("transaction sender".into());
    }
    if facts.recipient == Some(facts.wallet) {
        roles.push(
            if facts.wallet_is_contract {
                "contract recipient"
            } else {
                "transaction recipient"
            }
            .into(),
        );
    }
    if facts.transfers.iter().any(|t| t.from == facts.wallet) {
        roles.push("token sender".into());
    }
    if facts.transfers.iter().any(|t| t.to == facts.wallet) {
        roles.push("token recipient".into());
    }
    let receipt_known = facts.receipt_complete
        && !facts.evidence.is_empty()
        && facts.execution_status != ExecutionStatus::Unknown;
    let activity = if !receipt_known || facts.execution_status == ExecutionStatus::Reverted {
        Activity::Unknown
    } else if facts.liquidity_count > 0 {
        Activity::LiquidityObserved
    } else if facts.swap_count > 1 {
        Activity::MultipleSwapsObserved
    } else if facts.swap_count == 1 {
        Activity::SwapObserved
    } else if !facts.transfers.is_empty() {
        Activity::TransferOnly
    } else {
        Activity::Unknown
    };
    let complete = receipt_known
        && facts.transaction_balances_complete
        && facts.balance_scope == BalanceScope::Transaction
        && !facts.changes.is_empty()
        && facts.changes.iter().all(|c| c.account == facts.wallet)
        && facts
            .changes
            .iter()
            .map(|c| c.asset)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == facts.changes.len();
    let net_asset_change = facts
        .valuation
        .as_ref()
        .filter(|v| complete && !v.basis.trim().is_empty())
        .and_then(|v| {
            profit(
                v.before.checked_add(v.external_in)?,
                v.after.checked_add(v.external_out)?,
                U256::ZERO,
            )
            .ok()
        });
    let mut limitations = vec![
        "transaction activity does not identify the beneficiary or prove arbitrage intent".into(),
        "realized profit needs cost basis; asset valuation is not realized profit".into(),
    ];
    if !complete {
        limitations.push("transaction-scoped balances/internal native transfers incomplete; block balances cannot isolate this transaction".into());
    }
    if facts.valuation.is_none() {
        limitations.push("valuation and external deposit/withdrawal basis unavailable".into());
    }
    WalletAttribution {
        transaction_hash: facts.transaction_hash,
        activity,
        roles,
        complete,
        net_asset_change,
        realized_profit: None,
        limitations,
    }
}
