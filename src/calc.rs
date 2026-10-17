//! Calculation off the window's thread. A worker owns its own engine and each
//! sheet's last calculation, so an edit redoes only the lines from the first one
//! that changed, and the window never waits on math (`100000!` can take its full
//! time limit while typing carries on).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use omacus_engine::{Engine, LineResult, Sheet};

/// Sheets whose calculations are kept; beyond this the worker starts afresh.
const KEEP: usize = 32;

struct Job {
    id: u64,
    sheet: String,
    text: String,
}

/// Answers for a sheet's text.
pub struct Done {
    pub sheet: String,
    pub text: String,
    pub results: Vec<LineResult>,
}

pub struct Calculator {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// The newest job asked for; the worker drops anything older.
    latest: Arc<AtomicU64>,
}

impl Calculator {
    /// Starts the worker. `wake` is called when answers are ready.
    pub fn new(engine: Engine, wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, done) = channel::<Done>();
        let latest = Arc::new(AtomicU64::new(0));
        let newest = latest.clone();
        std::thread::Builder::new()
            .name("omacus-calc".into())
            .spawn(move || {
                let mut sheets: HashMap<String, Sheet> = HashMap::new();
                while let Ok(mut job) = inbox.recv() {
                    // Only the newest text matters.
                    while let Ok(next) = inbox.try_recv() {
                        job = next;
                    }
                    if sheets.len() > KEEP && !sheets.contains_key(&job.sheet) {
                        sheets.clear();
                    }
                    let sheet = sheets.entry(job.sheet.clone()).or_default();
                    let stop = || newest.load(Ordering::Relaxed) != job.id;
                    let Some(results) = engine.eval_sheet_until(&job.text, sheet, stop) else {
                        continue;
                    };
                    let done = Done {
                        sheet: job.sheet,
                        text: job.text,
                        results,
                    };
                    if outbox.send(done).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .expect("start the calculation thread");
        Self { jobs, done, latest }
    }

    /// Calculate `text` for `sheet`, replacing anything still waiting.
    pub fn request(&self, sheet: &str, text: &str) {
        let id = self.latest.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.jobs.send(Job {
            id,
            sheet: sheet.to_string(),
            text: text.to_string(),
        });
    }

    /// The newest answers that have arrived, if any.
    pub fn take(&self) -> Option<Done> {
        self.done.try_iter().last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omacus_engine::Rates;
    use std::time::{Duration, Instant};

    fn engine() -> Engine {
        Engine::new(Rates::fixed(&[("USD", 1.0)])).with_clock(omacus_engine::monotonic_ms)
    }

    fn wait(calc: &Calculator) -> Done {
        let start = Instant::now();
        loop {
            if let Some(done) = calc.take() {
                return done;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "no answers");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn never_makes_the_caller_wait() {
        let calc = Calculator::new(engine(), || {});
        let slow = vec!["100000!"; 20].join("\n");
        let start = Instant::now();
        calc.request("a", &slow);
        // A newer edit while the slow sheet is still being worked out.
        calc.request("a", "2 + 2");
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "request blocked"
        );
        // The answers that arrive in the end are for the newest text.
        let mut last = wait(&calc);
        while let Some(more) = calc.take() {
            last = more;
        }
        std::thread::sleep(Duration::from_millis(300));
        if let Some(more) = calc.take() {
            last = more;
        }
        assert_eq!(last.results.len(), 1);
        assert_eq!(last.results[0].display, "4");
    }

    #[test]
    fn answers_match_calculating_from_scratch() {
        let calc = Calculator::new(engine(), || {});
        let reference = engine();
        let mut text = String::new();
        for line in [
            "rent = $1000",
            "rent + 5",
            "",
            "tax = 8%",
            "$100 + tax",
            "total",
            "x = 3",
            "x * 2",
        ] {
            text.push_str(line);
            calc.request("s", &text);
            let done = wait(&calc);
            let fresh = reference.eval_document(&text);
            let got: Vec<_> = done.results.iter().map(|r| r.display.clone()).collect();
            let want: Vec<_> = fresh.iter().map(|r| r.display.clone()).collect();
            assert_eq!(got, want, "{text:?}");
            text.push('\n');
        }
    }
}
