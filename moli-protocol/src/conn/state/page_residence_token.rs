use tokio::sync::watch;

use super::TargetPageAttachmentId;

/// Terminal result of observing one exact installed Page residence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetPageResidenceObservation {
    Superseded,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetPageResidenceState {
    Live(TargetPageAttachmentId),
    Superseded(TargetPageAttachmentId),
}

/// Slot-owned lifetime for one concrete Page attachment.
///
/// Dropping this publisher without first superseding it means the owning slot
/// disappeared unexpectedly. Replacing or clearing the installed Page must
/// explicitly call `supersede`, which resolves every token for that attachment.
#[derive(Debug)]
pub(crate) struct TargetPageResidencePublisher {
    sender: watch::Sender<TargetPageResidenceState>,
}

impl TargetPageResidencePublisher {
    pub(crate) fn new(attachment_id: TargetPageAttachmentId) -> Self {
        let (sender, _receiver) = watch::channel(TargetPageResidenceState::Live(attachment_id));
        Self { sender }
    }

    pub(crate) fn token(&self) -> TargetPageResidenceToken {
        TargetPageResidenceToken {
            receiver: self.sender.subscribe(),
        }
    }

    pub(crate) fn supersede(self) {
        let attachment_id = match *self.sender.borrow() {
            TargetPageResidenceState::Live(attachment_id)
            | TargetPageResidenceState::Superseded(attachment_id) => attachment_id,
        };
        self.sender
            .send_replace(TargetPageResidenceState::Superseded(attachment_id));
    }
}

/// Move-only lifetime token for one concrete installed Page attachment.
///
/// Moving the target's whole Page slot preserves this token. Replacing or
/// clearing that attachment resolves it as `Superseded`; losing the owning slot
/// without an explicit lifecycle transition resolves it as `Unavailable`.
#[derive(Debug)]
pub(crate) struct TargetPageResidenceToken {
    receiver: watch::Receiver<TargetPageResidenceState>,
}

impl TargetPageResidenceToken {
    pub(crate) async fn wait(mut self) -> TargetPageResidenceObservation {
        loop {
            match *self.receiver.borrow_and_update() {
                TargetPageResidenceState::Live(_) => {}
                TargetPageResidenceState::Superseded(_) => {
                    return TargetPageResidenceObservation::Superseded;
                }
            }
            if self.receiver.changed().await.is_err() {
                return TargetPageResidenceObservation::Unavailable;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment_id(raw: u64) -> TargetPageAttachmentId {
        TargetPageAttachmentId::from_raw_for_test(raw)
    }

    #[tokio::test]
    async fn explicit_supersede_resolves_every_attachment_token() {
        let publisher = TargetPageResidencePublisher::new(attachment_id(7));
        let first = publisher.token();
        let second = publisher.token();

        publisher.supersede();

        assert_eq!(
            first.wait().await,
            TargetPageResidenceObservation::Superseded
        );
        assert_eq!(
            second.wait().await,
            TargetPageResidenceObservation::Superseded
        );
    }

    #[tokio::test]
    async fn publisher_loss_reports_unavailable() {
        let publisher = TargetPageResidencePublisher::new(attachment_id(7));
        let token = publisher.token();

        drop(publisher);

        assert_eq!(
            token.wait().await,
            TargetPageResidenceObservation::Unavailable
        );
    }
}
