use arb_adapters::{feed::FeedSource, store::Store};
use arb_core::types::ExecutionStatus;
fn sample(sequence: u64) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../tests/data/verified/feed-message.json"
    ))
    .unwrap();
    value["messages"].as_array_mut().unwrap().truncate(1);
    value["messages"][0]["sequenceNumber"] = sequence.into();
    serde_json::to_vec(&value).unwrap()
}
#[test]
fn feed_gap_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("feed.db")).unwrap();
    let feed = FeedSource::new(
        "wss://feed.mainnet.chain.robinhood.com",
        4663,
        1024 * 1024,
        1000,
        1,
    )
    .unwrap();
    let first = store
        .append_feed(&feed.decode(&sample(10), "test", 1).unwrap())
        .unwrap();
    assert!(first.gaps.is_empty());
    let next = store
        .append_feed(&feed.decode(&sample(12), "test", 2).unwrap())
        .unwrap();
    assert_eq!(next.gaps, vec![(11, 11)]);
    let repeated = store
        .append_feed(&feed.decode(&sample(12), "reconnect", 3).unwrap())
        .unwrap();
    assert_eq!(repeated.new_messages, 0);
    assert_eq!(repeated.duplicates, 1);
    assert_eq!(store.feed_next_sequence(4663).unwrap(), Some(13));
    assert_eq!(
        store.read_raw_after(0, 10).unwrap()[0]
            .record
            .execution_status,
        ExecutionStatus::Unknown
    );
    assert!(
        feed.decode(&vec![b' '; 1024 * 1024 + 1], "test", 4)
            .is_err()
    );
    let mut conflict: serde_json::Value = serde_json::from_slice(&sample(12)).unwrap();
    conflict["messages"][0]["message"]["delayedMessagesRead"] = 999.into();
    assert!(
        store
            .append_feed(
                &feed
                    .decode(&serde_json::to_vec(&conflict).unwrap(), "test", 4)
                    .unwrap()
            )
            .is_err()
    );
    assert_eq!(store.raw_count().unwrap(), 3);
}

#[tokio::test]
#[ignore = "explicit public Feed acceptance; writes a local evidence sample"]
async fn live_feed_acceptance() {
    let feed = FeedSource::new(
        "wss://feed.mainnet.chain.robinhood.com",
        4663,
        4 * 1024 * 1024,
        10000,
        1,
    )
    .unwrap();
    let mut session = feed.connect(None).await.unwrap();
    let packet = session.receive("t25-live").await.unwrap();
    assert!(!packet.messages.is_empty());
    assert!(packet.raw.position.is_none());
    std::fs::create_dir_all("../../data").unwrap();
    std::fs::write("../../data/t25-live-feed.json", packet.raw.payload).unwrap();
    println!("observed sequence {}", packet.messages[0].0);
}
