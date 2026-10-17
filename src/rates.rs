//! Currency exchange rates for fend, cached on disk and refreshed in the background.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const RATES_URL: &str = "https://open.er-api.com/v6/latest/USD";
const MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);
/// After a failed refresh (offline), wait this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

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
    /// When the rates in `table` were fetched, and when a refresh was last tried.
    fetched_at: Arc<AtomicU64>,
    tried_at: Arc<AtomicU64>,
    fetching: Arc<AtomicBool>,
}

impl Rates {
    /// Load cached rates, then refresh them on a background thread if stale.
    pub fn load(on_update: impl Fn() + Send + 'static) -> Self {
        let rates = Self::default();
        let cache = read_cache();
        rates.fetched_at.store(cache.fetched_at, Ordering::Relaxed);
        rates.set(cache.rates);
        rates.refresh_if_stale(on_update);
        rates
    }

    /// Fetch new rates in the background when they're over 12 hours old. Called at
    /// launch and then on every frame (two atomic reads when nothing's due), so a
    /// window left open for days stays current.
    pub fn refresh_if_stale(&self, on_update: impl Fn() + Send + 'static) {
        let now = now_secs();
        let stale = now.saturating_sub(self.fetched_at.load(Ordering::Relaxed)) > MAX_AGE.as_secs();
        let retry =
            now.saturating_sub(self.tried_at.load(Ordering::Relaxed)) > RETRY_AFTER.as_secs();
        if !stale || !retry || self.fetching.swap(true, Ordering::Relaxed) {
            return;
        }
        self.tried_at.store(now, Ordering::Relaxed);
        let rates = self.clone();
        std::thread::spawn(move || {
            if let Ok(fresh) = fetch() {
                write_cache(&fresh);
                rates.fetched_at.store(now_secs(), Ordering::Relaxed);
                rates.set(fresh);
                on_update();
            }
            rates.fetching.store(false, Ordering::Relaxed);
        });
    }

    /// For the CLI: fetch synchronously when the cache is stale, since the process
    /// would exit before a background refresh could finish.
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

/// The HTTP client for rates and updates, with timeouts so being offline (or a
/// stalled connection) never hangs the command line.
pub fn http() -> &'static ureq::Agent {
    static AGENT: std::sync::LazyLock<ureq::Agent> = std::sync::LazyLock::new(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into()
    });
    &AGENT
}

fn fetch() -> Result<HashMap<String, f64>, Box<dyn std::error::Error>> {
    let body = http().get(RATES_URL).call()?.body_mut().read_to_string()?;
    Ok(serde_json::from_str::<ApiResponse>(&body)?.rates)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
