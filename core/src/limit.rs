//! Speed limits (S5 for all downloads, and the Options tab's limit for one): a shared budget of bytes per second
//! that every connection under it draws from.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A budget of bytes per second. A connection that has received a chunk takes its size from the budget and
/// sleeps off any debt, so all connections under one limit share it, whatever their number. Zero is no limit.
#[derive(Debug)]
pub struct Limiter {
    rate: AtomicU64,
    /// Bytes that may still pass now (negative: owed), and when it was last topped up.
    budget: Mutex<(f64, Instant)>,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter::new(None)
    }
}

impl Limiter {
    pub fn new(rate: Option<u64>) -> Limiter {
        Limiter {
            rate: AtomicU64::new(rate.unwrap_or(0)),
            budget: Mutex::new((0.0, Instant::now())),
        }
    }

    /// Bytes per second, or `None` for no limit.
    pub fn rate(&self) -> Option<u64> {
        Some(self.rate.load(Ordering::SeqCst)).filter(|&r| r > 0)
    }

    pub fn set_rate(&self, rate: Option<u64>) {
        self.rate.store(rate.unwrap_or(0), Ordering::SeqCst);
        *self.budget.lock().unwrap() = (0.0, Instant::now());
    }

    /// Takes `bytes` from the budget and returns how long to wait before receiving more.
    pub fn take(&self, bytes: usize) -> Duration {
        let rate = self.rate.load(Ordering::SeqCst);
        if rate == 0 {
            return Duration::ZERO;
        }
        let rate = rate as f64;
        let mut budget = self.budget.lock().unwrap();
        let now = Instant::now();
        // At most a quarter of a second's worth saved up, so the speed stays even.
        budget.0 = (budget.0 + now.duration_since(budget.1).as_secs_f64() * rate).min(rate / 4.0);
        budget.1 = now;
        budget.0 -= bytes as f64;
        if budget.0 >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-budget.0 / rate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_limit_never_waits() {
        let l = Limiter::new(None);
        assert_eq!(l.take(10_000_000), Duration::ZERO);
    }

    #[test]
    fn debt_is_paid_at_the_rate() {
        let l = Limiter::new(Some(1000));
        let wait = l.take(2000);
        assert!(
            wait > Duration::from_millis(1900) && wait <= Duration::from_millis(2000),
            "{wait:?}"
        );
        // A second connection under the same limit waits behind the first.
        assert!(l.take(1000) > Duration::from_millis(2900));
        l.set_rate(None);
        assert_eq!(l.take(1000), Duration::ZERO);
    }
}
