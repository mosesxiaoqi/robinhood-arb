mod support;
use alloy_primitives::{Address, B256};
use arb_core::{route::*, types::PoolVerification};
#[test]
fn pair_two_pools_both_directions() {
    let first = support::bootstrap().pools.remove(0).descriptor;
    let mut second = first.clone();
    second.id.locator = PoolLocator::Singleton {
        manager: Address::repeat_byte(2),
        pool_id: B256::repeat_byte(9),
    };
    let mut unrelated = first.clone();
    unrelated.token = Address::repeat_byte(6);
    unrelated.currency1 = unrelated.token;
    unrelated.id.locator = PoolLocator::Contract(Address::repeat_byte(7));
    let pools = vec![second.clone(), unrelated.clone(), first.clone()];
    let routes = two_leg_routes(&pools, Address::ZERO);
    assert_eq!(routes.len(), 2);
    assert!(
        routes
            .iter()
            .all(|r| r.legs.len() == 2 && r.validate().is_ok() && r.legs[0].pool != r.legs[1].pool)
    );
    let mut reversed = pools;
    reversed.reverse();
    assert_eq!(two_leg_routes(&reversed, Address::ZERO), routes);
    assert_eq!(
        two_leg_routes(
            &[first.clone(), second.clone(), first.clone()],
            Address::ZERO
        ),
        routes
    );
    unrelated.quote_asset = Address::repeat_byte(8);
    unrelated.token = first.token;
    assert_eq!(
        two_leg_routes(&[first.clone(), second.clone(), unrelated], Address::ZERO),
        routes
    );
    second.verification = PoolVerification::Pending;
    let result = route_candidates(&[first, second], Address::ZERO);
    assert!(result.routes.is_empty());
    assert_eq!(result.excluded_pools, 1);
}
