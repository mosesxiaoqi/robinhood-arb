use alloy_primitives::{Address, B256, U256};
use arb_core::{
    research::DerivedBlock,
    route::Route,
    simulation::{SimulationRecord, validates_candidate},
    types::ChainPosition,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub struct OpportunitySummary {
    pub run_id: String,
    pub chain_id: u64,
    pub quote_asset: Address,
    pub route: Route,
    pub start: ChainPosition,
    pub end: ChainPosition,
    pub blocks: u64,
    pub sampled_capacity: U256,
    pub simulated_capacity: Option<U256>,
}
#[derive(Debug, Default, Serialize)]
pub struct OpportunityAnalysis {
    pub from: u64,
    pub to: u64,
    pub complete_blocks: u64,
    pub gap_blocks: u64,
    pub orphan_blocks: u64,
    pub token_block_samples: u64,
    pub excluded_pool_samples: u64,
    pub exclusions: BTreeMap<String, u64>,
    pub windows: Vec<OpportunitySummary>,
}
/// Complete views, including failed/zero-candidate views, are necessary to measure coverage.
/// A caller must bound the loaded bytes as well as this 10,000-block analysis window.
pub fn summarize_opportunities(
    blocks: &[DerivedBlock],
    simulations: &[SimulationRecord],
    from: u64,
    to: u64,
) -> Result<OpportunityAnalysis, &'static str> {
    if from > to || to - from >= 10000 || blocks.len() > 20000 || simulations.len() > 100000 {
        return Err("analysis window budget");
    }
    let mut output = OpportunityAnalysis {
        from,
        to,
        ..Default::default()
    };
    let mut ordered = blocks
        .iter()
        .filter(|b| (from..=to).contains(&b.view.position.block_number))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|b| {
        (
            &b.run_id,
            b.view.pools.first().map(|p| p.descriptor.id.chain_id),
            b.view.position.block_number,
        )
    });
    let mut groups = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut open = BTreeMap::<String, (usize, B256)>::new();
    let mut simulation_index = BTreeMap::<(&str, B256), Vec<&SimulationRecord>>::new();
    for s in simulations {
        simulation_index
            .entry((&s.run_id, s.candidate_id))
            .or_default()
            .push(s);
    }
    for block in ordered {
        let chain = block
            .view
            .pools
            .first()
            .ok_or("view has no pools")?
            .descriptor
            .id
            .chain_id;
        groups.insert((&block.run_id, chain));
        if !block.canonical {
            output.orphan_blocks += 1;
            continue;
        }
        if block.batch.position != block.view.position
            || !seen.insert((&block.run_id, chain, block.view.position.block_number))
        {
            return Err("conflicting canonical view");
        }
        output.complete_blocks += 1;
        output.token_block_samples += block
            .view
            .pools
            .iter()
            .map(|p| (p.descriptor.id.chain_id, p.descriptor.token))
            .collect::<BTreeSet<_>>()
            .len() as u64;
        output.excluded_pool_samples += block.excluded_pools as u64;
        for e in &block.exclusions {
            for e in &e.entries {
                *output.exclusions.entry(e.reason.clone()).or_default() += 1;
            }
        }
        let mut samples =
            BTreeMap::<String, (&arb_core::research::Candidate, U256, Option<U256>)>::new();
        for c in &block.candidates {
            if !c.canonical
                || c.run_id != block.run_id
                || c.view_id != block.view_id
                || c.opportunity.position != block.view.position
                || c.opportunity.net_profit.as_ref().is_none_or(|p| p.negative)
            {
                *output
                    .exclusions
                    .entry("candidate unknown or invalid association".into())
                    .or_default() += 1;
                continue;
            }
            c.opportunity
                .route
                .validate()
                .map_err(|_| "invalid candidate route")?;
            let key = serde_json::to_string(&(&c.run_id, &c.opportunity.route))
                .map_err(|_| "route encoding")?;
            let amount = c.opportunity.amount_in;
            let verified = simulation_index
                .get(&(c.run_id.as_str(), c.id))
                .is_some_and(|records| {
                    records.iter().any(|s| {
                        s.canonical
                            && s.validates_original_candidate
                            && s.view_id == c.view_id
                            && s.result.as_ref().is_some_and(|r| validates_candidate(c, r))
                    })
                });
            let entry = samples.entry(key).or_insert((c, amount, None));
            entry.1 = entry.1.max(amount);
            if verified {
                entry.2 = Some(entry.2.unwrap_or_default().max(amount));
            }
        }
        for (key, (c, capacity, verified)) in samples {
            let previous = open.get(&key).copied().filter(|(i, hash)| {
                output.windows[*i].end.block_number.checked_add(1)
                    == Some(block.view.position.block_number)
                    && *hash == block.batch.parent_hash
            });
            let index = if let Some((index, _)) = previous {
                let window = &mut output.windows[index];
                window.end = block.view.position.clone();
                window.blocks += 1;
                window.sampled_capacity = window.sampled_capacity.max(capacity);
                window.simulated_capacity = window.simulated_capacity.max(verified);
                index
            } else {
                output.windows.push(OpportunitySummary {
                    run_id: c.run_id.clone(),
                    chain_id: chain,
                    quote_asset: c.opportunity.route.legs[0].asset_in,
                    route: c.opportunity.route.clone(),
                    start: block.view.position.clone(),
                    end: block.view.position.clone(),
                    blocks: 1,
                    sampled_capacity: capacity,
                    simulated_capacity: verified,
                });
                output.windows.len() - 1
            };
            open.insert(key, (index, block.view.position.block_hash));
        }
    }
    output.gap_blocks = (to - from + 1) * (groups.len().max(1) as u64) - output.complete_blocks;
    Ok(output)
}
