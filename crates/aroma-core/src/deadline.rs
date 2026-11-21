//! One query budget shared by every stage (D23). A `Deadline` is cheap to clone; all
//! clones share one cancellation flag, so another thread can stop a running query.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Deadline {
    at: Option<Instant>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("query budget exhausted during {stage}")]
pub struct Cancelled {
    pub stage: &'static str,
}

impl Deadline {
    /// No time limit; only the cancellation flag can stop the query.
    pub fn none() -> Deadline {
        Deadline {
            at: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Expires `budget` from now. A budget too large to represent means no time limit.
    pub fn after(budget: Duration) -> Deadline {
        Deadline {
            at: Instant::now().checked_add(budget),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Store `true` into the returned flag to cancel every clone of this deadline.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// `Err(Cancelled { stage })` once cancelled or once the instant is reached.
    pub fn check(&self, stage: &'static str) -> Result<(), Cancelled> {
        let expired = self.at.is_some_and(|at| Instant::now() >= at);
        if expired || self.cancel.load(Ordering::Relaxed) {
            Err(Cancelled { stage })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_never_expires() {
        assert_eq!(Deadline::none().check("search"), Ok(()));
    }

    #[test]
    fn zero_budget_is_expired() {
        let d = Deadline::after(Duration::ZERO);
        assert_eq!(d.check("x"), Err(Cancelled { stage: "x" }));
    }

    #[test]
    fn generous_budget_is_not_expired() {
        assert_eq!(
            Deadline::after(Duration::from_secs(3600)).check("prune"),
            Ok(())
        );
        assert_eq!(Deadline::after(Duration::MAX).check("prune"), Ok(()));
    }

    #[test]
    fn cancel_handle_stops_every_clone() {
        let d = Deadline::none();
        let clone = d.clone();
        d.cancel_handle().store(true, Ordering::Relaxed);
        assert_eq!(clone.check("cluster"), Err(Cancelled { stage: "cluster" }));
    }

    #[test]
    fn cancelled_message_names_the_stage() {
        assert_eq!(
            Cancelled { stage: "prune" }.to_string(),
            "query budget exhausted during prune"
        );
    }
}
