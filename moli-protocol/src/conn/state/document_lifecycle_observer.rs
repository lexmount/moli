use tokio::sync::watch;

/// Protocol receipt of one exact renderer Document milestone.
///
/// `Pending` is transport state only. Every other variant is terminal and is
/// published by the Page-slot projection with the exact Document/epoch
/// binding. Native progress and generic wakes alone never change this value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererDocumentLifecycleObservation {
    Pending,
    Reached,
    Interrupted,
    Superseded,
    Unavailable,
}

impl RendererDocumentLifecycleObservation {
    pub(crate) fn is_terminal(self) -> bool {
        self != Self::Pending
    }
}

/// The Page-slot side of one exact lifecycle observation.
///
/// This publisher remains with the registered lifecycle waiter. Dropping the
/// Page slot closes the channel, which the observer interprets as
/// `Unavailable`; replacement and lifecycle termination publish their typed
/// terminal before the registration is retired.
#[derive(Debug)]
pub(crate) struct RendererDocumentLifecycleObservationPublisher {
    sender: watch::Sender<RendererDocumentLifecycleObservation>,
}

impl RendererDocumentLifecycleObservationPublisher {
    pub(crate) fn publish(&self, observation: RendererDocumentLifecycleObservation) {
        assert!(
            observation.is_terminal(),
            "a lifecycle observation publisher must publish a terminal fact"
        );
        self.sender.send_replace(observation);
    }

    pub(crate) fn has_observer(&self) -> bool {
        self.sender.receiver_count() != 0
    }
}

/// A move-only wait token for one exact renderer Document milestone.
///
/// The token is deliberately independent of renderer wake routing. Its state
/// changes only when the Page-slot projection consumes the requested
/// milestone, termination, replacement, or loss of the owning Page slot.
#[derive(Debug)]
pub(crate) struct RendererDocumentLifecycleObserver {
    receiver: watch::Receiver<RendererDocumentLifecycleObservation>,
}

impl RendererDocumentLifecycleObserver {
    #[cfg(test)]
    pub(crate) fn channel(
        initial: RendererDocumentLifecycleObservation,
    ) -> (
        RendererDocumentLifecycleObservationPublisher,
        RendererDocumentLifecycleObserver,
    ) {
        let (sender, receiver) = watch::channel(initial);
        (
            RendererDocumentLifecycleObservationPublisher { sender },
            RendererDocumentLifecycleObserver { receiver },
        )
    }

    #[cfg(test)]
    pub(crate) fn resolved(observation: RendererDocumentLifecycleObservation) -> Self {
        assert!(
            observation.is_terminal(),
            "a resolved lifecycle observer requires a terminal fact"
        );
        let (publisher, observer) = Self::channel(observation);
        drop(publisher);
        observer
    }

    #[cfg(test)]
    pub(crate) fn observation(&self) -> RendererDocumentLifecycleObservation {
        *self.receiver.borrow()
    }

    pub(crate) async fn wait(mut self) -> RendererDocumentLifecycleObservation {
        loop {
            let observation = *self.receiver.borrow_and_update();
            if observation.is_terminal() {
                return observation;
            }
            if self.receiver.changed().await.is_err() {
                return RendererDocumentLifecycleObservation::Unavailable;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RendererDocumentLifecycleObservation, RendererDocumentLifecycleObserver};

    #[tokio::test]
    async fn observer_waits_for_a_typed_terminal() {
        let (publisher, observer) = RendererDocumentLifecycleObserver::channel(
            RendererDocumentLifecycleObservation::Pending,
        );
        publisher.publish(RendererDocumentLifecycleObservation::Reached);

        assert_eq!(
            observer.wait().await,
            RendererDocumentLifecycleObservation::Reached
        );
    }

    #[tokio::test]
    async fn observer_maps_publisher_loss_to_unavailable() {
        let (publisher, observer) = RendererDocumentLifecycleObserver::channel(
            RendererDocumentLifecycleObservation::Pending,
        );
        drop(publisher);

        assert_eq!(
            observer.wait().await,
            RendererDocumentLifecycleObservation::Unavailable
        );
    }
}
