use super::*;
use alloy_primitives::{Address, B256, U256};
use arb_core::{
    simulation::SimulationRecord,
    types::{ChainPosition, Offset},
    wallet::WalletFacts,
};
use rusqlite::{Row, Transaction};
use serde::de::DeserializeOwned;

fn parse<T: std::str::FromStr>(row: &Row<'_>, name: &str) -> Result<T, StoreError> {
    row.get::<_, String>(name)?
        .parse()
        .map_err(|_| StoreError::Invalid("invalid simulation/wallet scalar"))
}
fn optional_parse<T: std::str::FromStr>(
    row: &Row<'_>,
    name: &str,
) -> Result<Option<T>, StoreError> {
    row.get::<_, Option<String>>(name)?
        .map(|s| {
            s.parse()
                .map_err(|_| StoreError::Invalid("invalid simulation/wallet scalar"))
        })
        .transpose()
}
fn integer<T: TryFrom<i64>>(row: &Row<'_>, name: &str) -> Result<T, StoreError> {
    row.get::<_, i64>(name)?
        .try_into()
        .map_err(|_| StoreError::Invalid("simulation/wallet integer range"))
}
fn optional_integer<T: TryFrom<i64>>(row: &Row<'_>, name: &str) -> Result<Option<T>, StoreError> {
    row.get::<_, Option<i64>>(name)?
        .map(|v| {
            v.try_into()
                .map_err(|_| StoreError::Invalid("simulation/wallet integer range"))
        })
        .transpose()
}
fn json<T: DeserializeOwned>(row: &Row<'_>, name: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_str(&row.get::<_, String>(name)?)?)
}
fn enum_value<T: DeserializeOwned>(row: &Row<'_>, name: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(
        row.get(name)?,
    ))?)
}
fn offset_columns(
    position: &ChainPosition,
) -> Result<(&'static str, Option<i64>, Option<i64>), StoreError> {
    Ok(match position.offset {
        Offset::BlockEnd => ("BlockEnd", None, None),
        Offset::Transaction { index, log_index } => (
            "Transaction",
            Some(sql_block_number(index)?),
            log_index.map(sql_block_number).transpose()?,
        ),
    })
}
fn position_from_row(row: &Row<'_>) -> Result<ChainPosition, StoreError> {
    let index: Option<u64> = optional_integer(row, "transaction_index")?;
    let log_index: Option<u64> = optional_integer(row, "log_index")?;
    let offset = match row.get::<_, String>("offset_kind")?.as_str() {
        "BlockEnd" if index.is_none() && log_index.is_none() => Offset::BlockEnd,
        "Transaction" => Offset::Transaction {
            index: index.ok_or(StoreError::Invalid("missing transaction index"))?,
            log_index,
        },
        _ => return Err(StoreError::Invalid("invalid position offset")),
    };
    Ok(ChainPosition {
        block_number: integer(row, "block_number")?,
        block_hash: parse(row, "block_hash")?,
        offset,
    })
}

pub(super) fn simulation_from_row(row: &Row<'_>) -> Result<SimulationRecord, StoreError> {
    check_row_budget(row, 67108864)?;
    Ok(SimulationRecord {
        id: parse(row, "job_id")?,
        queue_id: row.get("queue_id")?,
        candidate_id: parse(row, "candidate_id")?,
        run_id: row.get("run_id")?,
        view_id: parse(row, "view_id")?,
        request: json(row, "request_json")?,
        phase: enum_value(row, "phase")?,
        outcome: enum_value(row, "outcome")?,
        submitted_at_ms: integer(row, "submitted_at_ms")?,
        queued_at_ms: optional_integer(row, "queued_at_ms")?,
        started_at_ms: optional_integer(row, "started_at_ms")?,
        ended_at_ms: optional_integer(row, "ended_at_ms")?,
        queue_wait_ns: optional_integer(row, "queue_wait_ns")?,
        elapsed_ns: optional_integer(row, "elapsed_ns")?,
        result: row
            .get::<_, Option<String>>("result_json")?
            .map(|v| serde_json::from_str(&v))
            .transpose()?,
        error: row.get("error")?,
        error_evidence: json(row, "error_evidence_json")?,
        canonical: row.get("canonical")?,
        validates_original_candidate: row.get("validates_original")?,
        expected_position: position_from_row(row)?,
    })
}

