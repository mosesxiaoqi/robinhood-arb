use arb_app::config::Config;
const VALID: &str = include_str!("../../../config/example.toml");
#[test]
fn reject_invalid_config() {
    assert!(Config::parse(VALID).is_ok());
    for (from, to) in [
        ("amounts = [\"1000000000000000\"]", "amounts = []"),
        ("queue_capacity = 8", "queue_capacity = 0"),
        ("chain_id = 4663", ""),
        ("https://rpc.invalid", "file:///secret"),
        ("1000000000000000", "1.5"),
        ("1000000000000000", "-1"),
        (
            "1000000000000000",
            "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        ),
        (
            "disk_reserve_bytes = 10485760",
            "disk_reserve_bytes = 999999999999",
        ),
    ] {
        assert!(Config::parse(&VALID.replace(from, to)).is_err(), "{from}");
    }
    assert!(Config::parse(&format!("{VALID}\nunknown_option = 1")).is_err());
}
#[test]
fn errors_do_not_expose_endpoint_credentials() {
    let bad = VALID.replace("https://rpc.invalid", "https://rpc.invalid/SECRET_TOKEN");
    let error = Config::parse(&format!("{bad}\ninvalid = true"))
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("SECRET_TOKEN"));
}
