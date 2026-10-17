//! The exchange-rate table fend reads. Where the rates come from (a file, the
//! network, the browser) is up to whoever fills it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Shared, cheaply clonable rate table: units of each currency per 1 USD.
/// `version` bumps whenever new rates arrive, so a UI knows to re-evaluate.
#[derive(Clone, Default)]
pub struct Rates {
    table: Arc<RwLock<HashMap<String, f64>>>,
    version: Arc<AtomicU64>,
}

impl Rates {
    pub fn fixed(pairs: &[(&str, f64)]) -> Self {
        let rates = Self::default();
        rates.set(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect());
        rates
    }

    /// Replace the table; an empty one (a failed fetch) is ignored.
    pub fn set(&self, table: HashMap<String, f64>) {
        if table.is_empty() {
            return;
        }
        *self.table.write().unwrap() = table;
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    pub fn knows(&self, code: &str) -> bool {
        self.table.read().unwrap().contains_key(code)
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
