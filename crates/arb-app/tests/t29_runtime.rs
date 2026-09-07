#[path = "support/replay.rs"]
mod support;
use alloy_primitives::B256;
use arb_adapters::store::Store;
use arb_app::{config::Config, runtime::*};
#[test]
fn stop_without_advancing_unwritten_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("runtime.db");
    let mut config = Config::parse(include_str!("../../../config/example.toml")).unwrap();
    config.database = database.clone();
    let mut engine = RuntimeEngine::open(&config).unwrap();
    assert!(RuntimeEngine::open(&config).is_err());
    let mut probe = engine.budget.clone();
    probe.path = dir.path().join("size.db");
    for (suffix, size) in [("", 10), ("-wal", 20), ("-shm", 30)] {
        std::fs::write(format!("{}{suffix}", probe.path.display()), vec![0; size]).unwrap();
    }
    assert_eq!(probe.used().unwrap(), 60);
    let first = support::block(2, B256::repeat_byte(1));
    assert_eq!(
        engine.commit(&first, false).unwrap(),
        RunStatus::AwaitingPools
    );
    assert_eq!(
        Store::open(&database)
            .unwrap()
            .cursor(4663, "rpc")
            .unwrap()
            .unwrap()
            .next_block,
        3
    );
    let second = support::block(3, B256::repeat_byte(2));
    assert_eq!(engine.commit(&second, true).unwrap(), RunStatus::Stopped);
    engine.budget.limit = 1;
    assert_eq!(
        engine.commit(&second, false).unwrap(),
        RunStatus::DiskPaused
    );
    assert_eq!(Store::open(&database).unwrap().raw_count().unwrap(), 3);
    assert_eq!(
        Store::open(&database)
            .unwrap()
            .cursor(4663, "rpc")
            .unwrap()
            .unwrap()
            .next_block,
        3
    );
    drop(engine);
    let mut engine = RuntimeEngine::open(&config).unwrap();
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_raw BEFORE INSERT ON raw_records BEGIN SELECT RAISE(ABORT,'write failed'); END;").unwrap();
    assert!(engine.commit(&second, false).is_err());
    assert_eq!(engine.next_collect().unwrap(), Some(3));
    assert_eq!(Store::open(&database).unwrap().raw_count().unwrap(), 3);
}

#[tokio::test]
async fn signal_cancels_slow_network_without_advancing_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = Config::parse(include_str!("../../../config/example.toml")).unwrap();
    config.database = dir.path().join("cancel.db");
    config.rpc_url = format!("http://{}", listener.local_addr().unwrap());
    config.timeout_ms = 300000;
    let (sender, receiver) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        sender.send(true).unwrap();
        std::future::pending::<()>().await;
    });
    let started = std::time::Instant::now();
    let database = config.database.clone();
    let result = run(config, receiver).await.unwrap();
    server.abort();
    assert_eq!(result.status, RunStatus::Stopped);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let store = Store::open(&database).unwrap();
    assert_eq!(store.cursor(4663, "rpc").unwrap(), None);
    assert_eq!(store.raw_count().unwrap(), 0);
}
