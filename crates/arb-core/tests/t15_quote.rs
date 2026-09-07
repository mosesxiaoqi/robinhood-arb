mod support;
use alloy_primitives::{Address, U256, address};
use arb_core::{
    protocol::verified::{QuoteError, quote},
    types::PoolVerification,
};
use serde_json::Value;
use std::collections::BTreeMap;
fn number(v: &Value, key: &str) -> U256 {
    U256::from_str_radix(v[key].as_str().unwrap(), 10).unwrap()
}
#[test]
fn match_verified_quote() {
    let buy: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/quote-expected.json"
    ))
    .unwrap();
    let sell: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/reverse-expected.json"
    ))
    .unwrap();
    let mut pool = support::bootstrap().pools.remove(0);
    pool.descriptor.id.locator = arb_core::route::PoolLocator::Singleton {
        manager: address!("8366a39cc670b4001a1121b8f6a443a643e40951"),
        pool_id: buy["pool_id"].as_str().unwrap().parse().unwrap(),
    };
    pool.descriptor.protocol = "pons-v2-v4".into();
    pool.descriptor.hook = address!("E5e702641Ea86F4ae6cC3cDaeD2B886f976Be044");
    pool.descriptor.token = buy["token"].as_str().unwrap().parse().unwrap();
    pool.descriptor.currency1 = pool.descriptor.token;
    pool.liquidity = number(&buy, "liquidity").to();
    pool.tick_bitmap = BTreeMap::from([(3, U256::ZERO)]);
    for (sample, key, tick_key, asset) in [
        (
            &buy,
            "initial_sqrt_price_x96",
            "initial_tick",
            Address::ZERO,
        ),
        (
            &sell,
            "sqrt_price_before",
            "tick_before",
            pool.descriptor.token,
        ),
    ] {
        pool.sqrt_price_x96 = number(sample, key);
        pool.tick = sample[tick_key].as_i64().unwrap() as i32;
        let q = quote(&pool, asset, number(sample, "input")).unwrap();
        assert_eq!(q.amount_out, number(sample, "net_output"));
        assert_eq!(q.gross_amount_out, number(sample, "gross_output"));
        assert_eq!(q.sqrt_price_after, number(sample, "sqrt_price_after"));
        assert_eq!(q.included_hook_fee, number(sample, "hook_fee"));
        assert_eq!(q.included_creator_tax, number(sample, "creator_tax"));
    }
    assert_eq!(
        quote(&pool, Address::ZERO, U256::ZERO),
        Err(QuoteError::ZeroInput)
    );
    assert_eq!(
        quote(&pool, Address::ZERO, U256::MAX),
        Err(QuoteError::Overflow)
    );
    let mut bad = pool.clone();
    bad.liquidity = 0;
    assert_eq!(
        quote(&bad, Address::ZERO, U256::from(1)),
        Err(QuoteError::EmptyLiquidity)
    );
    bad = pool.clone();
    bad.descriptor.creator_tax_bps = Some(9000);
    assert_eq!(
        quote(&bad, Address::ZERO, U256::from(1)),
        Err(QuoteError::Unsupported)
    );
    bad = pool.clone();
    bad.descriptor.hook = Address::ZERO;
    assert_eq!(
        quote(&bad, Address::ZERO, U256::from(1)),
        Err(QuoteError::Unsupported)
    );
    bad = pool.clone();
    bad.descriptor.verification = PoolVerification::Pending;
    assert_eq!(
        quote(&bad, Address::ZERO, U256::from(1)),
        Err(QuoteError::Unsupported)
    );
    // Large input must not be quoted using constant liquidity across an unproven word boundary.
    assert_eq!(
        quote(&pool, Address::ZERO, U256::from(10).pow(U256::from(30))),
        Err(QuoteError::TickBoundary)
    );
}
