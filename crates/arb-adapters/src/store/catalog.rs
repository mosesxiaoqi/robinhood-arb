use super::*;
use arb_core::{
    checkpoint::{Checkpoint, ProcessingCursor},
    protocol::Bootstrap,
    research::RunSpec,
    route::{PoolId, PoolLocator},
    types::{ChainPosition, Offset, PoolDescriptor, PoolVerification},
};

fn scalar<T: std::str::FromStr>(row: &rusqlite::Row<'_>, name: &str) -> Result<T, StoreError> {
    row.get::<_, String>(name)?
        .parse()
        .map_err(|_| StoreError::Invalid("invalid stored catalog scalar"))
}
fn integer(row: &rusqlite::Row<'_>, name: &str) -> Result<u64, StoreError> {
    row.get::<_, i64>(name)?
        .try_into()
        .map_err(|_| StoreError::Invalid("negative catalog integer"))
}
fn json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    name: &str,
) -> Result<T, StoreError> {
    let value = row.get_ref(name)?;
    let text = value
        .as_str()
        .map_err(|_| StoreError::Invalid("catalog JSON must be text"))?;
    Ok(serde_json::from_str(text)?)
}
fn read_budget(row: &rusqlite::Row<'_>, fields: &[&str]) -> Result<usize, StoreError> {
    let mut bytes = 0usize;
    for field in fields {
        match row.get_ref(*field)? {
            rusqlite::types::ValueRef::Text(text) => bytes = bytes.saturating_add(text.len()),
            rusqlite::types::ValueRef::Null => {}
            _ => return Err(StoreError::Invalid("catalog text column type")),
        }
    }
    if bytes > 64 * 1024 * 1024 {
        return Err(StoreError::Invalid("catalog record exceeds storage budget"));
    }
    Ok(bytes)
}
const CHECKPOINT_TEXT_FIELDS: &[&str] = &[
    "research_run_id",
    "registry_version",
    "config_hash",
    "algorithm_version",
    "last_raw_id",
    "state_json",
];
fn require_null(row: &rusqlite::Row<'_>, fields: &[&str]) -> Result<(), StoreError> {
    for field in fields {
        if !matches!(row.get_ref(*field)?, rusqlite::types::ValueRef::Null) {
            return Err(StoreError::Invalid("unexpected catalog variant field"));
        }
    }
    Ok(())
}
fn budget<T: serde::Serialize>(value: &T) -> Result<usize, StoreError> {
    let size = serde_json::to_vec(value)?.len();
    if size > 64 * 1024 * 1024 {
        return Err(StoreError::Invalid("catalog record exceeds storage budget"));
    }
    Ok(size)
}

