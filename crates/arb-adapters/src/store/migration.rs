use super::*;

/// Convert v14 JSON with the same typed mappings used by normal writes.
/// Store::open owns the transaction: a malformed record rolls back DDL and data together.
pub(super) fn migrate_v15(tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    raw::migrate(tx)?;
    catalog::migrate(tx)?;
    research::migrate(tx)?;
    simulation::migrate(tx)?;
    tx.execute_batch("INSERT INTO runtime_checkpoints SELECT * FROM runtime_checkpoints_v014; DROP TABLE runtime_checkpoints_v014;")?;
    for table in [
        "raw_records",
        "bootstraps",
        "checkpoints",
        "derived_blocks",
        "simulations",
    ] {
        // Preserve deleted high IDs too, so processing cursors never see a reused ID.
        tx.execute(
            "INSERT INTO sqlite_sequence(name,seq) SELECT ?1,seq FROM sqlite_sequence WHERE name=?2 AND NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name=?1)",
            params![table, format!("{table}_v014")],
        )?;
        tx.execute(
            "UPDATE sqlite_sequence SET seq=max(seq,coalesce((SELECT seq FROM sqlite_sequence WHERE name=?2),0)) WHERE name=?1",
            params![table, format!("{table}_v014")],
        )?;
    }
    for table in [
        "raw_records",
        "pool_registry",
        "bootstraps",
        "checkpoints",
        "research_runs",
        "derived_blocks",
        "recovery_jobs",
        "simulations",
        "wallet_facts",
        "runtime_status",
    ] {
        tx.execute_batch(&format!("DROP TABLE {table}_v014;"))?;
    }
    if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(StoreError::Invalid("migrated foreign key mismatch"));
    }
    tx.pragma_update(None, "user_version", 15)?;
    Ok(())
}
