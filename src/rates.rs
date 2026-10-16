//! Currency exchange rates for fend, cached on disk and refreshed in the background.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const RATES_URL: &str = "https://open.er-api.com/v6/latest/USD";
const MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    fetched_at: u64,
    /// Units of each currency per 1 USD.
    rates: HashMap<String, f64>,
}

#[derive(Deserialize)]
struct ApiResponse {
    rates: HashMap<String, f64>,
}

/// Shared, cheaply clonable rate table. `version` bumps whenever new rates arrive
/// so the UI knows to re-evaluate.
#[derive(Clone, Default)]
pub struct Rates {
    table: Arc<RwLock<HashMap<String, f64>>>,
    version: Arc<AtomicU64>,
}

impl Rates {
    /// Load cached rates, then refresh them on a background thread if stale.
    pub fn load(on_update: impl Fn() + Send + 'static) -> Self {
        let rates = Self::default();
        let cache = read_cache();
        let stale = now_secs().saturating_sub(cache.fetched_at) > MAX_AGE.as_secs();
        rates.set(cache.rates);

        if stale {
            let rates = rates.clone();
            std::thread::spawn(move || {
                if let Ok(fresh) = fetch() {
                    write_cache(&fresh);
                    rates.set(fresh);
                    on_update();
                }
            });
        }
        rates
    }

    /// For the CLI: fetch synchronously when there is no cache yet, since the
    /// process would exit before a background refresh could finish.
    pub fn load_blocking() -> Self {
        let rates = Self::default();
        let cache = read_cache();
        let stale = now_secs().saturating_sub(cache.fetched_at) > MAX_AGE.as_secs();
        rates.set(cache.rates);
        if stale && let Ok(fresh) = fetch() {
            write_cache(&fresh);
            rates.set(fresh);
        }
        rates
    }

    #[cfg(test)]
    pub fn fixed(pairs: &[(&str, f64)]) -> Self {
        let rates = Self::default();
        rates.set(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect());
        rates
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    pub fn knows(&self, code: &str) -> bool {
        self.table.read().unwrap().contains_key(code)
    }

    fn set(&self, table: HashMap<String, f64>) {
        if table.is_empty() {
            return;
        }
        *self.table.write().unwrap() = table;
        self.version.fetch_add(1, Ordering::Relaxed);
    }
}

impl fend_core::ExchangeRateFnV2 for Rates {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        if currency == "USD" {
            return Ok(1.0);
        }
        self.table
            .read()
            .unwrap()
            .get(currency)
            .copied()
            .ok_or_else(|| format!("no exchange rate for {currency}").into())
    }
}

fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("omacus").join("rates.json"))
}

fn read_cache() -> Cache {
    cache_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_cache(rates: &HashMap<String, f64>) {
    let Some(path) = cache_path() else { return };
    let cache = Cache {
        fetched_at: now_secs(),
        rates: rates.clone(),
    };
    if let (Some(dir), Ok(json)) = (path.parent(), serde_json::to_vec(&cache)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, json);
    }
}

fn fetch() -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
    let body = ureq::get(RATES_URL).call()?.body_mut().read_to_string()?;
    Ok(serde_json::from_str::<ApiResponse>(&body)?.rates)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
