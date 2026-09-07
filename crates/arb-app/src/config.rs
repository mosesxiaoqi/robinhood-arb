use alloy_primitives::{Address, U256};
use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rpc_url: String,
    pub chain_id: u64,
    pub quote_asset: Address,
    pub amounts: Vec<String>,
    pub min_depth: String,
    pub min_profit: String,
    pub confirmations: u64,
    pub requests_per_second: u32,
    pub max_concurrency: usize,
    pub retry_limit: u32,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
    pub queue_capacity: usize,
    pub database: PathBuf,
    pub window_seconds: u64,
    pub disk_budget_bytes: u64,
    pub disk_reserve_bytes: u64,
}

#[derive(Debug, Error)]
#[error("invalid configuration: {0}")]
pub struct ConfigError(pub &'static str);

impl Config {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        // Do not display toml errors: they include source lines, possibly RPC credentials.
        let config: Self = toml::from_str(text)
            .map_err(|_| ConfigError("TOML syntax, unknown/missing field or invalid field type"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let url = Url::parse(&self.rpc_url).map_err(|_| ConfigError("rpc_url"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.fragment().is_some()
        {
            return Err(ConfigError(
                "rpc_url must be HTTP(S), with a host and no fragment",
            ));
        }
        if self.chain_id == 0 {
            return Err(ConfigError("chain_id must be positive"));
        }
        if self.amounts.is_empty() {
            return Err(ConfigError("amounts must not be empty"));
        }
        for value in &self.amounts {
            if parse_amount(value)? == U256::ZERO {
                return Err(ConfigError("amounts must be positive"));
            }
        }
        parse_amount(&self.min_depth)?;
        parse_amount(&self.min_profit)?;
        for (name, value) in [
            ("requests_per_second", u64::from(self.requests_per_second)),
            ("max_concurrency", self.max_concurrency as u64),
            ("timeout_ms", self.timeout_ms),
            ("max_response_bytes", self.max_response_bytes as u64),
            ("queue_capacity", self.queue_capacity as u64),
            ("window_seconds", self.window_seconds),
        ] {
            if value == 0 {
                return Err(ConfigError(name));
            }
        }
        if self.max_concurrency > 1024
            || self.queue_capacity > 65536
            || self.max_response_bytes > 64 * 1024 * 1024
            || self.retry_limit > 10
        {
            return Err(ConfigError(
                "concurrency, queue, response or retry safety limit exceeded",
            ));
        }
        if self.timeout_ms > 300_000 || self.window_seconds > 31_536_000 {
            return Err(ConfigError("timeout/window exceeds supported range"));
        }
        if self.database.as_os_str().is_empty() {
            return Err(ConfigError("database path"));
        }
        if self.disk_reserve_bytes == 0 || self.disk_budget_bytes <= self.disk_reserve_bytes {
            return Err(ConfigError(
                "disk_budget_bytes must exceed positive disk_reserve_bytes",
            ));
        }
        Ok(())
    }
}

pub fn parse_amount(value: &str) -> Result<U256, ConfigError> {
    if value.is_empty() || !value.bytes().all(|v| v.is_ascii_digit()) {
        return Err(ConfigError("amount must be a decimal integer string"));
    }
    U256::from_str_radix(value, 10).map_err(|_| ConfigError("amount exceeds U256"))
}
