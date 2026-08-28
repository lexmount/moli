use std::{
    fmt,
    sync::{Arc, Weak},
};

use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::{
    RendererOutputCursor, RendererOutputFenceLeaseId, RendererOutputPublication,
    RendererOutputResidenceIdentity, RendererOutputStreamControl, RendererOutputStreamIdentity,
};
use crate::runtime::RendererPageOutputOwnerReservationId;
use crate::runtime::{PageId, RendererOwnerLocalHostId};

const DEFAULT_MAX_PENDING_MESSAGES: usize = 2_048;
const DEFAULT_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_OBSERVATION_MESSAGES: usize = 1_536;
const DEFAULT_MAX_OBSERVATION_BYTES: usize = 48 * 1024 * 1024;
const DEFAULT_MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// One concrete message on the renderer-to-protocol output transport.
///
/// Unlike the removed source-shaped wake, every variant owns the exact fact
/// that protocol must admit. Routing is derived from the typed stream
/// residence; a Page identity is therefore never fabricated for worker
/// output, and protocol never re-enters renderer state to discover a payload.
#[derive(Clone, Debug, PartialEq)]
pub enum RendererOutputTransportMessage {
    StreamControl(RendererOutputStreamControl),
    /// Releases the protocol owner reserved before renderer bootstrap.
    ///
    /// A successful bootstrap sends `Opened` first from the newly-created
    /// journal; a pre-journal failure sends only this marker. Both use the
    /// same transport, so protocol never has to guess whether a concurrently
    /// completing reservation may still open later.
    PageReservationReleased {
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
        reservation_id: RendererPageOutputOwnerReservationId,
    },
    /// Declares that a cursor is leaving the concrete transport on an
    /// independent command/navigation completion channel.
    CursorLeaseDeclared {
        cursor: RendererOutputCursor,
        lease_id: RendererOutputFenceLeaseId,
    },
    /// Releases one previously declared external cursor lease.
    CursorLeaseReleased {
        stream: RendererOutputStreamIdentity,
        lease_id: RendererOutputFenceLeaseId,
    },
    Publication(RendererOutputPublication),
}

impl RendererOutputTransportMessage {
    pub(crate) fn page_reservation_released(
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
        reservation_id: RendererPageOutputOwnerReservationId,
    ) -> Self {
        Self::PageReservationReleased {
            owner_local_host_id,
            page_id,
            reservation_id,
        }
    }

    pub fn residence(&self) -> RendererOutputResidenceIdentity {
        match self {
            Self::StreamControl(RendererOutputStreamControl::Opened { stream })
            | Self::StreamControl(RendererOutputStreamControl::Closed { stream, .. }) => {
                stream.residence()
            }
            Self::PageReservationReleased {
                owner_local_host_id,
                page_id,
                ..
            } => RendererOutputResidenceIdentity::Page {
                owner_local_host_id: *owner_local_host_id,
                page_id: *page_id,
            },
            Self::CursorLeaseDeclared { cursor, .. } => cursor.stream().residence(),
            Self::CursorLeaseReleased { stream, .. } => stream.residence(),
            Self::Publication(publication) => publication.cursor().stream().residence(),
        }
    }

    fn admission_class(&self) -> RendererOutputTransportAdmissionClass {
        match self {
            Self::Publication(publication)
                if !publication.requires_essential_transport_admission() =>
            {
                RendererOutputTransportAdmissionClass::Observation
            }
            Self::StreamControl(_)
            | Self::PageReservationReleased { .. }
            | Self::CursorLeaseDeclared { .. }
            | Self::CursorLeaseReleased { .. }
            | Self::Publication(_) => RendererOutputTransportAdmissionClass::Essential,
        }
    }

    fn transport_charge_bytes(&self) -> usize {
        match self {
            Self::Publication(publication) => publication.transport_charge_bytes(),
            Self::StreamControl(_)
            | Self::PageReservationReleased { .. }
            | Self::CursorLeaseDeclared { .. }
            | Self::CursorLeaseReleased { .. } => std::mem::size_of::<Self>().max(256),
        }
    }
}

