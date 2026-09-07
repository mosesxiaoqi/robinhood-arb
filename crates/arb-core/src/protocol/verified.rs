use super::PoolState;
use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub amount_in: U256,
    pub asset_out: Address,
    pub amount_out: U256,
    pub gross_amount_out: U256,
    pub sqrt_price_after: U256,
    pub included_lp_fee: U256,
    pub included_hook_fee: U256,
    pub included_creator_tax: U256,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QuoteError {
    #[error("unsupported protocol, hook, fee or unverified state")]
    Unsupported,
    #[error("zero input")]
    ZeroInput,
    #[error("empty liquidity")]
    EmptyLiquidity,
    #[error("invalid price, tick or asset")]
    InvalidState,
    #[error("integer or on-chain amount overflow")]
    Overflow,
    #[error("quote crosses loaded tick boundary")]
    TickBoundary,
}
// TickMath / SqrtPriceMath formulas adapted from Uniswap v4-core, SPDX-License-Identifier: MIT.
// Verified source fingerprints and independent bidirectional samples: docs/verification/protocol.md.
pub fn sqrt_at_tick(tick: i32) -> Result<U256, QuoteError> {
    if !(-887272..=887272).contains(&tick) {
        return Err(QuoteError::InvalidState);
    }
    const FACTORS: [u128; 20] = [
        0xfffcb933bd6fad37aa2d162d1a594001,
        0xfff97272373d413259a46990580e213a,
        0xfff2e50f5f656932ef12357cf3c7fdcc,
        0xffe5caca7e10e4e61c3624eaa0941cd0,
        0xffcb9843d60f6159c9db58835c926644,
        0xff973b41fa98c081472e6896dfb254c0,
        0xff2ea16466c96a3843ec78b326b52861,
        0xfe5dee046a99a2a811c461f1969c3053,
        0xfcbe86c7900a88aedcffc83b479aa3a4,
        0xf987a7253ac413176f2b074cf7815e54,
        0xf3392b0822b70005940c7a398e4b70f3,
        0xe7159475a2c29b7443b29c7fa6e889d9,
        0xd097f3bdfd2022b8845ad8f792aa5825,
        0xa9f746462d870fdf8a65dc1f90e061e5,
        0x70d869a156d2a1b890bb3df62baf32f7,
        0x31be135f97d08fd981231505542fcfa6,
        0x9aa508b5b7a84e1c677de54f3e99bc9,
        0x5d6af8dedb81196699c329225ee604,
        0x2216e584f5fa1ea926041bedfe98,
        0x48a170391f7dc42444e8fa2,
    ];
    let mut ratio = U256::from(1) << 128;
    for (bit, factor) in FACTORS.iter().enumerate() {
        if tick.unsigned_abs() & (1 << bit) != 0 {
            ratio = (ratio * U256::from(*factor)) >> 128;
        }
    }
    if tick > 0 {
        ratio = U256::MAX / ratio;
    }
    Ok((ratio + U256::from(u32::MAX)) >> 32)
}
fn narrow(n: alloy_primitives::U512) -> Result<U256, QuoteError> {
    if n > alloy_primitives::U512::from(U256::MAX) {
        return Err(QuoteError::Overflow);
    }
    Ok(n.to())
}
pub fn quote(pool: &PoolState, asset_in: Address, amount_in: U256) -> Result<Quote, QuoteError> {
    use crate::route::PoolLocator;
    use alloy_primitives::{U512, address};
    let p = &pool.descriptor;
    if !p.is_quoteable()
        || p.protocol != "pons-v2-v4"
        || p.id.chain_id != 4663
        || p.hook != address!("E5e702641Ea86F4ae6cC3cDaeD2B886f976Be044")
        || !matches!(p.id.locator,PoolLocator::Singleton{manager,..} if manager==address!("8366a39cc670b4001a1121b8f6a443a643e40951"))
        || p.lp_fee != 0
        || pool.protocol_fee != 0
    {
        return Err(QuoteError::Unsupported);
    }
    let hook = u32::from(p.hook_fee_bps.ok_or(QuoteError::Unsupported)?);
    let tax = u32::from(p.creator_tax_bps.ok_or(QuoteError::Unsupported)?);
    if hook + tax > 2000 {
        return Err(QuoteError::Unsupported);
    }
    if amount_in.is_zero() {
        return Err(QuoteError::ZeroInput);
    }
    if amount_in > U256::from(i128::MAX as u128) {
        return Err(QuoteError::Overflow);
    }
    if pool.liquidity == 0 {
        return Err(QuoteError::EmptyLiquidity);
    }
    if p.currency0 >= p.currency1
        || p.tick_spacing <= 0
        || p.tick_spacing > 32767
        || !(asset_in == p.currency0 || asset_in == p.currency1)
        || !((p.token == p.currency0 && p.quote_asset == p.currency1)
            || (p.token == p.currency1 && p.quote_asset == p.currency0))
        || pool.tick >= 887272
        || pool.sqrt_price_x96 < sqrt_at_tick(pool.tick)?
        || pool.sqrt_price_x96 > sqrt_at_tick(pool.tick + 1)?
    {
        return Err(QuoteError::InvalidState);
    }
    let zero_for_one = asset_in == p.currency0;
    let compressed = pool.tick.div_euclid(p.tick_spacing) + i32::from(!zero_for_one);
    let word = compressed.div_euclid(256);
    let bit = compressed.rem_euclid(256) as usize;
    let bitmap = pool
        .tick_bitmap
        .get(&(word as i16))
        .ok_or(QuoteError::TickBoundary)?;
    let boundary_bit = if zero_for_one {
        (0..=bit).rev().find(|b| bitmap.bit(*b)).unwrap_or(0)
    } else {
        (bit..256).find(|b| bitmap.bit(*b)).unwrap_or(255)
    };
    let boundary_tick =
        ((word * 256 + boundary_bit as i32) * p.tick_spacing).clamp(-887272, 887272);
    let boundary = U512::from(sqrt_at_tick(boundary_tick)?);
    let price = U512::from(pool.sqrt_price_x96);
    let liquidity = U512::from(pool.liquidity);
    let amount = U512::from(amount_in);
    let q96: U512 = U512::from(1) << 96;
    // ponytail: one proven SwapMath step; reject crossing instead of assuming tick liquidity.
    let next = if zero_for_one {
        let denominator = liquidity * q96 + amount * price;
        // The deployed helper has a different overflow fallback; this verified path rejects it.
        if amount * price > U512::from(U256::MAX) || denominator > U512::from(U256::MAX) {
            return Err(QuoteError::Overflow);
        }
        let numerator = liquidity * q96 * price;
        let (quotient, remainder) = numerator.div_rem(denominator);
        quotient + U512::from(!remainder.is_zero())
    } else {
        price + amount * q96 / liquidity
    };
    if (zero_for_one && next <= boundary) || (!zero_for_one && next >= boundary) {
        return Err(QuoteError::TickBoundary);
    }
    let gross = if zero_for_one {
        liquidity * (price - next) / q96
    } else {
        ((liquidity * q96 * (next - price)) / next) / price
    };
    if gross > U512::from(i128::MAX as u128) {
        return Err(QuoteError::Overflow);
    }
    let hook_fee = gross * U512::from(hook) / U512::from(10000);
    let creator_tax = gross * U512::from(tax) / U512::from(10000);
    Ok(Quote {
        amount_in,
        asset_out: if zero_for_one {
            p.currency1
        } else {
            p.currency0
        },
        amount_out: narrow(gross - hook_fee - creator_tax)?,
        gross_amount_out: narrow(gross)?,
        sqrt_price_after: narrow(next)?,
        included_lp_fee: U256::ZERO,
        included_hook_fee: narrow(hook_fee)?,
        included_creator_tax: narrow(creator_tax)?,
    })
}
