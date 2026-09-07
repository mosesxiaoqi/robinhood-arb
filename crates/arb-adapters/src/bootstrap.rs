use crate::{
    discovery::{FACTORY, HOOK, MANAGER},
    rpc::{RpcSource, SourceError},
};
use alloy_primitives::{Address, B256, Bytes, U256, b256, keccak256};
use alloy_sol_types::SolValue;
use arb_core::{
    protocol::{Bootstrap, PoolState},
    route::PoolLocator,
    types::{ChainPosition, Offset, PoolDescriptor, PoolVerification},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn invalid() -> SourceError {
    SourceError::Invalid("bootstrap state or metadata mismatch")
}
fn words(v: Value, n: usize) -> Result<Vec<U256>, SourceError> {
    let bytes: Bytes = serde_json::from_value(v).map_err(|_| invalid())?;
    if bytes.len() != n * 32 {
        return Err(invalid());
    }
    Ok(bytes.chunks_exact(32).map(U256::from_be_slice).collect())
}
fn address_word(a: Address) -> U256 {
    U256::from_be_slice(a.as_slice())
}
fn calldata(signature: &str, args: Vec<u8>) -> Bytes {
    let mut bytes = keccak256(signature).as_slice()[..4].to_vec();
    bytes.extend(args);
    bytes.into()
}
impl RpcSource {
    async fn fixed_call(
        &self,
        address: Address,
        signature: &str,
        args: Vec<u8>,
        tag: &str,
        n: usize,
        evidence: &mut Vec<Vec<u8>>,
    ) -> Result<Vec<U256>, SourceError> {
        words(
            self.evidence_request(
                "eth_call",
                json!([{"to":address,"data":calldata(signature,args)},tag]),
                evidence,
            )
            .await?,
            n,
        )
    }
    pub async fn bootstrap(
        &self,
        pools: &[PoolDescriptor],
        at: ChainPosition,
    ) -> Result<Bootstrap, SourceError> {
        if self.chain_id() != 4663
            || pools.is_empty()
            || pools.len() > 128
            || at.offset != Offset::BlockEnd
            || at.block_hash == B256::ZERO
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        for p in pools {
            if p.id.chain_id != 4663
                || !ids.insert(p.id.clone())
                || p.hook != HOOK
                || p.tick_spacing <= 0
                || p.tick_spacing > 32767
                || p.initialized_at.block_number > at.block_number
                || p.currency0 >= p.currency1
            {
                return Err(invalid());
            }
        }
        let _permit = self.gate.acquire().await.map_err(|_| invalid())?;
        let mut evidence = vec![];
        let chain = self
            .evidence_request("eth_chainId", json!([]), &mut evidence)
            .await?;
        if chain != json!("0x1237") {
            return Err(invalid());
        }
        let tag = format!("0x{:x}", at.block_number);
        let header = self
            .evidence_request("eth_getBlockByNumber", json!([tag, false]), &mut evidence)
            .await?;
        if header["hash"] != json!(at.block_hash) || header["number"] != json!(tag) {
            return Err(invalid());
        }
        // Frozen T09 runtime fingerprints. Manager is bytecode-pinned, not independently recompiled.
        for (address, expected) in [
            (
                FACTORY,
                b256!("89a27da6f703e0a7cdd4f233e7cb57604ff75b164530962d3ff7cf8483a67d84"),
            ),
            (
                HOOK,
                b256!("c21b1e6c1b45403e81a581f22ed6d9c747997af1cfdac1b1dc9f4b1d346a10db"),
            ),
            (
                MANAGER,
                b256!("bd3881180b547f5fe817545743cfb4343e96b1bc6640dcd70c106b0066e95626"),
            ),
        ] {
            let code: Bytes = serde_json::from_value(
                self.evidence_request("eth_getCode", json!([address, tag]), &mut evidence)
                    .await?,
            )
            .map_err(|_| invalid())?;
            if keccak256(code) != expected {
                return Err(invalid());
            }
        }
        let mut states = vec![];
        for pool in pools {
            let PoolLocator::Singleton { manager, pool_id } = pool.id.locator else {
                return Err(invalid());
            };
            if manager != MANAGER {
                return Err(invalid());
            }
            let mut descriptor = pool.clone();
            let launch = self
                .fixed_call(
                    FACTORY,
                    "getLaunchedToken(address)",
                    pool.token.abi_encode(),
                    &tag,
                    15,
                    &mut evidence,
                )
                .await?;
            let fees = self
                .fixed_call(
                    HOOK,
                    "launches(bytes32)",
                    pool_id.abi_encode(),
                    &tag,
                    13,
                    &mut evidence,
                )
                .await?;
            let expected_id = keccak256(
                (
                    pool.currency0,
                    pool.currency1,
                    U256::from(pool.lp_fee),
                    U256::from(pool.tick_spacing as u32),
                    HOOK,
                )
                    .abi_encode(),
            );
            if pool_id != expected_id
                || launch[0] != address_word(pool.token)
                || launch[4] != address_word(pool.quote_asset)
                || launch[6] != U256::from(pool.lp_fee)
                || launch[7] != U256::from(pool.tick_spacing as u32)
                || launch[10] != U256::from(2)
                || launch[14] != U256::from(1)
                || fees[0] != U256::from(1)
                || fees[1] != U256::from((pool.token == pool.currency0) as u8)
                || fees[2] != address_word(pool.token)
                || fees[3] != address_word(pool.quote_asset)
                || fees[7] != launch[8]
                || fees[7] > U256::from(2000)
                || fees[10] > U256::from(2000)
                || fees[7] + fees[10] > U256::from(2000)
            {
                return Err(invalid());
            }
            descriptor.hook_fee_bps = Some(fees[10].to());
            descriptor.creator_tax_bps = Some(fees[7].to());
            for (asset, decimals) in [
                (pool.token, &mut descriptor.token_decimals),
                (pool.quote_asset, &mut descriptor.quote_decimals),
            ] {
                let d = if asset == Address::ZERO {
                    U256::from(18)
                } else {
                    self.fixed_call(asset, "decimals()", vec![], &tag, 1, &mut evidence)
                        .await?[0]
                };
                if d > U256::from(255) {
                    return Err(invalid());
                }
                *decimals = Some(d.to());
            }
            // Uniswap v4 StateLibrary (MIT): pools mapping slot 6, liquidity +3, bitmap +5.
            let base = U256::from_be_bytes(keccak256((pool_id, U256::from(6)).abi_encode()).0);
            let slot = self
                .fixed_call(
                    MANAGER,
                    "extsload(bytes32)",
                    base.abi_encode(),
                    &tag,
                    1,
                    &mut evidence,
                )
                .await?[0];
            let liquidity = self
                .fixed_call(
                    MANAGER,
                    "extsload(bytes32)",
                    (base.wrapping_add(U256::from(3))).abi_encode(),
                    &tag,
                    1,
                    &mut evidence,
                )
                .await?[0];
            let sqrt_price_x96 = slot & ((U256::from(1) << 160) - U256::from(1));
            let tick_bits = ((slot >> 160usize) & U256::from(0xffffff)).to::<u32>();
            let tick = ((tick_bits << 8) as i32) >> 8;
            let protocol_fee = ((slot >> 184usize) & U256::from(0xffffff)).to::<u32>();
            let lp_fee = ((slot >> 208usize) & U256::from(0xffffff)).to::<u32>();
            if sqrt_price_x96 == U256::ZERO
                || !(-887272..=887272).contains(&tick)
                || liquidity > U256::from(u128::MAX)
                || lp_fee != pool.lp_fee
            {
                return Err(invalid());
            }
            let mut tick_bitmap = BTreeMap::new();
            let compressed = tick.div_euclid(pool.tick_spacing);
            // ponytail: quote only inside loaded bitmap boundaries; load adjacent words when crossing is required.
            for index in [compressed.div_euclid(256), (compressed + 1).div_euclid(256)] {
                let index = i16::try_from(index).map_err(|_| invalid())?;
                if tick_bitmap.contains_key(&index) {
                    continue;
                }
                let key = keccak256((index, base.wrapping_add(U256::from(5))).abi_encode());
                let bitmap = self
                    .fixed_call(
                        MANAGER,
                        "extsload(bytes32)",
                        key.abi_encode(),
                        &tag,
                        1,
                        &mut evidence,
                    )
                    .await?[0];
                tick_bitmap.insert(index, bitmap);
            }
            descriptor.verification = if lp_fee == 0 && protocol_fee == 0 {
                PoolVerification::Supported
            } else {
                PoolVerification::Unsupported("nonzero pool fee".into())
            };
            states.push(PoolState {
                descriptor,
                sqrt_price_x96,
                tick,
                liquidity: liquidity.to(),
                protocol_fee,
                tick_bitmap,
            });
        }
        let end = self
            .evidence_request("eth_getBlockByNumber", json!([tag, false]), &mut evidence)
            .await?;
        if end["hash"] != header["hash"] || end["number"] != header["number"] {
            return Err(invalid());
        }
        Ok(Bootstrap {
            version: 1,
            position: at,
            pools: states,
            evidence,
        })
    }
}
