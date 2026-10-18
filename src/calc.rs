//! Calculation off the window's thread. A worker owns its own engine and each
//! sheet's last calculation, so an edit redoes only the lines from the first one
//! that changed, and the window never waits on math (`100000!` can take its full
//! time limit while typing carries on). Answers come back every SLICE as they're
//! worked out, so a long, slow sheet fills in from the top even while it's edited.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use omacus_engine::{Engine, LineResult, Sheet};

/// Sheets whose calculations are kept; beyond this the worker starts afresh.
const KEEP: usize = 32;
/// How often answers so far are sent back while a sheet is being worked out.
const SLICE: Duration = Duration::from_millis(100);

struct Job {
    id: u64,
    sheet: String,
    text: String,
}

/// Answers for a sheet's text: all of them, or (`complete` false) the first lines'.
pub struct Done {
    pub sheet: String,
    pub text: String,
    pub results: Vec<LineResult>,
    pub complete: bool,
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
                    loop {
                        let newer = || newest.load(Ordering::Relaxed) != job.id;
                        let until = Instant::now() + SLICE;
                        let stop = || newer() || Instant::now() > until;
                        let results = engine.eval_sheet_until(&job.text, sheet, stop);
                        if results.is_none() && newer() {
                            break;
                        }
                        let complete = results.is_some();
                        let done = Done {
                            sheet: job.sheet.clone(),
                            text: job.text.clone(),
                            results: results.unwrap_or_else(|| sheet.done().to_vec()),
                            complete,
                        };
                        if outbox.send(done).is_err() {
                            return;
                        }
                        wake();
                        if complete {
                            break;
                        }
                    }
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
            if let Some(done) = calc.take().filter(|d| d.complete) {
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

    /// Switching sheets mid-calculation: the newest answers are the new sheet's, and
    /// the sheet that was cut off still gets the right answers when it comes back
    /// (its saved work mustn't keep a half-finished calculation).
    #[test]
    fn switching_sheets_mid_calculation() {
        let calc = Calculator::new(engine(), || {});
        let reference = engine();
        let a = ["x = 2", "100000!", "x * 3", "100000!", "total", "x + 1"].join("\n");
        for _ in 0..5 {
            calc.request("a", &a);
            std::thread::sleep(Duration::from_millis(20));
            calc.request("b", "1 + 1");
            let mut last = wait(&calc);
            std::thread::sleep(Duration::from_millis(50));
            while let Some(more) = calc.take() {
                last = more;
            }
            assert_eq!(last.sheet, "b");
            assert_eq!(last.results[0].display, "2");
        }
        calc.request("a", &a);
        let done = wait(&calc);
        assert_eq!(done.sheet, "a");
        let got: Vec<_> = done
            .results
            .iter()
            .map(|r| (r.display.clone(), r.note))
            .collect();
        let want: Vec<_> = reference
            .eval_document(&a)
            .iter()
            .map(|r| (r.display.clone(), r.note))
            .collect();
        assert_eq!(got, want);
    }

    /// A slow sheet fills in from the top: the first lines' answers arrive before
    /// the slow lines below them are done, and then all of them (from the third
    /// review: a slow sheet showed nothing until every line was worked out).
    #[test]
    fn answers_arrive_part_way_through() {
        let calc = Calculator::new(engine(), || {});
        let text = ["1 + 1", "100000!", "100000!", "100000!", "100000!", "2 + 2"].join("\n");
        calc.request("s", &text);
        let start = Instant::now();
        let mut part = None;
        let done = loop {
            assert!(start.elapsed() < Duration::from_secs(30), "no answers");
            match calc.take() {
                Some(d) if d.complete => break d,
                Some(d) => part = part.or(Some(d)),
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        let part = part.expect("answers so far, before the slow lines were done");
        assert_eq!(part.text, text);
        assert!(part.results.len() < 6);
        assert_eq!(part.results[0].display, "2");
        assert_eq!(done.results.len(), 6);
        assert_eq!(done.results[5].display, "4");
    }
}
