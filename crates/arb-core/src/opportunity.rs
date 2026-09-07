use crate::{protocol::verified::Quote, route::Route, state::StateView, types::ChainPosition};
use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAmount {
    pub negative: bool,
    pub magnitude: U256,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversion {
    pub quote_asset: Address,
    pub numerator: U256,
    pub denominator: U256,
    pub position: ChainPosition,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub asset: Address,
    pub amount: Option<U256>,
    pub conversion: Option<Conversion>,
    pub basis: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opportunity {
    pub route: Route,
    pub position: ChainPosition,
    pub amount_in: U256,
    pub amount_out: U256,
    pub quotes: Vec<Quote>,
    pub gross_profit: SignedAmount,
    pub net_profit: Option<SignedAmount>,
    pub costs: CostEstimate,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvaluateError {
    #[error("invalid route, view or cost estimate")]
    Invalid,
    #[error("arithmetic overflow")]
    Overflow,
    #[error(transparent)]
    Quote(#[from] crate::protocol::verified::QuoteError),
}

pub fn profit(input: U256, output: U256, cost: U256) -> Result<SignedAmount, EvaluateError> {
    if output >= input {
        let gross = output - input;
        Ok(if gross >= cost {
            SignedAmount {
                negative: false,
                magnitude: gross - cost,
            }
        } else {
            SignedAmount {
                negative: true,
                magnitude: cost - gross,
            }
        })
    } else {
        Ok(SignedAmount {
            negative: true,
            magnitude: (input - output)
                .checked_add(cost)
                .ok_or(EvaluateError::Overflow)?,
        })
    }
}
impl CostEstimate {
    pub fn in_quote_asset(
        &self,
        quote: Address,
        position: &ChainPosition,
    ) -> Result<Option<U256>, EvaluateError> {
        use alloy_primitives::U512;
        if self.basis.is_empty() || self.basis.len() > 1024 {
            return Err(EvaluateError::Invalid);
        }
        let Some(amount) = self.amount else {
            return Ok(None);
        };
        if self.asset == quote {
            return Ok(Some(amount));
        }
        let Some(c) = &self.conversion else {
            return Ok(None);
        };
        if c.quote_asset != quote || c.position != *position {
            return Ok(None);
        }
        if c.numerator.is_zero() || c.denominator.is_zero() {
            return Err(EvaluateError::Invalid);
        }
        let (value, remainder) =
            (U512::from(amount) * U512::from(c.numerator)).div_rem(U512::from(c.denominator));
        let value = value + U512::from(!remainder.is_zero());
        if value > U512::from(U256::MAX) {
            return Err(EvaluateError::Overflow);
        }
        Ok(Some(value.to()))
    }
}
pub fn evaluate(
    view: &StateView,
    route: &Route,
    amount: U256,
    costs: &CostEstimate,
) -> Result<Opportunity, EvaluateError> {
    use crate::protocol::verified::quote;
    route.validate().map_err(|_| EvaluateError::Invalid)?;
    if route.legs.len() != 2
        || route.legs[0].pool == route.legs[1].pool
        || view.position.offset != crate::types::Offset::BlockEnd
    {
        return Err(EvaluateError::Invalid);
    }
    let mut current = amount;
    let mut quotes = vec![];
    for leg in &route.legs {
        let pools = view
            .pools
            .iter()
            .filter(|p| p.descriptor.id == leg.pool)
            .collect::<Vec<_>>();
        if pools.len() != 1 || pools[0].descriptor.protocol != leg.protocol {
            return Err(EvaluateError::Invalid);
        }
        let q = quote(pools[0], leg.asset_in, current)?;
        if q.asset_out != leg.asset_out {
            return Err(EvaluateError::Invalid);
        }
        current = q.amount_out;
        quotes.push(q);
    }
    let gross_profit = profit(amount, current, U256::ZERO)?;
    let net_profit = costs
        .in_quote_asset(route.legs[0].asset_in, &view.position)?
        .map(|cost| profit(amount, current, cost))
        .transpose()?;
    Ok(Opportunity {
        route: route.clone(),
        position: view.position.clone(),
        amount_in: amount,
        amount_out: current,
        quotes,
        gross_profit,
        net_profit,
        costs: costs.clone(),
    })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclusion {
    pub amount: U256,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanResult {
    pub opportunities: Vec<Opportunity>,
    pub excluded: Vec<Exclusion>,
    pub min_depth: U256,
    pub min_profit: U256,
}
pub fn scan(
    view: &StateView,
    route: &Route,
    amounts: &[U256],
    costs: &CostEstimate,
    min_depth: U256,
    min_profit: U256,
) -> ScanResult {
    use crate::protocol::verified::quote_depth;
    let mut result = ScanResult {
        opportunities: vec![],
        excluded: vec![],
        min_depth,
        min_profit,
    };
    let depth = route
        .legs
        .first()
        .and_then(|first| {
            route
                .legs
                .iter()
                .map(|leg| {
                    view.pools
                        .iter()
                        .find(|p| p.descriptor.id == leg.pool)
                        .and_then(|pool| quote_depth(pool, first.asset_in).ok())
                })
                .collect::<Option<Vec<_>>>()
        })
        .and_then(|depths| depths.into_iter().min());
    for &amount in amounts {
        let reason = if depth.is_none_or(|d| d < min_depth) {
            Some("insufficient proven local quote depth".into())
        } else {
            match evaluate(view, route, amount, costs) {
                Ok(opportunity) => {
                    let accepted = opportunity
                        .net_profit
                        .as_ref()
                        .is_some_and(|p| !p.negative && p.magnitude >= min_profit);
                    if accepted {
                        result.opportunities.push(opportunity);
                        None
                    } else {
                        Some(
                            if opportunity.net_profit.is_none() {
                                "unknown additional costs"
                            } else {
                                "below net profit threshold"
                            }
                            .into(),
                        )
                    }
                }
                Err(error) => Some(error.to_string()),
            }
        };
        if let Some(reason) = reason {
            result.excluded.push(Exclusion { amount, reason });
        }
    }
    result
}
