mod support;
use alloy_primitives::{Address, B256, U256, address};
use arb_core::{opportunity::*, protocol::verified::sqrt_at_tick, route::*, state::State};
#[test]
fn subtract_cost_once() {
    assert_eq!(
        profit(U256::from(100), U256::from(110), U256::from(3)).unwrap(),
        SignedAmount {
            negative: false,
            magnitude: U256::from(7)
        }
    );
    assert_eq!(
        profit(U256::from(100), U256::from(90), U256::from(3)).unwrap(),
        SignedAmount {
            negative: true,
            magnitude: U256::from(13)
        }
    );
    let mut bootstrap = support::bootstrap();
    let p = &mut bootstrap.pools[0];
    p.descriptor.protocol = "pons-v2-v4".into();
    p.descriptor.hook = address!("E5e702641Ea86F4ae6cC3cDaeD2B886f976Be044");
    p.descriptor.id.locator = PoolLocator::Singleton {
        manager: address!("8366a39cc670b4001a1121b8f6a443a643e40951"),
        pool_id: B256::repeat_byte(1),
    };
    p.tick = 4000;
    p.sqrt_price_x96 = sqrt_at_tick(p.tick).unwrap();
    let mut second = p.clone();
    second.tick = 1000;
    second.sqrt_price_x96 = sqrt_at_tick(second.tick).unwrap();
    second.descriptor.id.locator = PoolLocator::Singleton {
        manager: address!("8366a39cc670b4001a1121b8f6a443a643e40951"),
        pool_id: B256::repeat_byte(2),
    };
    bootstrap.pools.push(second);
    let view = State::from_bootstrap(bootstrap).unwrap().view();
    let routes = two_leg_routes(
        &view
            .pools
            .iter()
            .map(|p| p.descriptor.clone())
            .collect::<Vec<_>>(),
        Address::ZERO,
    );
    let costs = CostEstimate {
        asset: Address::ZERO,
        amount: Some(U256::from(3)),
        conversion: None,
        basis: "synthetic test estimate".into(),
    };
    let result = evaluate(&view, &routes[0], U256::from(100), &costs).unwrap();
    assert_eq!(result.quotes[1].amount_in, result.quotes[0].amount_out);
    assert_eq!(
        result.net_profit,
        Some(profit(result.amount_in, result.amount_out, U256::from(3)).unwrap())
    );
    let scan_result = scan(
        &view,
        &routes[0],
        &[U256::from(100)],
        &costs,
        U256::ZERO,
        U256::ZERO,
    );
    assert_eq!(scan_result.opportunities.len(), 1);
    assert!(scan_result.excluded.is_empty());
    assert_eq!(
        scan(
            &view,
            &routes[0],
            &[U256::from(100)],
            &costs,
            U256::MAX,
            U256::ZERO
        )
        .excluded
        .len(),
        1
    );
    let mut unknown = costs.clone();
    unknown.amount = None;
    assert_eq!(
        evaluate(&view, &routes[0], U256::from(100), &unknown)
            .unwrap()
            .net_profit,
        None
    );
    unknown.amount = Some(U256::from(3));
    unknown.asset = Address::repeat_byte(9);
    assert_eq!(
        evaluate(&view, &routes[0], U256::from(100), &unknown)
            .unwrap()
            .net_profit,
        None
    );
    unknown.conversion = Some(Conversion {
        quote_asset: Address::ZERO,
        numerator: U256::from(2),
        denominator: U256::from(1),
        position: view.position.clone(),
    });
    assert_eq!(
        evaluate(&view, &routes[0], U256::from(100), &unknown)
            .unwrap()
            .net_profit,
        Some(profit(result.amount_in, result.amount_out, U256::from(6)).unwrap())
    );
}