impl From<RendererOutputStreamControl> for RendererOutputTransportMessage {
    fn from(control: RendererOutputStreamControl) -> Self {
        Self::StreamControl(control)
    }
}

impl From<RendererOutputPublication> for RendererOutputTransportMessage {
    fn from(publication: RendererOutputPublication) -> Self {
        Self::Publication(publication)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererOutputTransportAdmissionClass {
    Observation,
    Essential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RendererOutputTransportLimits {
    pub max_pending_messages: usize,
    pub max_pending_bytes: usize,
    pub max_observation_messages: usize,
    pub max_observation_bytes: usize,
    pub max_message_bytes: usize,
}

impl Default for RendererOutputTransportLimits {
    fn default() -> Self {
        Self {
            max_pending_messages: DEFAULT_MAX_PENDING_MESSAGES,
            max_pending_bytes: DEFAULT_MAX_PENDING_BYTES,
            max_observation_messages: DEFAULT_MAX_OBSERVATION_MESSAGES,
            max_observation_bytes: DEFAULT_MAX_OBSERVATION_BYTES,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

impl RendererOutputTransportLimits {
    fn validate(self) {
        assert!(
            self.max_pending_messages > 0 && self.max_pending_bytes > 0,
            "renderer output transport limits must be non-zero"
        );
        assert!(
            self.max_observation_messages < self.max_pending_messages,
            "renderer output transport must reserve messages for control, owner actions, and terminal responses"
        );
        assert!(
            self.max_observation_bytes < self.max_pending_bytes,
            "renderer output transport must reserve bytes for control, owner actions, and terminal responses"
        );
        assert!(
            self.max_message_bytes <= self.max_pending_bytes,
            "one renderer output message cannot exceed the whole transport budget"
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererOutputTransportDiagnostics {
    pub pending_messages: usize,
    pub pending_bytes: usize,
    pub pending_observation_messages: usize,
    pub pending_observation_bytes: usize,
    pub peak_pending_messages: usize,
    pub peak_pending_bytes: usize,
    pub peak_pending_observation_messages: usize,
    pub peak_pending_observation_bytes: usize,
    pub admitted_messages: u64,
    pub admitted_bytes: u64,
    pub admitted_page_messages: u64,
    pub admitted_shared_worker_messages: u64,
    pub admitted_service_worker_messages: u64,
    pub admitted_observation_publications: u64,
    pub admitted_essential_messages: u64,
    pub terminal: bool,
}

#[derive(Debug)]
struct RendererOutputTransportBudgetState {
    limits: RendererOutputTransportLimits,
    diagnostics: RendererOutputTransportDiagnostics,
}

impl RendererOutputTransportBudgetState {
    fn reserve(
        &mut self,
        class: RendererOutputTransportAdmissionClass,
        residence: RendererOutputResidenceIdentity,
        bytes: usize,
    ) -> bool {
        if self.diagnostics.terminal || bytes > self.limits.max_message_bytes {
            return false;
        }
        let next_messages = self.diagnostics.pending_messages.saturating_add(1);
        let next_bytes = self.diagnostics.pending_bytes.saturating_add(bytes);
        if next_messages > self.limits.max_pending_messages
            || next_bytes > self.limits.max_pending_bytes
        {
            return false;
        }
        if class == RendererOutputTransportAdmissionClass::Observation {
            let next_observation_messages = self
                .diagnostics
                .pending_observation_messages
                .saturating_add(1);
            let next_observation_bytes = self
                .diagnostics
                .pending_observation_bytes
                .saturating_add(bytes);
            if next_observation_messages > self.limits.max_observation_messages
                || next_observation_bytes > self.limits.max_observation_bytes
            {
                return false;
            }
            self.diagnostics.pending_observation_messages = next_observation_messages;
            self.diagnostics.pending_observation_bytes = next_observation_bytes;
            self.diagnostics.peak_pending_observation_messages = self
                .diagnostics
                .peak_pending_observation_messages
                .max(next_observation_messages);
            self.diagnostics.peak_pending_observation_bytes = self
                .diagnostics
                .peak_pending_observation_bytes
                .max(next_observation_bytes);
            self.diagnostics.admitted_observation_publications = self
                .diagnostics
                .admitted_observation_publications
                .saturating_add(1);
        } else {
            self.diagnostics.admitted_essential_messages = self
                .diagnostics
                .admitted_essential_messages
                .saturating_add(1);
        }
        self.diagnostics.pending_messages = next_messages;
        self.diagnostics.pending_bytes = next_bytes;
        self.diagnostics.peak_pending_messages =
            self.diagnostics.peak_pending_messages.max(next_messages);
        self.diagnostics.peak_pending_bytes = self.diagnostics.peak_pending_bytes.max(next_bytes);
        self.diagnostics.admitted_messages = self.diagnostics.admitted_messages.saturating_add(1);
        self.diagnostics.admitted_bytes = self
            .diagnostics
            .admitted_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        match residence {
            RendererOutputResidenceIdentity::Page { .. } => {
                self.diagnostics.admitted_page_messages =
                    self.diagnostics.admitted_page_messages.saturating_add(1);
            }
            RendererOutputResidenceIdentity::SharedWorker { .. } => {
                self.diagnostics.admitted_shared_worker_messages = self
                    .diagnostics
                    .admitted_shared_worker_messages
                    .saturating_add(1);
            }
            RendererOutputResidenceIdentity::ServiceWorker { .. } => {
                self.diagnostics.admitted_service_worker_messages = self
                    .diagnostics
                    .admitted_service_worker_messages
                    .saturating_add(1);
            }
        }
        true
    }

    fn release(&mut self, class: RendererOutputTransportAdmissionClass, bytes: usize) {
        self.diagnostics.pending_messages = self
            .diagnostics
            .pending_messages
            .checked_sub(1)
            .expect("renderer output transport message accounting underflow");
        self.diagnostics.pending_bytes = self
            .diagnostics
            .pending_bytes
            .checked_sub(bytes)
            .expect("renderer output transport byte accounting underflow");
        if class == RendererOutputTransportAdmissionClass::Observation {
            self.diagnostics.pending_observation_messages = self
                .diagnostics
                .pending_observation_messages
                .checked_sub(1)
                .expect("renderer output observation accounting underflow");
            self.diagnostics.pending_observation_bytes = self
                .diagnostics
                .pending_observation_bytes
                .checked_sub(bytes)
                .expect("renderer output observation byte accounting underflow");
        }
    }

    fn terminate(&mut self) -> bool {
        if self.diagnostics.terminal {
            return false;
        }
        self.diagnostics.terminal = true;
        true
    }
}

#[derive(Debug)]
struct RendererOutputTransportShared {
    /// Serializes admission with enqueue and terminal publication.
    ///
    /// The budget mutex alone is not enough: one sender could reserve a
    /// message, another sender could publish `Terminal`, and the first sender
    /// could then enqueue its already-admitted message behind the terminal
    /// sentinel. Keeping the non-blocking `send()` operation in this short
    /// critical section makes terminal an exact FIFO boundary without ever
    /// awaiting from a V8 callback or owner turn.
    send_order: Mutex<()>,
    budget: Mutex<RendererOutputTransportBudgetState>,
}

struct RendererOutputTransportReservation {
    shared: Weak<RendererOutputTransportShared>,
    class: RendererOutputTransportAdmissionClass,
    bytes: usize,
}

impl Drop for RendererOutputTransportReservation {
    fn drop(&mut self) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        shared.budget.lock().release(self.class, self.bytes);
    }
}

enum RendererOutputTransportEnvelope {
    Message {
        message: RendererOutputTransportMessage,
        _reservation: RendererOutputTransportReservation,
    },
    Terminal,
}

#[derive(Clone)]
pub struct RendererOutputTransportSender {
    tx: mpsc::UnboundedSender<RendererOutputTransportEnvelope>,
    shared: Arc<RendererOutputTransportShared>,
}

impl fmt::Debug for RendererOutputTransportSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererOutputTransportSender")
            .field("diagnostics", &self.diagnostics())
            .finish_non_exhaustive()
    }
}

impl RendererOutputTransportSender {
    pub fn send(
        &self,
        message: RendererOutputTransportMessage,
    ) -> Result<(), RendererOutputTransportSendError> {
        let _send_order = self.shared.send_order.lock();
        let class = message.admission_class();
        let residence = message.residence();
        let bytes = message.transport_charge_bytes();
        let admitted = self.shared.budget.lock().reserve(class, residence, bytes);
        if !admitted {
            self.terminate();
            return Err(RendererOutputTransportSendError {
                message: Box::new(message),
            });
        }
        let envelope = RendererOutputTransportEnvelope::Message {
            message,
            _reservation: RendererOutputTransportReservation {
                shared: Arc::downgrade(&self.shared),
                class,
                bytes,
            },
        };
        self.tx.send(envelope).map_err(|error| {
            self.shared.budget.lock().terminate();
            let RendererOutputTransportEnvelope::Message { message, .. } = error.0 else {
                unreachable!("only concrete renderer output is sent through this branch")
            };
            RendererOutputTransportSendError {
                message: Box::new(message),
            }
        })
    }

    pub fn same_channel(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    pub fn diagnostics(&self) -> RendererOutputTransportDiagnostics {
        self.shared.budget.lock().diagnostics
    }

    fn terminate(&self) {
        if self.shared.budget.lock().terminate() {
            // This one fixed-size sentinel is deliberately outside the
            // publication budget. It is sent at most once and closes the
            // consumer after every already-admitted message, so a saturated
            // queue can never hide terminal from command-response fences.
            let _ = self.tx.send(RendererOutputTransportEnvelope::Terminal);
        }
    }
}

pub struct RendererOutputTransportReceiver {
    rx: mpsc::UnboundedReceiver<RendererOutputTransportEnvelope>,
    shared: Arc<RendererOutputTransportShared>,
    closed: bool,
}

impl RendererOutputTransportReceiver {
    pub fn diagnostics(&self) -> RendererOutputTransportDiagnostics {
        self.shared.budget.lock().diagnostics
    }

    pub fn is_terminal(&self) -> bool {
        self.diagnostics().terminal
    }

    /// Whether the consumer has reached the ordered transport terminal.
    ///
    /// `is_terminal()` may become true while already-admitted FIFO messages
    /// still precede the sentinel. Owners may stop accepting new commands only
    /// after this method becomes true.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub async fn recv(&mut self) -> Option<RendererOutputTransportMessage> {
        match self.rx.recv().await {
            Some(RendererOutputTransportEnvelope::Message { message, .. }) => Some(message),
            Some(RendererOutputTransportEnvelope::Terminal) => {
                self.closed = true;
                self.rx.close();
                while self.rx.try_recv().is_ok() {}
                None
            }
            None => {
                self.closed = true;
                None
            }
        }
    }

    pub fn try_recv(
        &mut self,
    ) -> Result<RendererOutputTransportMessage, mpsc::error::TryRecvError> {
        match self.rx.try_recv() {
            Ok(RendererOutputTransportEnvelope::Message { message, .. }) => Ok(message),
            Ok(RendererOutputTransportEnvelope::Terminal) => {
                self.closed = true;
                self.rx.close();
                while self.rx.try_recv().is_ok() {}
                Err(mpsc::error::TryRecvError::Disconnected)
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.closed = true;
                Err(mpsc::error::TryRecvError::Disconnected)
            }
            Err(mpsc::error::TryRecvError::Empty) => Err(mpsc::error::TryRecvError::Empty),
        }
    }
}

pub struct RendererOutputTransportSendError {
    message: Box<RendererOutputTransportMessage>,
}

impl RendererOutputTransportSendError {
    pub fn into_inner(self) -> RendererOutputTransportMessage {
        *self.message
    }
}

impl fmt::Debug for RendererOutputTransportSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererOutputTransportSendError")
            .field("residence", &self.message.residence())
            .finish_non_exhaustive()
    }
}

pub fn renderer_output_transport_channel() -> (
    RendererOutputTransportSender,
    RendererOutputTransportReceiver,
) {
    renderer_output_transport_channel_with_limits(RendererOutputTransportLimits::default())
}

fn renderer_output_transport_channel_with_limits(
    limits: RendererOutputTransportLimits,
) -> (
    RendererOutputTransportSender,
    RendererOutputTransportReceiver,
) {
    limits.validate();
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(RendererOutputTransportShared {
        send_order: Mutex::new(()),
        budget: Mutex::new(RendererOutputTransportBudgetState {
            limits,
            diagnostics: RendererOutputTransportDiagnostics::default(),
        }),
    });
    (
        RendererOutputTransportSender {
            tx,
            shared: shared.clone(),
        },
        RendererOutputTransportReceiver {
            rx,
            shared,
            closed: false,
        },
    )
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(in crate::runtime) struct RendererOutputTransportTestLimits {
    pub max_pending_messages: usize,
    pub max_pending_bytes: usize,
    pub max_observation_messages: usize,
    pub max_observation_bytes: usize,
    pub max_message_bytes: usize,
}

#[cfg(test)]
pub(in crate::runtime) fn renderer_output_transport_channel_with_test_limits(
    limits: RendererOutputTransportTestLimits,
) -> (
    RendererOutputTransportSender,
    RendererOutputTransportReceiver,
) {
    renderer_output_transport_channel_with_limits(RendererOutputTransportLimits {
        max_pending_messages: limits.max_pending_messages,
        max_pending_bytes: limits.max_pending_bytes,
        max_observation_messages: limits.max_observation_messages,
        max_observation_bytes: limits.max_observation_bytes,
        max_message_bytes: limits.max_message_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use moli_shared_worker::SharedWorkerInstanceId;

    use super::*;
    use crate::runtime::{
        RendererOutputItem, RendererOutputRecord, RendererOwnerAction,
        RendererPendingDownloadActivation, RendererPendingDownloadResponse,
        RendererProtocolObservation, RendererSharedWorkerTargetEvent,
    };

    fn test_limits() -> RendererOutputTransportLimits {
        RendererOutputTransportLimits {
            max_pending_messages: 4,
            max_pending_bytes: 64 * 1024,
            max_observation_messages: 1,
            max_observation_bytes: 8 * 1024,
            max_message_bytes: 32 * 1024,
        }
    }

    fn stream() -> RendererOutputStreamIdentity {
        RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(7)
    }

    fn observation_record(text_bytes: usize) -> RendererOutputRecord {
        RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
            RendererProtocolObservation::RuntimeLifecycleError {
                text: "x".repeat(text_bytes),
                execution_context_id: None,
            },
        ))
    }

    fn owner_action_record() -> RendererOutputRecord {
        RendererOutputRecord::new_for_test(RendererOutputItem::OwnerAction(
            RendererOwnerAction::SharedWorkerTargetLifecycle(
                RendererSharedWorkerTargetEvent::Destroyed {
                    instance_id: SharedWorkerInstanceId::from_u64(7),
                },
            ),
        ))
    }

    fn publication(
        sequence: u64,
        records: Vec<RendererOutputRecord>,
    ) -> RendererOutputTransportMessage {
        RendererOutputPublication::new_for_test(
            RendererOutputCursor::new_for_test(stream(), sequence),
            records,
        )
        .into()
    }

    #[tokio::test]
    async fn observation_budget_exhaustion_is_an_ordered_transport_terminal() {
        let (sender, mut receiver) = renderer_output_transport_channel_with_limits(test_limits());
        let opened =
            RendererOutputTransportMessage::StreamControl(RendererOutputStreamControl::Opened {
                stream: stream(),
            });
        let first = publication(1, vec![observation_record(1)]);

        sender.send(opened.clone()).expect("stream open admission");
        sender
            .send(first.clone())
            .expect("first observation admission");
        assert!(
            sender
                .send(publication(2, vec![observation_record(1)]))
                .is_err(),
            "the observation ceiling must not borrow the essential reserve"
        );
        assert!(sender.diagnostics().terminal);
        assert!(receiver.is_terminal());
        assert!(
            !receiver.is_closed(),
            "the consumer must drain admitted FIFO messages before closing"
        );

        assert_eq!(receiver.recv().await, Some(opened));
        assert_eq!(receiver.recv().await, Some(first));
        assert_eq!(receiver.recv().await, None);
        assert!(receiver.is_terminal());
        assert!(receiver.is_closed());
        assert_eq!(receiver.diagnostics().pending_messages, 0);
    }

    #[tokio::test]
    async fn mixed_publication_is_admitted_atomically_from_the_essential_reserve() {
        let (sender, mut receiver) = renderer_output_transport_channel_with_limits(test_limits());
        let observation = publication(1, vec![observation_record(1)]);
        let mixed = publication(2, vec![observation_record(1), owner_action_record()]);

        sender
            .send(observation.clone())
            .expect("observation admission");
        sender
            .send(mixed.clone())
            .expect("mixed publication must use the essential reserve as one envelope");

        let diagnostics = sender.diagnostics();
        assert_eq!(diagnostics.pending_observation_messages, 1);
        assert_eq!(diagnostics.admitted_observation_publications, 1);
        assert_eq!(diagnostics.admitted_essential_messages, 1);
        assert_eq!(receiver.recv().await, Some(observation));
        assert_eq!(receiver.recv().await, Some(mixed));
        assert!(!sender.diagnostics().terminal);
    }

    #[tokio::test]
    async fn dequeue_releases_observation_budget_for_the_next_publication() {
        let (sender, mut receiver) = renderer_output_transport_channel_with_limits(test_limits());
        let first = publication(1, vec![observation_record(1)]);
        let second = publication(2, vec![observation_record(1)]);

        sender.send(first.clone()).expect("first admission");
        assert_eq!(receiver.recv().await, Some(first));
        assert_eq!(sender.diagnostics().pending_messages, 0);
        sender
            .send(second.clone())
            .expect("dequeue must release the exact reservation");
        assert_eq!(receiver.recv().await, Some(second));
    }

    #[tokio::test]
    async fn one_oversized_message_terminates_without_entering_the_queue() {
        let mut limits = test_limits();
        limits.max_message_bytes = 4 * 1024;
        let (sender, mut receiver) = renderer_output_transport_channel_with_limits(limits);

        assert!(
            sender
                .send(publication(1, vec![observation_record(4 * 1024)]))
                .is_err()
        );
        assert_eq!(receiver.recv().await, None);
        assert!(receiver.is_terminal());
        assert!(receiver.is_closed());
        assert_eq!(receiver.diagnostics().pending_messages, 0);
    }

    #[tokio::test]
    async fn essential_download_body_obeys_the_single_message_limit() {
        let mut limits = test_limits();
        limits.max_message_bytes = 4 * 1024;
        let (sender, mut receiver) = renderer_output_transport_channel_with_limits(limits);
        let download = RendererOutputRecord::new_for_test(RendererOutputItem::OwnerAction(
            RendererOwnerAction::Download(RendererPendingDownloadActivation {
                url: "https://example.test/download".to_owned(),
                suggested_filename: Some("payload.bin".to_owned()),
                response: Some(RendererPendingDownloadResponse {
                    final_url: "https://example.test/payload.bin".to_owned(),
                    status: 200,
                    headers: Vec::new(),
                    body: vec![0; 8 * 1024],
                }),
            }),
        ));

        assert!(sender.send(publication(1, vec![download])).is_err());
        assert_eq!(receiver.recv().await, None);
        assert!(receiver.is_terminal());
        assert!(receiver.is_closed());
        assert_eq!(receiver.diagnostics().pending_messages, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_last_admission_cannot_enqueue_after_terminal() {
        let mut limits = test_limits();
        limits.max_pending_messages = 1;
        limits.max_observation_messages = 0;
        let (sender, mut receiver) = renderer_output_transport_channel_with_limits(limits);
        let barrier = Arc::new(Barrier::new(8));
        let mut workers = Vec::new();
        for sequence in 1..=8 {
            let sender = sender.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                sender.send(publication(sequence, vec![owner_action_record()]))
            }));
        }

        let admitted = workers
            .into_iter()
            .map(|worker| worker.join().expect("transport sender thread"))
            .filter(Result::is_ok)
            .count();
        assert_eq!(admitted, 1);
        assert!(matches!(
            receiver.recv().await,
            Some(RendererOutputTransportMessage::Publication(_))
        ));
        assert_eq!(receiver.recv().await, None);
        assert!(receiver.try_recv().is_err());
    }
}
