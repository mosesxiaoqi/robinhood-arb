use alloy_primitives::{Address, B256};
use arb_core::route::*;
fn leg(n: u8, a: u8, b: u8) -> Leg {
    Leg {
        pool: PoolId {
            chain_id: 4663,
            locator: PoolLocator::Singleton {
                manager: Address::repeat_byte(9),
                pool_id: B256::repeat_byte(n),
            },
        },
        protocol: "v4".into(),
        asset_in: Address::repeat_byte(a),
        asset_out: Address::repeat_byte(b),
    }
}
#[test]
fn validate_closed_route() {
    let two = Route {
        legs: vec![leg(1, 0, 1), leg(2, 1, 0)],
    };
    let three = Route {
        legs: vec![leg(1, 0, 1), leg(2, 1, 2), leg(3, 2, 0)],
    };
    assert!(two.validate().is_ok());
    assert!(three.validate().is_ok());
    assert!(Route { legs: vec![] }.validate().is_err());
    assert!(
        Route {
            legs: vec![leg(1, 0, 1), leg(2, 2, 0)]
        }
        .validate()
        .is_err()
    );
    assert!(
        Route {
            legs: vec![leg(1, 0, 1), leg(2, 1, 2)]
        }
        .validate()
        .is_err()
    );
    let mut cross = two.clone();
    cross.legs[1].pool.chain_id = 1;
    assert!(cross.validate().is_err());
    assert_ne!(two.legs[0].pool, two.legs[1].pool);
}
