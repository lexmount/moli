use std::{
    fmt,
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
};

/// Destination that owns the terminal response of one renderer Inspector
/// command.
///
/// Frontend DevTools commands may publish directly through their concrete
/// session capability. Internal protocol adapters retain a private command
/// reply channel even when they synthesize the same CDP method.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererInspectorResponseDelivery {
    CommandReply,
    DevToolsSession,
}

static NEXT_RENDERER_DEVTOOLS_AGENT_TOKEN: AtomicU64 = AtomicU64::new(1);
static NEXT_RENDERER_AGENT_ATTACHMENT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_RENDERER_DEVTOOLS_COMMAND_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DevToolsSessionKey {
    Primary,
    Attached(String),
}

impl DevToolsSessionKey {
    pub fn from_wire_session_id(session_id: Option<&str>) -> Self {
        match session_id {
            Some(session_id) => Self::Attached(session_id.to_owned()),
            None => Self::Primary,
        }
    }

    pub fn wire_session_id(&self) -> Option<&str> {
        match self {
            Self::Primary => None,
            Self::Attached(session_id) => Some(session_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RendererDevToolsAgentToken(NonZeroU64);

impl RendererDevToolsAgentToken {
    pub fn allocate() -> Self {
        Self(allocate_nonzero_u64(
            &NEXT_RENDERER_DEVTOOLS_AGENT_TOKEN,
            "renderer DevTools agent token",
        ))
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// Identity of one live browser-to-renderer DevTools attachment.
///
/// This is a capability identity, not a chronological epoch. A replacement
/// renderer receives a different identity, but callers must establish
/// equality before comparing any attachment-local ingress or output position;
/// the numeric value does not define ordering between attachments.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererAgentAttachmentId(NonZeroU64);

impl RendererAgentAttachmentId {
    pub fn allocate() -> Self {
        Self(allocate_nonzero_u64(
            &NEXT_RENDERER_AGENT_ATTACHMENT_ID,
            "renderer agent attachment id",
        ))
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// Process-unique identity of one command admitted to a renderer DevTools
/// endpoint.
///
/// This is deliberately not an ordering primitive. Receiver-local ingress
/// order and session-local output order use their own types below, so a
/// process-global allocation gap can never be mistaken for a missing command.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RendererDevToolsCommandId(NonZeroU64);

impl RendererDevToolsCommandId {
    pub fn allocate() -> Self {
        Self(allocate_nonzero_u64(
            &NEXT_RENDERER_DEVTOOLS_COMMAND_ID,
            "renderer DevTools command id",
        ))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrontendCommandId(u64);

impl FrontendCommandId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for FrontendCommandId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererCallId(i32);

impl RendererCallId {
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> i32 {
        self.0
    }
}

impl TryFrom<FrontendCommandId> for RendererCallId {
    type Error = RendererCallIdOutOfRange;

    fn try_from(value: FrontendCommandId) -> Result<Self, Self::Error> {
        i32::try_from(value.get())
            .map(Self)
            .map_err(|_| RendererCallIdOutOfRange(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererCallIdOutOfRange(FrontendCommandId);

impl RendererCallIdOutOfRange {
    pub const fn frontend_command_id(self) -> FrontendCommandId {
        self.0
    }
}

impl fmt::Display for RendererCallIdOutOfRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "frontend command id {} exceeds the V8 Inspector i32 call-id range",
            self.0.get()
        )
    }
}

impl std::error::Error for RendererCallIdOutOfRange {}

fn allocate_nonzero_u64(counter: &AtomicU64, name: &str) -> NonZeroU64 {
    let raw = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .unwrap_or_else(|_| panic!("{name} exhausted"));
    NonZeroU64::new(raw).unwrap_or_else(|| panic!("{name} allocator returned zero"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn devtools_session_key_round_trips_wire_identity() {
        let primary = DevToolsSessionKey::from_wire_session_id(None);
        assert_eq!(primary, DevToolsSessionKey::Primary);
        assert_eq!(primary.wire_session_id(), None);

        let attached = DevToolsSessionKey::from_wire_session_id(Some("session-1"));
        assert_eq!(
            attached,
            DevToolsSessionKey::Attached("session-1".to_owned())
        );
        assert_eq!(attached.wire_session_id(), Some("session-1"));
    }

    #[test]
    fn renderer_agent_and_attachment_ids_are_nonzero_and_distinct() {
        let first_agent = RendererDevToolsAgentToken::allocate();
        let second_agent = RendererDevToolsAgentToken::allocate();
        assert_ne!(first_agent, second_agent);
        assert_ne!(first_agent.get(), 0);

        let first_attachment = RendererAgentAttachmentId::allocate();
        let second_attachment = RendererAgentAttachmentId::allocate();
        assert_ne!(first_attachment, second_attachment);
        assert_ne!(first_attachment.get(), 0);

        let first_command = RendererDevToolsCommandId::allocate();
        let second_command = RendererDevToolsCommandId::allocate();
        assert_ne!(first_command, second_command);
        assert_ne!(first_command.get(), 0);
    }

    #[test]
    fn internal_nonzero_ids_preserve_option_niche() {
        assert_eq!(
            size_of::<Option<RendererDevToolsAgentToken>>(),
            size_of::<RendererDevToolsAgentToken>()
        );
        assert_eq!(
            size_of::<Option<RendererAgentAttachmentId>>(),
            size_of::<RendererAgentAttachmentId>()
        );
        assert_eq!(
            size_of::<Option<RendererDevToolsCommandId>>(),
            size_of::<RendererDevToolsCommandId>()
        );
    }

    #[test]
    fn checked_allocator_rejects_exhaustion_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX);
        let exhausted =
            std::panic::catch_unwind(|| allocate_nonzero_u64(&counter, "test identity"));
        assert!(exhausted.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn frontend_command_zero_is_valid_and_renderer_range_is_checked() {
        assert_eq!(
            RendererCallId::try_from(FrontendCommandId::new(0)),
            Ok(RendererCallId::new(0))
        );
        assert_eq!(
            RendererCallId::try_from(FrontendCommandId::new(i32::MAX as u64)),
            Ok(RendererCallId::new(i32::MAX))
        );

        let out_of_range = FrontendCommandId::new(i32::MAX as u64 + 1);
        let error = RendererCallId::try_from(out_of_range).unwrap_err();
        assert_eq!(error.frontend_command_id(), out_of_range);
    }
}