fn write_simulation(
    connection: &Connection,
    id: Option<i64>,
    r: &SimulationRecord,
    conflict: &str,
) -> Result<usize, StoreError> {
    if serde_json::to_vec(r)?.len() > 67108864 {
        return Err(StoreError::Invalid("simulation/wallet record budget"));
    }
    let (offset, index, log_index) = offset_columns(&r.expected_position)?;
    let sql = format!(
        "INSERT INTO simulations(id,job_id,queue_id,candidate_id,run_id,view_id,block_number,block_hash,offset_kind,transaction_index,log_index,request_json,phase,outcome,submitted_at_ms,queued_at_ms,started_at_ms,ended_at_ms,queue_wait_ns,elapsed_ns,result_json,error,error_evidence_json,canonical,validates_original) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25) {conflict}"
    );
    Ok(connection.execute(
        &sql,
        params![
            id,
            r.id.to_string(),
            &r.queue_id,
            r.candidate_id.to_string(),
            &r.run_id,
            r.view_id.to_string(),
            sql_block_number(r.expected_position.block_number)?,
            r.expected_position.block_hash.to_string(),
            offset,
            index,
            log_index,
            serde_json::to_string(&r.request)?,
            format!("{:?}", r.phase),
            format!("{:?}", r.outcome),
            sql_block_number(r.submitted_at_ms)?,
            r.queued_at_ms.map(sql_block_number).transpose()?,
            r.started_at_ms.map(sql_block_number).transpose()?,
            r.ended_at_ms.map(sql_block_number).transpose()?,
            r.queue_wait_ns.map(sql_block_number).transpose()?,
            r.elapsed_ns.map(sql_block_number).transpose()?,
            r.result.as_ref().map(serde_json::to_string).transpose()?,
            &r.error,
            serde_json::to_string(&r.error_evidence)?,
            r.canonical,
            r.validates_original_candidate
        ],
    )?)
}
const UPDATE_SIMULATION: &str = "ON CONFLICT(job_id) DO UPDATE SET job_id=excluded.job_id,queue_id=excluded.queue_id,candidate_id=excluded.candidate_id,run_id=excluded.run_id,view_id=excluded.view_id,block_number=excluded.block_number,block_hash=excluded.block_hash,offset_kind=excluded.offset_kind,transaction_index=excluded.transaction_index,log_index=excluded.log_index,request_json=excluded.request_json,phase=excluded.phase,outcome=excluded.outcome,submitted_at_ms=excluded.submitted_at_ms,queued_at_ms=excluded.queued_at_ms,started_at_ms=excluded.started_at_ms,ended_at_ms=excluded.ended_at_ms,queue_wait_ns=excluded.queue_wait_ns,elapsed_ns=excluded.elapsed_ns,result_json=excluded.result_json,error=excluded.error,error_evidence_json=excluded.error_evidence_json,canonical=excluded.canonical,validates_original=excluded.validates_original";

pub(super) fn wallet_from_row(row: &Row<'_>) -> Result<WalletFacts, StoreError> {
    check_row_budget(row, 67108864)?;
    Ok(WalletFacts {
        chain_id: parse::<u64>(row, "chain_id")?,
        transaction_hash: parse(row, "transaction_hash")?,
        wallet: parse(row, "wallet")?,
        sender: parse(row, "sender")?,
        recipient: optional_parse::<Address>(row, "recipient")?,
        wallet_is_contract: row.get("wallet_is_contract")?,
        execution_status: enum_value(row, "execution_status")?,
        changes: json(row, "changes_json")?,
        transfers: json(row, "transfers_json")?,
        swap_count: integer(row, "swap_count")?,
        liquidity_count: integer(row, "liquidity_count")?,
        gas_cost: optional_parse::<U256>(row, "gas_cost")?,
        balance_scope: enum_value(row, "balance_scope")?,
        receipt_complete: row.get("receipt_complete")?,
        transaction_balances_complete: row.get("transaction_balances_complete")?,
        valuation: row
            .get::<_, Option<String>>("valuation_json")?
            .map(|v| serde_json::from_str(&v))
            .transpose()?,
        evidence: json(row, "evidence_json")?,
        position: position_from_row(row)?,
    })
}

