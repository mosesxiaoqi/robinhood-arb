use crate::{
    discovery::{HOOK, MANAGER},
    rpc::{RpcSource, SourceError, hash, quantity},
};
use alloy_primitives::{Address, B256, Bytes, U256, address, b256, keccak256};
use alloy_sol_types::{SolCall, SolValue, sol};
use arb_core::{route::PoolLocator, simulation::*, types::Offset};
use serde_json::{Value, json};
pub const CALLER: Address = address!("000000000000000000000000000000000000cafe");
pub const HELPER: Address = address!("000000000000000000000000000000000000beef");
sol! {
    struct ProbeKey {address currency0;address currency1;uint24 fee;int24 tickSpacing;address hooks;}
    function execute(ProbeKey first,ProbeKey second,uint256 amount,bool failSecond) external payable returns(uint256 middle,uint256 output);
    function inspect(address caller,address token,bytes32 firstId,bytes32 secondId) external view;
}
#[derive(Debug, thiserror::Error)]
pub enum SimulationTransportError {
    #[error("{cause}")]
    WithEvidence {
        cause: Box<SimulationTransportError>,
        evidence: Vec<Vec<u8>>,
    },
    #[error("invalid simulation input: {0}")]
    Input(&'static str),
    #[error("unverifiable simulation response: {0}")]
    Response(&'static str),
    #[error(transparent)]
    Source(#[from] SourceError),
}
impl SimulationTransportError {
    pub fn outcome(&self) -> SimulationOutcome {
        match self {
            Self::WithEvidence { cause, .. } => cause.outcome(),
            Self::Input(_) | Self::Source(SourceError::Rpc(-32601 | -32000, _)) => {
                SimulationOutcome::Unavailable
            }
            _ => SimulationOutcome::Unknown,
        }
    }
}
fn words(value: &Value, count: usize) -> Result<Vec<U256>, SimulationTransportError> {
    let bytes: Bytes = serde_json::from_value(value.clone())
        .map_err(|_| SimulationTransportError::Response("ABI bytes"))?;
    if bytes.len() != count * 32 {
        return Err(SimulationTransportError::Response("ABI length"));
    }
    Ok(bytes.chunks_exact(32).map(U256::from_be_slice).collect())
}
fn key(pool: &SimulationPool) -> ProbeKey {
    ProbeKey {
        currency0: pool.currency0,
        currency1: pool.currency1,
        fee: pool.fee.try_into().expect("validated uint24"),
        tickSpacing: pool
            .tick_spacing
            .try_into()
            .expect("validated positive int24"),
        hooks: pool.hook,
    }
}
fn validate(request: &SimulationRequest) -> Result<[B256; 2], SimulationTransportError> {
    let invalid = || {
        SimulationTransportError::Input(
            "requires the verified native-quote, Pons-to-plain V4 route",
        )
    };
    request.route.validate().map_err(|_| invalid())?;
    if request.route.legs.len() != 2
        || request.position.offset != Offset::BlockEnd
        || request.position.block_hash == B256::ZERO
        || request.position.block_number == u64::MAX
        || request.amount.is_zero()
        || request.amount > U256::from(i128::MAX as u128)
        || request.funding_balance < request.amount
        || request.pools[0].hook != HOOK
        || request.pools[1].hook != Address::ZERO
        || request.pools[0].currency1 != request.pools[1].currency1
    {
        return Err(invalid());
    }
    let mut ids = [B256::ZERO; 2];
    for (index, pool) in request.pools.iter().enumerate() {
        if pool.currency0 != Address::ZERO
            || pool.currency1 == Address::ZERO
            || pool.fee > 1_000_000
            || pool.tick_spacing <= 0
            || pool.tick_spacing > 32767
        {
            return Err(invalid());
        }
        let leg = &request.route.legs[index];
        if leg.pool.chain_id != 4663
            || (index == 0 && (leg.asset_in != Address::ZERO || leg.asset_out != pool.currency1))
            || (index == 1 && (leg.asset_in != pool.currency1 || leg.asset_out != Address::ZERO))
        {
            return Err(invalid());
        }
        let PoolLocator::Singleton { manager, pool_id } = leg.pool.locator else {
            return Err(invalid());
        };
        ids[index] = keccak256(key(pool).abi_encode());
        if manager != MANAGER || ids[index] != pool_id {
            return Err(invalid());
        }
    }
    if ids[0] == ids[1] {
        return Err(invalid());
    }
    Ok(ids)
}
impl RpcSource {
    async fn simulate_inner(
        &self,
        request: &SimulationRequest,
        evidence: &mut Vec<Vec<u8>>,
    ) -> Result<SimulationResult, SimulationTransportError> {
        let ids = validate(request)?;
        if self.chain_id() != 4663 {
            return Err(SimulationTransportError::Input("chain id"));
        }
        let _permit = self
            .gate
            .acquire()
            .await
            .map_err(|_| SimulationTransportError::Response("source closed"))?;
        if self
            .evidence_request("eth_chainId", json!([]), evidence)
            .await?
            != json!("0x1237")
        {
            return Err(SimulationTransportError::Response("chain id"));
        }
        let tag = format!("0x{:x}", request.position.block_number);
        let header = self
            .evidence_request("eth_getBlockByNumber", json!([tag, false]), evidence)
            .await?;
        if hash(&header["hash"])? != request.position.block_hash
            || quantity(&header["number"])? != request.position.block_number
        {
            return Err(SimulationTransportError::Response("base block"));
        }
        for (address, expected) in [
            (
                MANAGER,
                b256!("bd3881180b547f5fe817545743cfb4343e96b1bc6640dcd70c106b0066e95626"),
            ),
            (
                HOOK,
                b256!("c21b1e6c1b45403e81a581f22ed6d9c747997af1cfdac1b1dc9f4b1d346a10db"),
            ),
        ] {
            let code: Bytes = serde_json::from_value(
                self.evidence_request("eth_getCode", json!([address, tag]), evidence)
                    .await?,
            )
            .map_err(|_| SimulationTransportError::Response("runtime code"))?;
            if keccak256(code) != expected {
                return Err(SimulationTransportError::Response(
                    "changed protocol runtime",
                ));
            }
        }
        for address in [CALLER, HELPER] {
            if self
                .evidence_request("eth_getCode", json!([address, tag]), evidence)
                .await?
                != json!("0x")
            {
                return Err(SimulationTransportError::Input(
                    "probe addresses must have no real code",
                ));
            }
        }
        let artifact: Value =
            serde_json::from_str(include_str!("../../../simulation/AtomicProbe.json"))
                .map_err(|_| SimulationTransportError::Input("compiled probe artifact"))?;
        let inspect: Bytes = inspectCall {
            caller: CALLER,
            token: request.pools[0].currency1,
            firstId: ids[0],
            secondId: ids[1],
        }
        .abi_encode()
        .into();
        let execute: Bytes = executeCall {
            first: key(&request.pools[0]),
            second: key(&request.pools[1]),
            amount: request.amount,
            failSecond: request.fail_second,
        }
        .abi_encode()
        .into();
        let read =
            json!({"from":CALLER,"to":HELPER,"data":inspect,"gas":"0x1e8480","gasPrice":"0x0"});
        let action = json!({"from":CALLER,"to":HELPER,"data":execute,"value":request.amount,"gas":"0x1e8480","gasPrice":"0x0"});
        let payload = json!({"blockStateCalls":[{"blockOverrides":{"baseFeePerGas":"0x0"},"stateOverrides":{CALLER.to_string():{"balance":request.funding_balance},HELPER.to_string():{"code":artifact["runtime_code"]}},"calls":[read,action,read]}],"validation":false,"traceTransfers":true});
        let response = self
            .evidence_request("eth_simulateV1", json!([payload, tag]), evidence)
            .await?;
        let end = self
            .evidence_request("eth_getBlockByNumber", json!([tag, false]), evidence)
            .await?;
        if hash(&end["hash"])? != request.position.block_hash {
            return Err(SimulationTransportError::Response("base block changed"));
        }
        parse_result(request, &response, vec![])
    }
}
pub fn parse_result(
    request: &SimulationRequest,
    response: &Value,
    evidence: Vec<Vec<u8>>,
) -> Result<SimulationResult, SimulationTransportError> {
    let ids = validate(request)?;
    let invalid =
        || SimulationTransportError::Response("atomic state, call results or asset changes");
    let blocks = response.as_array().ok_or_else(invalid)?;
    if blocks.len() != 1 {
        return Err(invalid());
    }
    let block = &blocks[0];
    if hash(&block["parentHash"])? != request.position.block_hash
        || quantity(&block["number"])? != request.position.block_number + 1
    {
        return Err(invalid());
    }
    let calls = block["calls"].as_array().ok_or_else(invalid)?;
    if calls.len() != 3 || calls[0]["status"] != "0x1" || calls[2]["status"] != "0x1" {
        return Err(invalid());
    }
    let before = words(&calls[0]["returnData"], 6)?;
    let after = words(&calls[2]["returnData"], 6)?;
    if before[0] != request.funding_balance {
        return Err(invalid());
    }
    let gas_used = quantity(&calls[1]["gasUsed"])?;
    let outcome = match calls[1]["status"].as_str() {
        Some("0x1") => SimulationOutcome::Succeeded,
        Some("0x0") => SimulationOutcome::Reverted,
        _ => return Err(invalid()),
    };
    if request.fail_second && outcome == SimulationOutcome::Succeeded {
        return Err(invalid());
    }
    let (middle_amount, amount_out) = if outcome == SimulationOutcome::Succeeded {
        let result = words(&calls[1]["returnData"], 2)?;
        let middle = result[0];
        let output = result[1];
        if before[0] != request.funding_balance
            || after[0]
                != before[0]
                    .checked_sub(request.amount)
                    .and_then(|v| v.checked_add(output))
                    .ok_or_else(invalid)?
            || before[1..4] != after[1..4]
        {
            return Err(invalid());
        }
        let logs = calls[1]["logs"].as_array().ok_or_else(invalid)?;
        let mut swaps = vec![];
        let mut fee = U256::ZERO;
        for log in logs {
            let address: Address =
                serde_json::from_value(log["address"].clone()).map_err(|_| invalid())?;
            if address == MANAGER
                && log["topics"][0]
                    == json!(keccak256(
                        "Swap(bytes32,address,int128,int128,uint160,uint128,int24,uint24)"
                    ))
            {
                swaps.push((hash(&log["topics"][1])?, words(&log["data"], 6)?));
            }
            if address == HOOK
                && log["topics"][0]
                    == json!(keccak256(
                        "HookFeeCollected(bytes32,address,uint256,uint256)"
                    ))
                && log["topics"][1] == json!(ids[0])
            {
                let values = words(&log["data"], 3)?;
                fee = fee
                    .checked_add(values[1])
                    .and_then(|n| n.checked_add(values[2]))
                    .ok_or_else(invalid)?;
            }
        }
        if swaps.len() != 2
            || swaps[0].0 != ids[0]
            || swaps[1].0 != ids[1]
            || swaps[0].1[0] != U256::ZERO.wrapping_sub(request.amount)
            || swaps[0].1[1].checked_sub(fee) != Some(middle)
            || swaps[1].1[1] != U256::ZERO.wrapping_sub(middle)
            || swaps[1].1[0] != output
            || (after[4] & ((U256::from(1) << 160) - U256::from(1))) != swaps[0].1[2]
            || (after[5] & ((U256::from(1) << 160) - U256::from(1))) != swaps[1].1[2]
        {
            return Err(invalid());
        }
        (Some(middle), Some(output))
    } else {
        if before != after || !calls[1]["logs"].as_array().ok_or_else(invalid)?.is_empty() {
            return Err(invalid());
        }
        (None, None)
    };
    let asset_changes = (0..4)
        .map(|i| AssetChange {
            account: if i < 2 { CALLER } else { HELPER },
            asset: if i % 2 == 0 {
                Address::ZERO
            } else {
                request.pools[0].currency1
            },
            before: before[i],
            after: after[i],
        })
        .collect();
    let pool_slots_before = [
        B256::from(before[4].to_be_bytes::<32>()),
        B256::from(before[5].to_be_bytes::<32>()),
    ];
    let pool_slots_after = [
        B256::from(after[4].to_be_bytes::<32>()),
        B256::from(after[5].to_be_bytes::<32>()),
    ];
    Ok(SimulationResult {
        request: request.clone(),
        actual_position: request.position.clone(),
        simulated_block_hash: hash(&block["hash"])?,
        state_rolled_back: outcome == SimulationOutcome::Reverted,
        outcome,
        middle_amount,
        amount_out,
        gas_used,
        asset_changes,
        pool_slots_before,
        pool_slots_after,
        error: calls[1].get("error").map(Value::to_string),
        evidence,
    })
}

impl RpcSource {
    pub async fn simulate(
        &self,
        request: &SimulationRequest,
    ) -> Result<SimulationResult, SimulationTransportError> {
        let mut evidence = vec![];
        match self.simulate_inner(request, &mut evidence).await {
            Ok(mut result) => {
                result.evidence = evidence;
                Ok(result)
            }
            Err(cause) => Err(SimulationTransportError::WithEvidence {
                cause: Box::new(cause),
                evidence,
            }),
        }
    }
}
impl SimulationTransportError {
    pub fn evidence(&self) -> &[Vec<u8>] {
        match self {
            Self::WithEvidence { evidence, .. } => evidence,
            _ => &[],
        }
    }
}
