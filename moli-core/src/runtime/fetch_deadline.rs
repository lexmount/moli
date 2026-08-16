//! One absolute timeout budget shared by every fetch-readiness phase.

use anyhow::{Context, Result, anyhow, bail};
use std::{future::Future, time::Duration};

/// An absolute deadline for a complete fetch-readiness plan.
///
/// Cloning or passing this value to another phase never restarts the timeout.
/// The initial navigation, lifecycle target, replacement navigation, response
/// match, selector, and script waits can therefore consume one caller-owned
/// budget instead of each receiving a fresh `Duration`.
#[derive(Clone, Copy, Debug)]
pub struct FetchDeadline {
    timeout: Duration,
    at: tokio::time::Instant,
}

impl FetchDeadline {
    /// Starts a new deadline with `timeout` as its complete readiness budget.
    pub fn new(timeout: Duration) -> Result<Self> {
        let at = tokio::time::Instant::now()
            .checked_add(timeout)
            .with_context(|| {
                anyhow!(
                    "fetch readiness timeout of {} ms exceeds the supported range",
                    timeout.as_millis()
                )
            })?;
        Ok(Self { timeout, at })
    }

    /// Returns the original budget used to create this deadline.
    pub fn timeout(self) -> Duration {
        self.timeout
    }

    /// Returns the budget still available without changing the deadline.
    pub fn remaining(self) -> Duration {
        self.at
            .saturating_duration_since(tokio::time::Instant::now())
    }

    pub(super) fn at(self) -> tokio::time::Instant {
        self.at
    }

    pub(super) async fn wait<T, F>(self, phase: &'static str, future: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        match tokio::time::timeout_at(self.at, future).await {
            Ok(result) => result,
            Err(_) => bail!(
                "fetch readiness timed out after {} ms while {phase}",
                self.timeout.as_millis()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FetchDeadline;
    use anyhow::Result;
    use std::time::Duration;

    #[tokio::test]
    async fn remaining_budget_is_not_restarted_between_phases() -> Result<()> {
        let deadline = FetchDeadline::new(Duration::from_millis(250))?;
        let initial = deadline.remaining();
        tokio::time::sleep(Duration::from_millis(75)).await;
        let after_first_phase = deadline.remaining();

        assert!(initial <= Duration::from_millis(250));
        assert!(
            after_first_phase < initial,
            "passing the deadline to another phase must retain elapsed time"
        );
        assert!(after_first_phase <= Duration::from_millis(200));
        Ok(())
    }

    #[tokio::test]
    async fn wait_reports_total_budget_and_active_phase() -> Result<()> {
        let deadline = FetchDeadline::new(Duration::from_millis(25))?;
        let error = deadline
            .wait("waiting for a selector", async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(())
            })
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "fetch readiness timed out after 25 ms while waiting for a selector"
        );
        Ok(())
    }

    #[test]
    fn impossible_deadline_range_is_an_anyhow_error() {
        let error = FetchDeadline::new(Duration::MAX).unwrap_err();
        assert!(error.to_string().contains(
            "fetch readiness timeout of 18446744073709551615999 ms exceeds the supported range"
        ));
    }
}