fn write_wallet(
    connection: &Connection,
    id: Option<i64>,
    run: &str,
    r: &WalletFacts,
    conflict: &str,
) -> Result<usize, StoreError> {
    if serde_json::to_vec(r)?.len() > 67108864 {
        return Err(StoreError::Invalid("simulation/wallet record budget"));
    }
    let (offset, index, log_index) = offset_columns(&r.position)?;
    let sql = format!(
        "INSERT INTO wallet_facts(id,run_id,chain_id,block_number,block_hash,offset_kind,transaction_index,log_index,transaction_hash,wallet,sender,recipient,wallet_is_contract,execution_status,changes_json,transfers_json,swap_count,liquidity_count,gas_cost,balance_scope,receipt_complete,transaction_balances_complete,valuation_json,evidence_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24) {conflict}"
    );
    Ok(connection.execute(
        &sql,
        params![
            id,
            run,
            r.chain_id.to_string(),
            sql_block_number(r.position.block_number)?,
            r.position.block_hash.to_string(),
            offset,
            index,
            log_index,
            r.transaction_hash.to_string(),
            r.wallet.to_string(),
            r.sender.to_string(),
            r.recipient.map(|a| a.to_string()),
            r.wallet_is_contract,
            format!("{:?}", r.execution_status),
            serde_json::to_string(&r.changes)?,
            serde_json::to_string(&r.transfers)?,
            i64::try_from(r.swap_count).map_err(|_| StoreError::Invalid("wallet count range"))?,
            i64::try_from(r.liquidity_count)
                .map_err(|_| StoreError::Invalid("wallet count range"))?,
            r.gas_cost.map(|v| v.to_string()),
            format!("{:?}", r.balance_scope),
            r.receipt_complete,
            r.transaction_balances_complete,
            r.valuation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            serde_json::to_string(&r.evidence)?
        ],
    )?)
}

pub(super) fn load_simulation(
    connection: &Connection,
    id: B256,
) -> Result<SimulationRecord, StoreError> {
    let mut statement = connection.prepare("SELECT * FROM simulations WHERE job_id=?1")?;
    let mut rows = statement.query([id.to_string()])?;
    simulation_from_row(rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?)
}

impl Store {
    pub fn insert_simulation(&mut self, record: &SimulationRecord) -> Result<bool, StoreError> {
        let mut record = record.clone();
        record.canonical = false;
        record.validates_original_candidate = false;
        Ok(write_simulation(
            &self.connection,
            None,
            &record,
            "ON CONFLICT(job_id) DO NOTHING",
        )? == 1)
    }
    pub fn save_simulation(
        &mut self,
        record: &mut arb_core::simulation::SimulationRecord,
    ) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (row_id, phase): (i64, String) = tx.query_row(
            "SELECT id,phase FROM simulations WHERE job_id=?1",
            [record.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if phase == "Interrupted" {
            return Ok(());
        }
        let block = super::load_derived(&tx, &record.run_id, record.expected_position.block_hash)?;
        record.canonical = false;
        record.validates_original_candidate = false;
        if let Some(block) = block {
            let canonical = block.canonical;
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM recovery_jobs WHERE run_id=?1 AND status='pending')",
                [&record.run_id],
                |r| r.get(0),
            )?;
            if let Some(mut candidate) = block
                .candidates
                .into_iter()
                .find(|c| c.id == record.candidate_id && c.view_id == record.view_id)
            {
                candidate.canonical = canonical && !pending;
                record.canonical = candidate.canonical;
                record.validates_original_candidate =
                    record.result.as_ref().is_some_and(|result| {
                        arb_core::simulation::validates_candidate(&candidate, result)
                    });
            }
        }
        write_simulation(&tx, Some(row_id), record, UPDATE_SIMULATION)?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_simulation(&self, id: B256) -> Result<SimulationRecord, StoreError> {
        load_simulation(&self.connection, id)
    }
    pub fn interrupt_simulations(&mut self, queue: &str, ended: u64) -> Result<(), StoreError> {
        self.connection.execute("UPDATE simulations SET phase='Interrupted',outcome='Unknown',ended_at_ms=?1,error='queue stopped before completion',validates_original=0 WHERE queue_id=?2 AND phase IN ('Queued','Running')",params![sql_block_number(ended)?,queue])?;
        Ok(())
    }
    pub fn save_wallet_facts(&mut self, run: &str, facts: &WalletFacts) -> Result<(), StoreError> {
        self.load_run(run)?;
        if facts.evidence.is_empty() || facts.chain_id == 0 || facts.transaction_hash == B256::ZERO
        {
            return Err(StoreError::Invalid("wallet evidence budget/scope"));
        }
        write_wallet(
            &self.connection,
            None,
            run,
            facts,
            "ON CONFLICT(run_id,transaction_hash,wallet,block_hash) DO NOTHING",
        )?;
        Ok(())
    }
}

pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let mut statement = tx.prepare("SELECT * FROM simulations_v014")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let bytes = row
            .get_ref("data")?
            .as_blob()
            .map_err(|_| StoreError::Invalid("legacy simulation data"))?;
        if bytes.len() > 67108864 {
            return Err(StoreError::Invalid("legacy simulation budget"));
        }
        let mut record: SimulationRecord = serde_json::from_slice(bytes)?;
        if record.id != parse::<B256>(row, "job_id")?
            || record.queue_id != row.get::<_, String>("queue_id")?
            || record.run_id != row.get::<_, String>("run_id")?
            || record.expected_position.block_hash != parse::<B256>(row, "block_hash")?
        {
            return Err(StoreError::Invalid(
                "simulation migration identity mismatch",
            ));
        }
        record.phase = enum_value(row, "phase")?;
        record.canonical = row.get("canonical")?;
        record.validates_original_candidate = row.get("validates_original")?;
        write_simulation(tx, Some(row.get("id")?), &record, "")?;
    }
    let mut statement = tx.prepare("SELECT * FROM wallet_facts_v014")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let bytes = row
            .get_ref("data")?
            .as_blob()
            .map_err(|_| StoreError::Invalid("legacy wallet data"))?;
        if bytes.len() > 67108864 {
            return Err(StoreError::Invalid("legacy wallet budget"));
        }
        let record: WalletFacts = serde_json::from_slice(bytes)?;
        if record.transaction_hash != parse::<B256>(row, "transaction_hash")?
            || record.wallet != parse::<Address>(row, "wallet")?
            || record.position.block_hash != parse::<B256>(row, "block_hash")?
        {
            return Err(StoreError::Invalid("wallet migration identity mismatch"));
        }
        write_wallet(
            tx,
            Some(row.get("id")?),
            &row.get::<_, String>("run_id")?,
            &record,
            "",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_core::{route::Route, simulation::*, types::ExecutionStatus, wallet::BalanceScope};

    #[test]
    fn scalar_columns_round_trip_and_interrupts_reject_late_results() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(&directory.path().join("simulation.db")).unwrap();
        let position = ChainPosition {
            block_number: 123,
            block_hash: B256::repeat_byte(3),
            offset: Offset::Transaction {
                index: 2,
                log_index: Some(4),
            },
        };
        let pool = SimulationPool {
            currency0: Address::ZERO,
            currency1: Address::repeat_byte(1),
            fee: 1,
            tick_spacing: 2,
            hook: Address::ZERO,
        };
        let mut record = SimulationRecord {
            id: B256::repeat_byte(1),
            queue_id: "queue".into(),
            candidate_id: B256::repeat_byte(2),
            run_id: "run".into(),
            view_id: B256::repeat_byte(4),
            expected_position: position.clone(),
            request: SimulationRequest {
                position: position.clone(),
                route: Route { legs: vec![] },
                pools: [pool.clone(), pool],
                amount: U256::MAX,
                funding_balance: U256::MAX,
                fail_second: false,
            },
            phase: SimulationPhase::Running,
            outcome: SimulationOutcome::Unknown,
            submitted_at_ms: 1,
            queued_at_ms: Some(2),
            started_at_ms: Some(3),
            ended_at_ms: None,
            queue_wait_ns: Some(4),
            elapsed_ns: Some(5),
            result: None,
            error: None,
            error_evidence: vec![vec![1, 2]],
            canonical: false,
            validates_original_candidate: false,
        };
        assert!(store.insert_simulation(&record).unwrap());
        assert!(!store.insert_simulation(&record).unwrap());
        assert_eq!(store.load_simulation(record.id).unwrap(), record);
        let sequence: i64 = store
            .connection
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='simulations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        record.elapsed_ns = Some(9);
        store.save_simulation(&mut record).unwrap();
        assert_eq!(store.load_simulation(record.id).unwrap(), record);
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='simulations'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            sequence
        );
        store
            .connection
            .execute(
                "UPDATE simulations SET canonical=1,validates_original=1,error='scalar value'",
                [],
            )
            .unwrap();
        let loaded = store.load_simulation(record.id).unwrap();
        assert!(loaded.canonical && loaded.validates_original_candidate);
        assert_eq!(loaded.error.as_deref(), Some("scalar value"));
        store.interrupt_simulations("queue", 100).unwrap();
        record.phase = SimulationPhase::Finished;
        record.outcome = SimulationOutcome::Succeeded;
        store.save_simulation(&mut record).unwrap();
        let stopped = store.load_simulation(record.id).unwrap();
        assert_eq!(stopped.phase, SimulationPhase::Interrupted);
        assert_eq!(stopped.outcome, SimulationOutcome::Unknown);
        assert_eq!(stopped.ended_at_ms, Some(100));
        assert!(!stopped.validates_original_candidate);
        record.id = B256::repeat_byte(9);
        record.submitted_at_ms = u64::MAX;
        assert!(store.insert_simulation(&record).is_err());

        let facts = WalletFacts {
            chain_id: u64::MAX,
            position,
            transaction_hash: B256::repeat_byte(5),
            wallet: Address::repeat_byte(6),
            sender: Address::repeat_byte(7),
            recipient: Some(Address::repeat_byte(8)),
            wallet_is_contract: true,
            execution_status: ExecutionStatus::Succeeded,
            changes: vec![],
            transfers: vec![],
            swap_count: 2,
            liquidity_count: 3,
            gas_cost: Some(U256::MAX),
            balance_scope: BalanceScope::Transaction,
            receipt_complete: true,
            transaction_balances_complete: false,
            valuation: None,
            evidence: vec![vec![1, 2, 3]],
        };
        write_wallet(&store.connection, Some(17), "run", &facts, "").unwrap();
        let mut query = store
            .connection
            .prepare("SELECT * FROM wallet_facts WHERE id=17")
            .unwrap();
        let mut rows = query.query([]).unwrap();
        assert_eq!(
            wallet_from_row(rows.next().unwrap().unwrap()).unwrap(),
            facts
        );

        // Recovery updates these authoritative columns independently of the old JSON.
        let old_path = directory.path().join("v14.db");
        let old = Connection::open(&old_path).unwrap();
        old.execute_batch(include_str!("../../schema/schema_v014.sql"))
            .unwrap();
        record.submitted_at_ms = 1;
        record.phase = SimulationPhase::Running;
        old.execute("INSERT INTO simulations(id,job_id,queue_id,run_id,block_hash,phase,canonical,validates_original,data) VALUES(41,?1,?2,?3,?4,'Finished',1,1,?5)", params![record.id.to_string(),record.queue_id,record.run_id,record.expected_position.block_hash.to_string(),serde_json::to_vec(&record).unwrap()]).unwrap();
        old.execute("INSERT INTO wallet_facts(id,run_id,transaction_hash,wallet,block_hash,data) VALUES(57,'run',?1,?2,?3,?4)", params![facts.transaction_hash.to_string(),facts.wallet.to_string(),facts.position.block_hash.to_string(),serde_json::to_vec(&facts).unwrap()]).unwrap();
        drop(old);
        let migrated = Store::open(&old_path).unwrap();
        record.phase = SimulationPhase::Finished;
        record.canonical = true;
        record.validates_original_candidate = true;
        assert_eq!(migrated.load_simulation(record.id).unwrap(), record);
        assert_eq!(
            migrated
                .connection
                .query_row("SELECT id FROM simulations", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            41
        );
        let mut query = migrated
            .connection
            .prepare("SELECT * FROM wallet_facts WHERE id=57")
            .unwrap();
        let mut rows = query.query([]).unwrap();
        assert_eq!(
            wallet_from_row(rows.next().unwrap().unwrap()).unwrap(),
            facts
        );
    }
}
