//! In-memory config cache. Loaded once at startup, mutated in-place,
//! flushed to disk on writes. Single-threaded — no locking needed.

use frgb_core::config::{load_config, save_config};
use frgb_model::config::Config;

pub struct ConfigCache {
    inner: Config,
}

impl ConfigCache {
    pub fn load() -> Self {
        let mut inner = load_config().unwrap_or_else(|e| {
            tracing::warn!("Failed to load config: {e}, using defaults");
            Config::default()
        });
        let warnings = inner.validate();
        for w in &warnings {
            tracing::warn!("Config validation: {w}");
        }
        Self { inner }
    }

    pub fn config(&self) -> &Config {
        &self.inner
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.inner
    }

    pub fn flush(&self) {
        if let Err(e) = save_config(&self.inner) {
            tracing::error!("Failed to flush config: {e}");
        }
    }

    pub fn reload(&mut self) {
        match load_config() {
            Ok(mut config) => {
                let warnings = config.validate();
                for w in &warnings {
                    tracing::warn!("Config validation: {w}");
                }
                self.inner = config;
            }
            Err(e) => tracing::error!("Failed to reload config: {e}"),
        }
    }
}

impl From<Config> for ConfigCache {
    fn from(inner: Config) -> Self {
        Self { inner }
    }
}