pub(super) fn insert_pool(
    connection: &Connection,
    pool: &PoolDescriptor,
) -> Result<(), StoreError> {
    let (kind, address, manager, pool_id) = match pool.id.locator {
        PoolLocator::Contract(a) => ("contract", Some(a.to_string()), None, None),
        PoolLocator::Singleton { manager, pool_id } => (
            "singleton",
            None,
            Some(manager.to_string()),
            Some(pool_id.to_string()),
        ),
    };
    let (offset, index, log) = match pool.initialized_at.offset {
        Offset::BlockEnd => ("block_end", None, None),
        Offset::Transaction { index, log_index } => (
            "transaction",
            Some(index.to_string()),
            log_index.map(|v| v.to_string()),
        ),
    };
    let (verification, reason) = match &pool.verification {
        PoolVerification::Pending => ("pending", None),
        PoolVerification::Supported => ("supported", None),
        PoolVerification::Unsupported(reason) => ("unsupported", Some(reason)),
    };
    connection.execute("INSERT INTO pool_registry(key,chain_id,locator_kind,address,manager,pool_id,protocol,token,quote_asset,currency0,currency1,hook,lp_fee,tick_spacing,hook_fee_bps,creator_tax_bps,token_decimals,quote_decimals,initialized_block_number,initialized_block_hash,initialized_offset_kind,initialized_transaction_index,initialized_log_index,verification_status,verification_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25) ON CONFLICT(key) DO NOTHING", params![serde_json::to_string(&pool.id)?,pool.id.chain_id.to_string(),kind,address,manager,pool_id,pool.protocol,pool.token.to_string(),pool.quote_asset.to_string(),pool.currency0.to_string(),pool.currency1.to_string(),pool.hook.to_string(),pool.lp_fee,pool.tick_spacing,pool.hook_fee_bps,pool.creator_tax_bps,pool.token_decimals,pool.quote_decimals,sql_block_number(pool.initialized_at.block_number)?,pool.initialized_at.block_hash.to_string(),offset,index,log,verification,reason])?;
    Ok(())
}
fn pool_row(row: &rusqlite::Row<'_>) -> Result<PoolDescriptor, StoreError> {
    let locator = match row.get::<_, String>("locator_kind")?.as_str() {
        "contract" => {
            require_null(row, &["manager", "pool_id"])?;
            PoolLocator::Contract(scalar(row, "address")?)
        }
        "singleton" => {
            require_null(row, &["address"])?;
            PoolLocator::Singleton {
                manager: scalar(row, "manager")?,
                pool_id: scalar(row, "pool_id")?,
            }
        }
        _ => return Err(StoreError::Invalid("pool locator kind")),
    };
    let offset = match row.get::<_, String>("initialized_offset_kind")?.as_str() {
        "block_end" => {
            require_null(
                row,
                &["initialized_transaction_index", "initialized_log_index"],
            )?;
            Offset::BlockEnd
        }
        "transaction" => Offset::Transaction {
            index: scalar(row, "initialized_transaction_index")?,
            log_index: row
                .get::<_, Option<String>>("initialized_log_index")?
                .map(|v| v.parse().map_err(|_| StoreError::Invalid("pool log index")))
                .transpose()?,
        },
        _ => return Err(StoreError::Invalid("pool offset kind")),
    };
    let verification = match row.get::<_, String>("verification_status")?.as_str() {
        "pending" => {
            require_null(row, &["verification_reason"])?;
            PoolVerification::Pending
        }
        "supported" => {
            require_null(row, &["verification_reason"])?;
            PoolVerification::Supported
        }
        "unsupported" => PoolVerification::Unsupported(row.get("verification_reason")?),
        _ => return Err(StoreError::Invalid("pool verification status")),
    };
    let pool = PoolDescriptor {
        id: PoolId {
            chain_id: scalar(row, "chain_id")?,
            locator,
        },
        protocol: row.get("protocol")?,
        token: scalar(row, "token")?,
        quote_asset: scalar(row, "quote_asset")?,
        currency0: scalar(row, "currency0")?,
        currency1: scalar(row, "currency1")?,
        hook: scalar(row, "hook")?,
        lp_fee: row.get("lp_fee")?,
        tick_spacing: row.get("tick_spacing")?,
        hook_fee_bps: row.get("hook_fee_bps")?,
        creator_tax_bps: row.get("creator_tax_bps")?,
        token_decimals: row.get("token_decimals")?,
        quote_decimals: row.get("quote_decimals")?,
        initialized_at: ChainPosition {
            block_number: integer(row, "initialized_block_number")?,
            block_hash: scalar(row, "initialized_block_hash")?,
            offset,
        },
        verification,
    };
    if row.get::<_, String>("key")? != serde_json::to_string(&pool.id)? {
        return Err(StoreError::Invalid("pool key mismatch"));
    }
    Ok(pool)
}
fn insert_bootstrap(
    connection: &Connection,
    id: Option<i64>,
    snapshot: &Bootstrap,
) -> Result<i64, StoreError> {
    snapshot.validate()?;
    budget(snapshot)?;
    connection.execute("INSERT INTO bootstraps(id,version,block_number,block_hash,pools_json,evidence_json) VALUES(?1,?2,?3,?4,?5,?6)", params![id,snapshot.version,sql_block_number(snapshot.position.block_number)?,snapshot.position.block_hash.to_string(),serde_json::to_string(&snapshot.pools)?,serde_json::to_string(&snapshot.evidence)?])?;
    Ok(connection.last_insert_rowid())
}
fn bootstrap_row(row: &rusqlite::Row<'_>) -> Result<Bootstrap, StoreError> {
    read_budget(row, &["block_hash", "pools_json", "evidence_json"])?;
    let snapshot = Bootstrap {
        version: row.get("version")?,
        position: ChainPosition {
            block_number: integer(row, "block_number")?,
            block_hash: scalar(row, "block_hash")?,
            offset: Offset::BlockEnd,
        },
        pools: json(row, "pools_json")?,
        evidence: json(row, "evidence_json")?,
    };
    snapshot.validate()?;
    budget(&snapshot)?;
    Ok(snapshot)
}
pub(super) fn insert_checkpoint(
    connection: &Connection,
    id: Option<i64>,
    checkpoint: &Checkpoint,
) -> Result<i64, StoreError> {
    checkpoint.validate()?;
    budget(checkpoint)?;
    connection.execute("INSERT INTO checkpoints(id,research_run_id,version,registry_version,config_hash,algorithm_version,last_raw_id,next_block,state_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![id,checkpoint.research_run_id,checkpoint.version,checkpoint.registry_version.to_string(),checkpoint.config_hash.to_string(),checkpoint.algorithm_version,checkpoint.processing_cursor.last_raw_id.to_string(),sql_block_number(checkpoint.processing_cursor.next_block)?,serde_json::to_string(&checkpoint.state)?])?;
    Ok(connection.last_insert_rowid())
}
fn checkpoint_row(row: &rusqlite::Row<'_>) -> Result<Checkpoint, StoreError> {
    read_budget(row, CHECKPOINT_TEXT_FIELDS)?;
    let checkpoint = Checkpoint {
        research_run_id: row.get("research_run_id")?,
        version: row.get("version")?,
        state: json(row, "state_json")?,
        registry_version: scalar(row, "registry_version")?,
        config_hash: scalar(row, "config_hash")?,
        algorithm_version: row.get("algorithm_version")?,
        processing_cursor: ProcessingCursor {
            last_raw_id: scalar(row, "last_raw_id")?,
            next_block: integer(row, "next_block")?,
        },
    };
    checkpoint.validate()?;
    budget(&checkpoint)?;
    Ok(checkpoint)
}
fn insert_run(connection: &Connection, run: &RunSpec) -> Result<(), StoreError> {
    run.validate()?;
    connection.execute("INSERT INTO research_runs(run_id,config_hash,algorithm_version,registry_version,quote_asset,amounts_json,min_depth,min_profit,cost_asset,cost_amount,cost_basis,cost_conversion_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![run.run_id,run.config_hash.to_string(),run.algorithm_version,run.registry_version.to_string(),run.quote_asset.to_string(),serde_json::to_string(&run.amounts)?,run.min_depth.to_string(),run.min_profit.to_string(),run.costs.asset.to_string(),run.costs.amount.map(|v|v.to_string()),run.costs.basis,serde_json::to_string(&run.costs.conversion)?])?;
    Ok(())
}
fn run_row(row: &rusqlite::Row<'_>) -> Result<RunSpec, StoreError> {
    let run = RunSpec {
        run_id: row.get("run_id")?,
        config_hash: scalar(row, "config_hash")?,
        algorithm_version: row.get("algorithm_version")?,
        registry_version: scalar(row, "registry_version")?,
        quote_asset: scalar(row, "quote_asset")?,
        amounts: json(row, "amounts_json")?,
        min_depth: scalar(row, "min_depth")?,
        min_profit: scalar(row, "min_profit")?,
        costs: arb_core::opportunity::CostEstimate {
            asset: scalar(row, "cost_asset")?,
            amount: row
                .get::<_, Option<String>>("cost_amount")?
                .map(|v| v.parse().map_err(|_| StoreError::Invalid("cost amount")))
                .transpose()?,
            conversion: json(row, "cost_conversion_json")?,
            basis: row.get("cost_basis")?,
        },
    };
    run.validate()?;
    Ok(run)
}
impl Store {
    pub fn register_pools(&mut self, pools: &[PoolDescriptor]) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for pool in pools {
            insert_pool(&tx, pool)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn read_pools(
        &self,
        after: Option<&PoolId>,
        limit: usize,
    ) -> Result<Vec<PoolDescriptor>, StoreError> {
        if !(1..=1000).contains(&limit) {
            return Err(StoreError::Invalid("page limit must be 1..1000"));
        }
        let key = after
            .map(serde_json::to_string)
            .transpose()?
            .unwrap_or_default();
        let mut stmt = self
            .connection
            .prepare("SELECT * FROM pool_registry WHERE key>?1 ORDER BY key LIMIT ?2")?;
        let mut rows = stmt.query(params![key, limit as i64])?;
        let mut result = vec![];
        while let Some(row) = rows.next()? {
            result.push(pool_row(row)?);
        }
        Ok(result)
    }
    pub fn save_bootstrap(&mut self, snapshot: &Bootstrap) -> Result<u64, StoreError> {
        Ok(insert_bootstrap(&self.connection, None, snapshot)? as u64)
    }
    pub fn load_bootstrap(&self, id: u64) -> Result<Bootstrap, StoreError> {
        let id = i64::try_from(id).map_err(|_| StoreError::Invalid("bootstrap id"))?;
        let mut stmt = self.connection.prepare("SELECT * FROM bootstraps WHERE id=?1 AND length(CAST(pools_json AS BLOB))+length(CAST(evidence_json AS BLOB))<=67108864")?;
        let mut rows = stmt.query([id])?;
        bootstrap_row(rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?)
    }
    pub fn save_checkpoint(&mut self, checkpoint: &Checkpoint) -> Result<u64, StoreError> {
        Ok(insert_checkpoint(&self.connection, None, checkpoint)? as u64)
    }
    pub fn load_checkpoint(&self, id: u64) -> Result<Checkpoint, StoreError> {
        let id = i64::try_from(id).map_err(|_| StoreError::Invalid("checkpoint id"))?;
        let mut stmt = self.connection.prepare(
            "SELECT * FROM checkpoints WHERE id=?1 AND length(CAST(state_json AS BLOB))<=67108864",
        )?;
        let mut rows = stmt.query([id])?;
        checkpoint_row(rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?)
    }
    pub fn register_run(&mut self, run: &RunSpec) -> Result<(), StoreError> {
        run.validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut stmt = tx.prepare("SELECT * FROM research_runs WHERE run_id=?1")?;
            let mut rows = stmt.query([&run.run_id])?;
            if let Some(row) = rows.next()? {
                if run_row(row)? != *run {
                    return Err(StoreError::Invalid(
                        "run id reused with different parameters",
                    ));
                }
            } else {
                insert_run(&tx, run)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn load_run(&self, id: &str) -> Result<RunSpec, StoreError> {
        let mut stmt = self
            .connection
            .prepare("SELECT * FROM research_runs WHERE run_id=?1")?;
        let mut rows = stmt.query([id])?;
        run_row(rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?)
    }
    pub fn read_checkpoints_before(
        &self,
        before: u64,
    ) -> Result<Vec<(u64, Checkpoint)>, StoreError> {
        let before = i64::try_from(before).map_err(|_| StoreError::Invalid("checkpoint cursor"))?;
        let mut stmt = self
            .connection
            .prepare("SELECT * FROM checkpoints WHERE id<?1 ORDER BY id DESC LIMIT 100")?;
        let mut rows = stmt.query([before])?;
        let mut result = vec![];
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            if bytes + read_budget(row, CHECKPOINT_TEXT_FIELDS)? > 67108864 {
                break;
            }
            let checkpoint = checkpoint_row(row)?;
            let size = budget(&checkpoint)?;
            if bytes + size > 67108864 {
                break;
            }
            bytes += size;
            result.push((integer(row, "id")?, checkpoint));
        }
        Ok(result)
    }
}
pub(super) fn migrate(tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    let mut stmt = tx.prepare("SELECT key,data FROM pool_registry_v014")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let pool: PoolDescriptor = serde_json::from_slice(&row.get::<_, Vec<u8>>("data")?)?;
        if row.get::<_, String>("key")? != serde_json::to_string(&pool.id)? {
            return Err(StoreError::Invalid("legacy pool key mismatch"));
        }
        insert_pool(tx, &pool)?;
    }
    let mut stmt =
        tx.prepare("SELECT id,data FROM bootstraps_v014 WHERE length(data)<=67108864")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let snapshot: Bootstrap = serde_json::from_slice(&row.get::<_, Vec<u8>>("data")?)?;
        insert_bootstrap(tx, Some(row.get("id")?), &snapshot)?;
    }
    let mut stmt =
        tx.prepare("SELECT id,data FROM checkpoints_v014 WHERE length(data)<=67108864")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let checkpoint: Checkpoint = serde_json::from_slice(&row.get::<_, Vec<u8>>("data")?)?;
        insert_checkpoint(tx, Some(row.get("id")?), &checkpoint)?;
    }
    for table in ["bootstraps_v014", "checkpoints_v014"] {
        let oversized: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE length(data)>67108864)"),
            [],
            |row| row.get(0),
        )?;
        if oversized {
            return Err(StoreError::Invalid("legacy catalog storage budget"));
        }
    }
    let mut stmt = tx.prepare("SELECT run_id,data FROM research_runs_v014")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let run: RunSpec = serde_json::from_slice(&row.get::<_, Vec<u8>>("data")?)?;
        if row.get::<_, String>("run_id")? != run.run_id {
            return Err(StoreError::Invalid("legacy research run id mismatch"));
        }
        insert_run(tx, &run)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256, U256};

    #[test]
    fn catalog_columns_roundtrip_and_are_authoritative() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("catalog.db")).unwrap();
        let position = ChainPosition {
            block_number: 8,
            block_hash: B256::repeat_byte(1),
            offset: Offset::BlockEnd,
        };
        let pool = PoolDescriptor {
            id: PoolId {
                chain_id: u64::MAX,
                locator: PoolLocator::Singleton {
                    manager: Address::repeat_byte(2),
                    pool_id: B256::repeat_byte(3),
                },
            },
            protocol: "test".into(),
            token: Address::repeat_byte(4),
            quote_asset: Address::repeat_byte(5),
            currency0: Address::repeat_byte(4),
            currency1: Address::repeat_byte(5),
            hook: Address::ZERO,
            lp_fee: 3000,
            tick_spacing: 60,
            hook_fee_bps: None,
            creator_tax_bps: Some(7),
            token_decimals: Some(18),
            quote_decimals: Some(6),
            initialized_at: ChainPosition {
                offset: Offset::Transaction {
                    index: u64::MAX,
                    log_index: Some(u64::MAX),
                },
                ..position.clone()
            },
            verification: PoolVerification::Unsupported("unsupported hook".into()),
        };
        store.register_pools(std::slice::from_ref(&pool)).unwrap();
        assert_eq!(store.read_pools(None, 10).unwrap(), vec![pool.clone()]);
        store
            .connection
            .execute("UPDATE pool_registry SET protocol='changed'", [])
            .unwrap();
        assert_eq!(store.read_pools(None, 10).unwrap()[0].protocol, "changed");
        let snapshot = Bootstrap {
            version: 1,
            position,
            pools: vec![arb_core::protocol::PoolState {
                descriptor: pool.clone(),
                sqrt_price_x96: U256::from(1),
                tick: 0,
                liquidity: 0,
                protocol_fee: 0,
                tick_bitmap: Default::default(),
            }],
            evidence: vec![vec![0, 255]],
        };
        let id = store.save_bootstrap(&snapshot).unwrap();
        assert_eq!(store.load_bootstrap(id).unwrap(), snapshot);
        let checkpoint = Checkpoint {
            research_run_id: Some("run".into()),
            version: 1,
            state: arb_core::state::State::from_bootstrap(snapshot.clone()).unwrap(),
            registry_version: u64::MAX,
            config_hash: B256::repeat_byte(9),
            algorithm_version: "v1".into(),
            processing_cursor: ProcessingCursor {
                last_raw_id: u64::MAX,
                next_block: 9,
            },
        };
        let id = store.save_checkpoint(&checkpoint).unwrap();
        assert_eq!(store.load_checkpoint(id).unwrap(), checkpoint);
        assert_eq!(
            store.read_checkpoints_before(id + 1).unwrap(),
            vec![(id, checkpoint.clone())]
        );
        let run = RunSpec {
            run_id: "run".into(),
            config_hash: B256::repeat_byte(9),
            algorithm_version: "v1".into(),
            registry_version: u64::MAX,
            quote_asset: Address::repeat_byte(5),
            amounts: vec![U256::MAX],
            min_depth: U256::MAX,
            min_profit: U256::from(2),
            costs: arb_core::opportunity::CostEstimate {
                asset: Address::ZERO,
                amount: Some(U256::MAX),
                conversion: None,
                basis: "estimate".into(),
            },
        };
        store.register_run(&run).unwrap();
        store.register_run(&run).unwrap();
        assert_eq!(store.load_run("run").unwrap(), run);
        let changed = RunSpec {
            min_profit: U256::from(3),
            ..run.clone()
        };
        assert!(store.register_run(&changed).is_err());

        let legacy_path = dir.path().join("legacy.db");
        let legacy = Connection::open(&legacy_path).unwrap();
        legacy
            .execute_batch(include_str!("../../schema/schema_v014.sql"))
            .unwrap();
        legacy
            .execute(
                "INSERT INTO pool_registry(key,data) VALUES(?1,?2)",
                params![
                    serde_json::to_string(&pool.id).unwrap(),
                    serde_json::to_vec(&pool).unwrap()
                ],
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO bootstraps(id,data) VALUES(41,?1)",
                params![serde_json::to_vec(&snapshot).unwrap()],
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO checkpoints(id,data) VALUES(73,?1)",
                params![serde_json::to_vec(&checkpoint).unwrap()],
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO research_runs(run_id,data) VALUES(?1,?2)",
                params![run.run_id, serde_json::to_vec(&run).unwrap()],
            )
            .unwrap();
        legacy
            .execute(
                "INSERT INTO runtime_checkpoints(run_id,checkpoint_id) VALUES('run',73)",
                [],
            )
            .unwrap();
        drop(legacy);
        let migrated = Store::open(&legacy_path).unwrap();
        assert_eq!(migrated.read_pools(None, 10).unwrap(), vec![pool]);
        assert_eq!(migrated.load_bootstrap(41).unwrap(), snapshot);
        assert_eq!(migrated.load_checkpoint(73).unwrap(), checkpoint);
        assert_eq!(migrated.load_run("run").unwrap(), run);
        assert_eq!(
            migrated.runtime_checkpoint("run").unwrap(),
            Some(checkpoint)
        );
        let reference: i64 = migrated
            .connection
            .query_row(
                "SELECT checkpoint_id FROM runtime_checkpoints WHERE run_id='run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reference, 73);
        let foreign_key_errors: i64 = migrated
            .connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(foreign_key_errors, 0);
        migrated
            .connection
            .pragma_update(None, "ignore_check_constraints", "ON")
            .unwrap();
        migrated
            .connection
            .execute(
                "UPDATE pool_registry SET initialized_offset_kind='block_end'",
                [],
            )
            .unwrap();
        assert!(migrated.read_pools(None, 10).is_err());
        migrated.connection.execute("UPDATE pool_registry SET initialized_offset_kind='transaction',initialized_transaction_index=NULL", []).unwrap();
        assert!(migrated.read_pools(None, 10).is_err());
        migrated.connection.execute("UPDATE checkpoints SET algorithm_version=CAST(zeroblob(67108865) AS TEXT) WHERE id=73", []).unwrap();
        assert!(matches!(
            migrated.load_checkpoint(73),
            Err(StoreError::Invalid("catalog record exceeds storage budget"))
        ));
        assert!(matches!(
            migrated.read_checkpoints_before(74),
            Err(StoreError::Invalid("catalog record exceeds storage budget"))
        ));
    }
}
