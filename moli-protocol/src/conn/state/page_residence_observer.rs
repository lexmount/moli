use tokio::sync::watch;

/// Terminal result of observing one exact installed Page residence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetPageResidenceObservation {
    Superseded,
    Unavailable,
}

/// Move-only notification that an installed Page generation was replaced.
///
/// The observer is tied directly to the target's Page slot. Moving that slot
/// between active and background storage preserves the observation; advancing
/// its installed Page generation publishes `Superseded`. Losing the slot
/// entirely closes the channel and produces `Unavailable`.
#[derive(Debug)]
pub(crate) struct TargetPageResidenceObserver {
    state: TargetPageResidenceObserverState,
}

#[derive(Debug)]
enum TargetPageResidenceObserverState {
    Watching {
        expected_generation: u64,
        receiver: watch::Receiver<u64>,
    },
    Resolved(TargetPageResidenceObservation),
}

impl TargetPageResidenceObserver {
    pub(crate) fn new(expected_generation: u64, receiver: watch::Receiver<u64>) -> Self {
        Self {
            state: TargetPageResidenceObserverState::Watching {
                expected_generation,
                receiver,
            },
        }
    }

    pub(crate) fn resolved(observation: TargetPageResidenceObservation) -> Self {
        Self {
            state: TargetPageResidenceObserverState::Resolved(observation),
        }
    }

    pub(crate) async fn wait(self) -> TargetPageResidenceObservation {
        let (expected_generation, mut receiver) = match self.state {
            TargetPageResidenceObserverState::Watching {
                expected_generation,
                receiver,
            } => (expected_generation, receiver),
            TargetPageResidenceObserverState::Resolved(observation) => return observation,
        };
        loop {
            if *receiver.borrow_and_update() != expected_generation {
                return TargetPageResidenceObservation::Superseded;
            }
            if receiver.changed().await.is_err() {
                return TargetPageResidenceObservation::Unavailable;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn generation_change_publishes_superseded() {
        let (publisher, receiver) = watch::channel(7);
        let observer = TargetPageResidenceObserver::new(7, receiver);

        publisher.send_replace(8);

        assert_eq!(
            observer.wait().await,
            TargetPageResidenceObservation::Superseded
        );
    }

    #[tokio::test]
    async fn publisher_loss_reports_unavailable() {
        let (publisher, receiver) = watch::channel(7);
        let observer = TargetPageResidenceObserver::new(7, receiver);

        drop(publisher);

        assert_eq!(
            observer.wait().await,
            TargetPageResidenceObservation::Unavailable
        );
    }
}
