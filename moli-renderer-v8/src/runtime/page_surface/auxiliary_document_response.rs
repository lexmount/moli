use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::oneshot;

/// One accepted auxiliary navigation's canonical response. The browser's load
/// owns the producer; the opener's local projection consumes the same bytes
/// without issuing a second request or bypassing Fetch interception.
#[derive(Clone, Debug)]
pub struct RendererAuxiliaryDocumentResponse(Arc<AuxiliaryDocumentResponseProducer>);

#[derive(Debug)]
struct AuxiliaryDocumentResponseProducer {
    sender: Mutex<Option<oneshot::Sender<Result<moli_fetch::RawResponse, String>>>>,
}

pub(crate) type AuxiliaryDocumentResponseReceiver =
    oneshot::Receiver<Result<moli_fetch::RawResponse, String>>;

impl RendererAuxiliaryDocumentResponse {
    pub(crate) fn channel() -> (Self, AuxiliaryDocumentResponseReceiver) {
        let (sender, receiver) = oneshot::channel();
        (
            Self(Arc::new(AuxiliaryDocumentResponseProducer {
                sender: Mutex::new(Some(sender)),
            })),
            receiver,
        )
    }

    /// Completion is single-use across clones carried through interception
    /// and redirects. Dropping the last producer also cancels the consumer.
    pub fn complete(&self, response: Result<moli_fetch::RawResponse, String>) {
        if let Some(sender) = self.0.sender.lock().take() {
            let _ = sender.send(response);
        }
    }
}

impl PartialEq for RendererAuxiliaryDocumentResponse {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for RendererAuxiliaryDocumentResponse {}
