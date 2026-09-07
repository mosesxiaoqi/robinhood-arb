use alloy_primitives::{Address, B256, U256};
use arb_core::{simulation::AssetChange, types::*, wallet::*};
use arb_research::wallets::{Activity, attribute};
fn facts() -> WalletFacts {
    WalletFacts {
        chain_id: 4663,
        position: ChainPosition {
            block_number: 2,
            block_hash: B256::repeat_byte(2),
            offset: Offset::BlockEnd,
        },
        transaction_hash: B256::repeat_byte(3),
        wallet: Address::repeat_byte(4),
        sender: Address::repeat_byte(5),
        recipient: Some(Address::repeat_byte(4)),
        wallet_is_contract: false,
        execution_status: ExecutionStatus::Succeeded,
        changes: vec![AssetChange {
            account: Address::repeat_byte(4),
            asset: Address::repeat_byte(6),
            before: U256::ZERO,
            after: U256::from(100),
        }],
        transfers: vec![TransferFact {
            asset: Address::repeat_byte(6),
            from: Address::repeat_byte(5),
            to: Address::repeat_byte(4),
            amount: U256::from(100),
            log_index: 0,
        }],
        swap_count: 0,
        liquidity_count: 0,
        gas_cost: Some(U256::from(2)),
        balance_scope: BalanceScope::BlockBoundary,
        receipt_complete: true,
        transaction_balances_complete: false,
        valuation: None,
        evidence: vec![b"fixture".to_vec()],
    }
}
#[test]
fn do_not_treat_transfer_as_profit() {
    let mut f = facts();
    let gift = attribute(&f);
    assert_eq!(gift.activity, Activity::TransferOnly);
    assert_eq!(gift.realized_profit, None);
    assert!(!gift.complete);
    f.wallet = f.sender;
    f.transfers[0].from = f.wallet;
    assert!(attribute(&f).roles.contains(&"token sender".into()));
    f.liquidity_count = 1;
    assert_eq!(attribute(&f).activity, Activity::LiquidityObserved);
    f.liquidity_count = 0;
    f.swap_count = 1;
    assert_eq!(attribute(&f).activity, Activity::SwapObserved);
    f.swap_count = 3;
    assert_eq!(attribute(&f).activity, Activity::MultipleSwapsObserved);
    f.wallet_is_contract = true;
    f.recipient = Some(f.wallet);
    let router = attribute(&f);
    assert!(router.roles.contains(&"contract recipient".into()));
    assert_eq!(router.realized_profit, None);
    f.receipt_complete = false;
    assert_eq!(attribute(&f).activity, Activity::Unknown);
    f = facts();
    f.balance_scope = BalanceScope::Transaction;
    f.transaction_balances_complete = true;
    f.valuation = Some(Valuation {
        quote_asset: Address::ZERO,
        before: U256::ZERO,
        after: U256::from(100),
        external_in: U256::from(100),
        external_out: U256::ZERO,
        basis: "complete transaction valuation fixture".into(),
    });
    assert_eq!(
        attribute(&f).net_asset_change.unwrap().magnitude,
        U256::ZERO
    );
    assert_eq!(attribute(&f).realized_profit, None);
}
