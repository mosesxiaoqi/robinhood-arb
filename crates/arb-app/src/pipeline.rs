use arb_adapters::store::{Store, StoreError};
use arb_core::{
    opportunity::Opportunity,
    research::RunSpec,
    state::{BlockBatch, State, StateError, StateView},
};
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("invalid pipeline input: {0}")]
    Invalid(&'static str),
    #[error("pipeline serialization failed")]
    Json(#[from] serde_json::Error),
}
pub struct Pipeline {
    store: Store,
    state: State,
    run: RunSpec,
}
impl Pipeline {
    pub fn new(mut store: Store, state: State, run: RunSpec) -> Result<Self, PipelineError> {
        state
            .validate()
            .map_err(|_| PipelineError::Invalid("state"))?;
        store.register_run(&run)?;
        Ok(Self { store, state, run })
    }
    pub fn view(&self) -> StateView {
        self.state.view()
    }
    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn process(
        &mut self,
        batch: BlockBatch,
        observed_at: u64,
    ) -> Result<Vec<Opportunity>, PipelineError> {
        use alloy_primitives::keccak256;
        use arb_core::{
            opportunity::scan,
            research::{Candidate, DerivedBlock, RouteExclusions, SimulationStatus},
            route::route_candidates,
        };
        let mut next = self.state.clone();
        let view = next.apply_block(&batch)?;
        if let Some(existing) = self
            .store
            .find_derived(&self.run.run_id, view.position.block_hash)?
        {
            if existing.batch != batch
                || existing.view != view
                || existing.candidates.iter().any(|c| !c.canonical)
            {
                return Err(PipelineError::Invalid("processed block conflict"));
            }
            self.state = next;
            return Ok(existing
                .candidates
                .into_iter()
                .map(|c| c.opportunity)
                .collect());
        }
        let view_id = keccak256(serde_json::to_vec(&view)?);
        let routes = route_candidates(
            &view
                .pools
                .iter()
                .map(|p| p.descriptor.clone())
                .collect::<Vec<_>>(),
            self.run.quote_asset,
        );
        let mut block = DerivedBlock {
            run_id: self.run.run_id.clone(),
            view_id,
            batch,
            view,
            candidates: vec![],
            exclusions: vec![],
            excluded_pools: routes.excluded_pools,
        };
        for route in routes.routes {
            let scanned = scan(
                &block.view,
                &route,
                &self.run.amounts,
                &self.run.costs,
                self.run.min_depth,
                self.run.min_profit,
            );
            for opportunity in scanned.opportunities {
                let id = keccak256(serde_json::to_vec(&(
                    &self.run.run_id,
                    view_id,
                    &route,
                    opportunity.amount_in,
                ))?);
                block.candidates.push(Candidate {
                    id,
                    run_id: self.run.run_id.clone(),
                    view_id,
                    config_hash: self.run.config_hash,
                    algorithm_version: self.run.algorithm_version.clone(),
                    raw_refs: block.view.raw_refs.clone(),
                    detected_at_ms: observed_at,
                    opportunity,
                    simulation_status: SimulationStatus::NotRun,
                    canonical: true,
                });
            }
            block.exclusions.push(RouteExclusions {
                route,
                entries: scanned.excluded,
            });
        }
        self.store.save_derived(&block)?;
        self.state = next;
        Ok(block
            .candidates
            .into_iter()
            .map(|c| c.opportunity)
            .collect())
    }
}
