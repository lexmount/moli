use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context as _, Result, anyhow, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD as BASE64_STANDARD_NO_PAD};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    browsing_context_model::TopLevelWindowProxyEndpointId,
    native_bridge::ChildBrowsingContextNavigationRequest,
    script_vm::RendererRemoteFrameToken,
    structured_clone::{RemoteStructuredCloneWirePayload, V8StructuredClonePayload},
};

use super::{
    RendererResolvedPopupTarget, RendererTopLevelNavigationSource, RendererWindowDocumentSource,
};

static NEXT_REMOTE_WINDOW_PROXY_COMMAND_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_REMOTE_WINDOW_PROXY_CHANNEL_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_REMOTE_FRAME_NAVIGATION_ID: AtomicU64 = AtomicU64::new(1);

const REMOTE_WINDOW_PROXY_WIRE_VERSION: u16 = 2;
const MAX_REMOTE_WINDOW_PROXY_WIRE_BYTES: usize = 64 * 1024 * 1024;
const MAX_REMOTE_WINDOW_PROXY_URL_BYTES: usize = 2 * 1024 * 1024;
const MAX_REMOTE_WINDOW_PROXY_STRING_BYTES: usize = 16 * 1024;
const MAX_REMOTE_WINDOW_PROXY_HEADERS: usize = 256;

fn allocate_nonzero_id(allocator: &AtomicU64, label: &str) -> u64 {
    allocator
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .unwrap_or_else(|_| panic!("{label} allocator overflow"))
}

fn allocate_remote_window_proxy_command_id() -> u64 {
    allocate_nonzero_id(
        &NEXT_REMOTE_WINDOW_PROXY_COMMAND_ID,
        "RemoteWindowProxy command id",
    )
}

/// Browser-owner binding of one logical WindowProxy endpoint to its current
/// renderer execution channel.
///
/// The logical endpoint remains stable across a same-group agent transition,
/// while this generation rotates at the commit boundary. An operation that
/// was admitted against the outgoing agent can therefore never execute in the
/// replacement realm merely because the Page residence and endpoint survived.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererRemoteWindowProxyChannel {
    owner_local_host_id: crate::runtime::RendererOwnerLocalHostId,
    generation: u64,
}

impl RendererRemoteWindowProxyChannel {
    pub(crate) fn allocate(target: RendererResolvedPopupTarget) -> Self {
        Self {
            owner_local_host_id: target.owner_local_host_id(),
            generation: allocate_nonzero_id(
                &NEXT_REMOTE_WINDOW_PROXY_CHANNEL_GENERATION,
                "RemoteWindowProxy channel generation",
            ),
        }
    }

    pub(crate) const fn from_wire_parts(owner_local_host_id: u64, generation: u64) -> Option<Self> {
        let Some(owner_local_host_id) =
            crate::runtime::RendererOwnerLocalHostId::from_wire(owner_local_host_id)
        else {
            return None;
        };
        if generation == 0 {
            return None;
        }
        Some(Self {
            owner_local_host_id,
            generation,
        })
    }

    pub(crate) const fn owner_local_host_id(self) -> crate::runtime::RendererOwnerLocalHostId {
        self.owner_local_host_id
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererRemoteFrameNavigationId(u64);

impl RendererRemoteFrameNavigationId {
    pub(crate) fn allocate() -> Self {
        Self(allocate_nonzero_id(
            &NEXT_REMOTE_FRAME_NAVIGATION_ID,
            "remote-frame navigation id",
        ))
    }

    const fn from_wire(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    const fn value(self) -> u64 {
        self.0
    }
}

/// Exact source browsing context captured when a RemoteWindowProxy operation
/// is accepted. No V8 handle is transportable; the receiving script agent
/// materializes its own projection from this group-qualified endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererRemoteWindowProxySource {
    endpoint: TopLevelWindowProxyEndpointId,
    page: RendererResolvedPopupTarget,
    serialized_origin: String,
    frame: Option<RendererRemoteFrameToken>,
}

impl RendererRemoteWindowProxySource {
    pub(crate) fn new(
        endpoint: TopLevelWindowProxyEndpointId,
        page: RendererResolvedPopupTarget,
        serialized_origin: String,
    ) -> Self {
        Self {
            endpoint,
            page,
            serialized_origin,
            frame: None,
        }
    }

    pub(crate) fn with_frame(
        mut self,
        frame: RendererRemoteFrameToken,
        serialized_origin: String,
    ) -> Self {
        debug_assert_eq!(frame.endpoint, self.endpoint);
        self.frame = Some(frame);
        self.serialized_origin = serialized_origin;
        self
    }

    pub(crate) const fn endpoint(&self) -> TopLevelWindowProxyEndpointId {
        self.endpoint
    }

    pub(crate) const fn page(&self) -> RendererResolvedPopupTarget {
        self.page
    }

    pub(crate) fn serialized_origin(&self) -> &str {
        &self.serialized_origin
    }

    pub(crate) const fn frame(&self) -> Option<RendererRemoteFrameToken> {
        self.frame
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererRemoteWindowProxyNavigationKind {
    Assign,
    Replace,
}

/// Source JavaScript world whose policy admitted one remote `javascript:` URL.
///
/// The destination always executes the URL in its current main world. This
/// value preserves the source dispatch-world distinction across the remote
/// owner boundary. Moli does not currently expose isolated-world-specific CSP
/// state, so both variants consult the source Document policy; universal
/// Window access remains a separate `CanNavigate` fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererRemoteJavaScriptUrlSourceWorld {
    Main,
    Isolated { grants_universal_access: bool },
}

/// Exact source-side admission retained for a remote `javascript:` URL.
///
/// The ordinary navigation source carries Document/currentness and referrer
/// causality. The WindowProxy source adds the related-group endpoint and Page
/// route. Origin/domain/opaque identity are frozen independently so the target
/// owner can compare them with its *current* Document before scheduling script.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererRemoteJavaScriptUrlSource {
    navigation_source: RendererTopLevelNavigationSource,
    window_source: RendererRemoteWindowProxySource,
    opaque_origin_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    document_domain: Option<String>,
    world: RendererRemoteJavaScriptUrlSourceWorld,
}

impl RendererRemoteJavaScriptUrlSource {
    pub(crate) fn new(
        navigation_source: RendererTopLevelNavigationSource,
        window_source: RendererRemoteWindowProxySource,
        opaque_origin_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
        document_domain: Option<String>,
        world: RendererRemoteJavaScriptUrlSourceWorld,
    ) -> Self {
        assert_eq!(
            window_source.serialized_origin() == "null",
            opaque_origin_nonce.is_some(),
            "remote javascript URL source opaque identity must match its serialized origin"
        );
        assert!(
            window_source.serialized_origin() != "null" || document_domain.is_none(),
            "an opaque remote javascript URL source cannot carry document.domain"
        );
        Self {
            navigation_source,
            window_source,
            opaque_origin_nonce,
            document_domain,
            world,
        }
    }

    pub(crate) fn navigation_source(&self) -> &RendererTopLevelNavigationSource {
        &self.navigation_source
    }

    pub(crate) fn window_source(&self) -> &RendererRemoteWindowProxySource {
        &self.window_source
    }

    pub(crate) const fn opaque_origin_nonce(&self) -> Option<moli_storage_key::OpaqueOriginNonce> {
        self.opaque_origin_nonce
    }

    pub(crate) fn document_domain(&self) -> Option<&str> {
        self.document_domain.as_deref()
    }

    pub(crate) const fn world(&self) -> RendererRemoteJavaScriptUrlSourceWorld {
        self.world
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererRemoteWindowProxyMessage {
    pub(crate) source: RendererRemoteWindowProxySource,
    pub(crate) payload: V8StructuredClonePayload,
    pub(crate) intended_target_origin: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum RendererRemoteWindowProxyCommandKind {
    Navigate {
        kind: RendererRemoteWindowProxyNavigationKind,
        url: String,
        source: RendererTopLevelNavigationSource,
    },
    NavigateJavaScriptUrl {
        kind: RendererRemoteWindowProxyNavigationKind,
        url: String,
        source: Box<RendererRemoteJavaScriptUrlSource>,
    },
    NavigateFrame {
        kind: RendererRemoteWindowProxyNavigationKind,
        request: Box<ChildBrowsingContextNavigationRequest>,
        scheduler_id: Option<RendererRemoteFrameNavigationId>,
    },
    NavigateFrameJavaScriptUrl {
        kind: RendererRemoteWindowProxyNavigationKind,
        request: Box<ChildBrowsingContextNavigationRequest>,
        source: Box<RendererRemoteJavaScriptUrlSource>,
        scheduler_id: Option<RendererRemoteFrameNavigationId>,
    },
    CancelFrameNavigation {
        scheduler_id: RendererRemoteFrameNavigationId,
    },
    PostMessage(Box<RendererRemoteWindowProxyMessage>),
    Focus,
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RendererRemoteWindowProxyRoute {
    id: u64,
    target_endpoint: TopLevelWindowProxyEndpointId,
    target_page: RendererResolvedPopupTarget,
    target_channel: RendererRemoteWindowProxyChannel,
    target_frame: Option<RendererRemoteFrameToken>,
}

/// Versioned, validated renderer-to-renderer command.
///
/// The browser owner sees only the immutable route plus serialized bytes. It
/// cannot accidentally retain a source V8 handle, a target host pointer, a
/// compiled Wasm module, or a storage-service capability while the operation
/// waits for its destination renderer.
#[derive(Clone)]
pub struct RendererRemoteWindowProxyCommand {
    route: RendererRemoteWindowProxyRoute,
    wire_bytes: Arc<[u8]>,
}

impl fmt::Debug for RendererRemoteWindowProxyCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererRemoteWindowProxyCommand")
            .field("id", &self.route.id)
            .field("target_endpoint", &self.route.target_endpoint)
            .field("target_page", &self.route.target_page)
            .field("target_channel", &self.route.target_channel)
            .field("target_frame", &self.route.target_frame)
            .field("wire_bytes", &self.wire_bytes.len())
            .finish()
    }
}

impl PartialEq for RendererRemoteWindowProxyCommand {
    fn eq(&self, other: &Self) -> bool {
        self.route == other.route && self.wire_bytes == other.wire_bytes
    }
}

impl Eq for RendererRemoteWindowProxyCommand {}

impl RendererRemoteWindowProxyCommand {
    fn new(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        kind: RendererRemoteWindowProxyCommandKind,
    ) -> Result<Self> {
        Self::new_with_optional_frame(target_endpoint, target_page, target_channel, None, kind)
    }

    fn new_for_frame(
        target_frame: RendererRemoteFrameToken,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        kind: RendererRemoteWindowProxyCommandKind,
    ) -> Result<Self> {
        Self::new_with_optional_frame(
            target_frame.endpoint,
            target_page,
            target_channel,
            Some(target_frame),
            kind,
        )
    }

    fn new_with_optional_frame(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        target_frame: Option<RendererRemoteFrameToken>,
        kind: RendererRemoteWindowProxyCommandKind,
    ) -> Result<Self> {
        ensure!(
            target_channel.owner_local_host_id() == target_page.owner_local_host_id(),
            "RemoteWindowProxy channel must belong to the selected target owner"
        );
        let route = RendererRemoteWindowProxyRoute {
            id: allocate_remote_window_proxy_command_id(),
            target_endpoint,
            target_page,
            target_channel,
            target_frame,
        };
        let wire = RemoteWindowProxyCommandWire::from_command(route, kind)?;
        let wire_bytes =
            encode_remote_window_proxy_wire(&wire, MAX_REMOTE_WINDOW_PROXY_WIRE_BYTES)?;
        Ok(Self {
            route,
            wire_bytes: Arc::from(wire_bytes),
        })
    }

    pub(crate) fn navigate(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        kind: RendererRemoteWindowProxyNavigationKind,
        url: String,
        source: RendererTopLevelNavigationSource,
    ) -> Result<Self> {
        Self::new(
            target_endpoint,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::Navigate { kind, url, source },
        )
    }

    pub(crate) fn navigate_javascript_url(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        kind: RendererRemoteWindowProxyNavigationKind,
        url: String,
        source: RendererRemoteJavaScriptUrlSource,
    ) -> Result<Self> {
        Self::new(
            target_endpoint,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl {
                kind,
                url,
                source: Box::new(source),
            },
        )
    }

    pub(crate) fn post_message(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        source: RendererRemoteWindowProxySource,
        payload: V8StructuredClonePayload,
        intended_target_origin: Option<String>,
    ) -> Result<Self> {
        Self::new(
            target_endpoint,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::PostMessage(Box::new(
                RendererRemoteWindowProxyMessage {
                    source,
                    payload,
                    intended_target_origin,
                },
            )),
        )
    }

    pub(crate) fn navigate_frame(
        target_frame: RendererRemoteFrameToken,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        kind: RendererRemoteWindowProxyNavigationKind,
        request: ChildBrowsingContextNavigationRequest,
        scheduler_id: Option<RendererRemoteFrameNavigationId>,
    ) -> Result<Self> {
        Self::new_for_frame(
            target_frame,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::NavigateFrame {
                kind,
                request: Box::new(request),
                scheduler_id,
            },
        )
    }

    pub(crate) fn navigate_frame_javascript_url(
        target_frame: RendererRemoteFrameToken,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        kind: RendererRemoteWindowProxyNavigationKind,
        request: ChildBrowsingContextNavigationRequest,
        source: RendererRemoteJavaScriptUrlSource,
        scheduler_id: Option<RendererRemoteFrameNavigationId>,
    ) -> Result<Self> {
        Self::new_for_frame(
            target_frame,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::NavigateFrameJavaScriptUrl {
                kind,
                request: Box::new(request),
                source: Box::new(source),
                scheduler_id,
            },
        )
    }

    pub(crate) fn cancel_frame_navigation(
        target_frame: RendererRemoteFrameToken,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        scheduler_id: RendererRemoteFrameNavigationId,
    ) -> Result<Self> {
        Self::new_for_frame(
            target_frame,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::CancelFrameNavigation { scheduler_id },
        )
    }

    pub(crate) fn post_message_frame(
        target_frame: RendererRemoteFrameToken,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
        source: RendererRemoteWindowProxySource,
        payload: V8StructuredClonePayload,
        intended_target_origin: Option<String>,
    ) -> Result<Self> {
        Self::new_for_frame(
            target_frame,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::PostMessage(Box::new(
                RendererRemoteWindowProxyMessage {
                    source,
                    payload,
                    intended_target_origin,
                },
            )),
        )
    }

    pub(crate) fn focus(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
    ) -> Result<Self> {
        Self::new(
            target_endpoint,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::Focus,
        )
    }

    pub(crate) fn close(
        target_endpoint: TopLevelWindowProxyEndpointId,
        target_page: RendererResolvedPopupTarget,
        target_channel: RendererRemoteWindowProxyChannel,
    ) -> Result<Self> {
        Self::new(
            target_endpoint,
            target_page,
            target_channel,
            RendererRemoteWindowProxyCommandKind::Close,
        )
    }

    pub const fn target_page(&self) -> RendererResolvedPopupTarget {
        self.route.target_page
    }

    pub(crate) const fn target_endpoint(&self) -> TopLevelWindowProxyEndpointId {
        self.route.target_endpoint
    }

    pub(crate) const fn target_channel(&self) -> RendererRemoteWindowProxyChannel {
        self.route.target_channel
    }

    pub(crate) const fn target_frame(&self) -> Option<RendererRemoteFrameToken> {
        self.route.target_frame
    }

    #[cfg(test)]
    pub(crate) fn kind_for_testing(&self) -> RendererRemoteWindowProxyCommandKind {
        self.decode_kind()
            .expect("renderer-created RemoteWindowProxy wire should decode")
    }

    pub(crate) fn into_kind(self) -> Result<RendererRemoteWindowProxyCommandKind> {
        self.decode_kind()
    }

    fn decode_kind(&self) -> Result<RendererRemoteWindowProxyCommandKind> {
        ensure!(
            self.wire_bytes.len() <= MAX_REMOTE_WINDOW_PROXY_WIRE_BYTES,
            "RemoteWindowProxy wire command exceeds the transport byte limit"
        );
        let wire: RemoteWindowProxyCommandWire = serde_json::from_slice(&self.wire_bytes)
            .context("RemoteWindowProxy wire command is not valid schema JSON")?;
        let (route, kind) = wire.into_command()?;
        ensure!(
            route == self.route,
            "RemoteWindowProxy wire route disagrees with its validated routing header"
        );
        Ok(kind)
    }

    pub(crate) fn transport_charge_bytes(&self) -> usize {
        self.wire_bytes.len()
    }

    #[cfg(test)]
    fn from_wire_bytes_for_testing(wire_bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            wire_bytes.len() <= MAX_REMOTE_WINDOW_PROXY_WIRE_BYTES,
            "RemoteWindowProxy wire command exceeds the transport byte limit"
        );
        let wire: RemoteWindowProxyCommandWire = serde_json::from_slice(&wire_bytes)
            .context("RemoteWindowProxy wire command is not valid schema JSON")?;
        let (route, _) = wire.into_command()?;
        Ok(Self {
            route,
            wire_bytes: Arc::from(wire_bytes),
        })
    }
}

fn encode_remote_window_proxy_wire(
    wire: &RemoteWindowProxyCommandWire,
    byte_limit: usize,
) -> Result<Vec<u8>> {
    let wire_bytes = serde_json::to_vec(wire)
        .context("RemoteWindowProxy wire schema could not be serialized as JSON")?;
    ensure!(
        wire_bytes.len() <= byte_limit,
        "RemoteWindowProxy wire command exceeds the transport byte limit"
    );
    Ok(wire_bytes)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteWindowProxyCommandWire {
    version: u16,
    request_id: u64,
    target_endpoint: RemoteEndpointWire,
    target_page: RemotePageWire,
    target_channel: RemoteChannelWire,
    target_frame: Option<RemoteFrameTokenWire>,
    command: RemoteCommandKindWire,
}

impl RemoteWindowProxyCommandWire {
    fn from_command(
        route: RendererRemoteWindowProxyRoute,
        kind: RendererRemoteWindowProxyCommandKind,
    ) -> Result<Self> {
        validate_remote_command_route_and_kind(route, &kind)?;
        Ok(Self {
            version: REMOTE_WINDOW_PROXY_WIRE_VERSION,
            request_id: route.id,
            target_endpoint: route.target_endpoint.into(),
            target_page: route.target_page.into(),
            target_channel: route.target_channel.into(),
            target_frame: route.target_frame.map(Into::into),
            command: RemoteCommandKindWire::from_kind(kind)?,
        })
    }

    fn into_command(
        self,
    ) -> Result<(
        RendererRemoteWindowProxyRoute,
        RendererRemoteWindowProxyCommandKind,
    )> {
        ensure!(
            self.version == REMOTE_WINDOW_PROXY_WIRE_VERSION,
            "unsupported RemoteWindowProxy wire version {}",
            self.version
        );
        ensure!(self.request_id != 0, "RemoteWindowProxy request id is zero");
        let target_endpoint = self.target_endpoint.try_into()?;
        let target_page: RendererResolvedPopupTarget = self.target_page.try_into()?;
        let target_channel: RendererRemoteWindowProxyChannel = self.target_channel.try_into()?;
        ensure!(
            target_channel.owner_local_host_id() == target_page.owner_local_host_id(),
            "RemoteWindowProxy target channel belongs to a different renderer owner"
        );
        let target_frame: Option<RendererRemoteFrameToken> =
            self.target_frame.map(TryInto::try_into).transpose()?;
        if let Some(frame) = target_frame {
            ensure!(
                frame.endpoint == target_endpoint,
                "RemoteWindowProxy frame endpoint disagrees with its top-level route"
            );
            ensure!(
                frame.root_document.frame.page_id == target_page.page_id()
                    && frame.root_document.document.page_id == target_page.page_id(),
                "RemoteWindowProxy frame root Document belongs to a different Page"
            );
        }
        let kind = self.command.into_kind()?;
        let route = RendererRemoteWindowProxyRoute {
            id: self.request_id,
            target_endpoint,
            target_page,
            target_channel,
            target_frame,
        };
        validate_remote_command_route_and_kind(route, &kind)?;
        Ok((route, kind))
    }
}

fn validate_remote_command_route_and_kind(
    route: RendererRemoteWindowProxyRoute,
    kind: &RendererRemoteWindowProxyCommandKind,
) -> Result<()> {
    ensure!(
        matches!(
            (kind, route.target_frame),
            (
                RendererRemoteWindowProxyCommandKind::NavigateFrame { .. }
                    | RendererRemoteWindowProxyCommandKind::NavigateFrameJavaScriptUrl { .. }
                    | RendererRemoteWindowProxyCommandKind::CancelFrameNavigation { .. },
                Some(_)
            ) | (RendererRemoteWindowProxyCommandKind::PostMessage(_), _)
                | (
                    RendererRemoteWindowProxyCommandKind::Navigate { .. }
                        | RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl { .. }
                        | RendererRemoteWindowProxyCommandKind::Focus
                        | RendererRemoteWindowProxyCommandKind::Close,
                    None
                )
        ),
        "RemoteWindowProxy command kind is incompatible with its frame route"
    );
    match kind {
        RendererRemoteWindowProxyCommandKind::Navigate { url, .. } => {
            ensure!(
                parse_url_string(url, "target navigation URL")?.scheme() != "javascript",
                "ordinary remote navigation cannot carry a javascript URL"
            );
        }
        RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl { url, source, .. } => {
            ensure!(
                parse_url_string(url, "target javascript navigation URL")?.scheme() == "javascript",
                "typed remote javascript navigation requires a javascript URL"
            );
            validate_remote_javascript_url_source_route(source, route)?;
        }
        RendererRemoteWindowProxyCommandKind::NavigateFrame { request, .. } => {
            ensure!(
                request.url.scheme() != "javascript",
                "ordinary remote frame navigation cannot carry a javascript URL"
            );
        }
        RendererRemoteWindowProxyCommandKind::NavigateFrameJavaScriptUrl {
            request,
            source,
            ..
        } => {
            ensure!(
                request.url.scheme() == "javascript",
                "typed remote frame javascript navigation requires a javascript URL"
            );
            validate_remote_javascript_url_source_route(source, route)?;
        }
        RendererRemoteWindowProxyCommandKind::CancelFrameNavigation { .. }
        | RendererRemoteWindowProxyCommandKind::PostMessage(_)
        | RendererRemoteWindowProxyCommandKind::Focus
        | RendererRemoteWindowProxyCommandKind::Close => {}
    }
    Ok(())
}

fn validate_remote_javascript_url_source_route(
    source: &RendererRemoteJavaScriptUrlSource,
    route: RendererRemoteWindowProxyRoute,
) -> Result<()> {
    let window_source = source.window_source();
    ensure!(
        window_source.endpoint().browsing_context_group_id()
            == route.target_endpoint.browsing_context_group_id(),
        "remote javascript URL source and target belong to different browsing-context groups"
    );
    let root_document = source
        .navigation_source()
        .root_document()
        .ok_or_else(|| anyhow!("remote javascript URL has no Window Document source"))?;
    ensure!(
        root_document.frame.page_id == window_source.page().page_id()
            && root_document.document.page_id == window_source.page().page_id(),
        "remote javascript URL source Document belongs to another Page"
    );
    match (source.navigation_source().window(), window_source.frame()) {
        (Some(RendererWindowDocumentSource::RootFrame), None) => {}
        (Some(RendererWindowDocumentSource::ChildFrame { .. }), Some(frame)) => {
            ensure!(
                frame.endpoint == window_source.endpoint(),
                "remote javascript URL source frame belongs to another endpoint"
            );
            ensure!(
                frame.root_document == root_document,
                "remote javascript URL source frame belongs to another root Document"
            );
        }
        _ => {
            return Err(anyhow!(
                "remote javascript URL source Window kind disagrees with its frame route"
            ));
        }
    }
    ensure!(
        (window_source.serialized_origin() == "null") == source.opaque_origin_nonce().is_some(),
        "remote javascript URL source opaque identity disagrees with its serialized origin"
    );
    ensure!(
        window_source.serialized_origin() != "null" || source.document_domain().is_none(),
        "opaque remote javascript URL source cannot carry document.domain"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteEndpointWire {
    browsing_context_group_id: u64,
    generation: u64,
}

impl From<TopLevelWindowProxyEndpointId> for RemoteEndpointWire {
    fn from(endpoint: TopLevelWindowProxyEndpointId) -> Self {
        Self {
            browsing_context_group_id: endpoint.browsing_context_group_id().value(),
            generation: endpoint.generation(),
        }
    }
}

impl TryFrom<RemoteEndpointWire> for TopLevelWindowProxyEndpointId {
    type Error = anyhow::Error;

    fn try_from(endpoint: RemoteEndpointWire) -> Result<Self> {
        TopLevelWindowProxyEndpointId::from_wire_parts(
            endpoint.browsing_context_group_id,
            endpoint.generation,
        )
        .ok_or_else(|| anyhow!("RemoteWindowProxy endpoint contains a zero identity"))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemotePageWire {
    owner_local_host_id: u64,
    page_id: u64,
}

impl From<RendererResolvedPopupTarget> for RemotePageWire {
    fn from(page: RendererResolvedPopupTarget) -> Self {
        Self {
            owner_local_host_id: page.owner_local_host_id().as_u64(),
            page_id: page.page_id().as_u64(),
        }
    }
}

impl TryFrom<RemotePageWire> for RendererResolvedPopupTarget {
    type Error = anyhow::Error;

    fn try_from(page: RemotePageWire) -> Result<Self> {
        RendererResolvedPopupTarget::from_wire_parts(page.owner_local_host_id, page.page_id)
            .ok_or_else(|| anyhow!("RemoteWindowProxy Page route contains a zero identity"))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteChannelWire {
    owner_local_host_id: u64,
    generation: u64,
}

impl From<RendererRemoteWindowProxyChannel> for RemoteChannelWire {
    fn from(channel: RendererRemoteWindowProxyChannel) -> Self {
        Self {
            owner_local_host_id: channel.owner_local_host_id().as_u64(),
            generation: channel.generation(),
        }
    }
}

impl TryFrom<RemoteChannelWire> for RendererRemoteWindowProxyChannel {
    type Error = anyhow::Error;

    fn try_from(channel: RemoteChannelWire) -> Result<Self> {
        RendererRemoteWindowProxyChannel::from_wire_parts(
            channel.owner_local_host_id,
            channel.generation,
        )
        .ok_or_else(|| anyhow!("RemoteWindowProxy channel contains a zero identity"))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteDocumentLifecycleWire {
    frame_page_id: u64,
    document_page_id: u64,
    document_generation: u64,
    epoch: u64,
}

impl From<crate::runtime::RendererDocumentLifecycleIdentity> for RemoteDocumentLifecycleWire {
    fn from(identity: crate::runtime::RendererDocumentLifecycleIdentity) -> Self {
        Self {
            frame_page_id: identity.frame.page_id.as_u64(),
            document_page_id: identity.document.page_id.as_u64(),
            document_generation: identity.document.lifecycle_document_id_for_wire(),
            epoch: identity.epoch.0,
        }
    }
}

impl TryFrom<RemoteDocumentLifecycleWire> for crate::runtime::RendererDocumentLifecycleIdentity {
    type Error = anyhow::Error;

    fn try_from(identity: RemoteDocumentLifecycleWire) -> Result<Self> {
        let frame_page_id = crate::runtime::PageId::from_wire(identity.frame_page_id)
            .ok_or_else(|| anyhow!("remote lifecycle frame Page id is zero"))?;
        let document_page_id = crate::runtime::PageId::from_wire(identity.document_page_id)
            .ok_or_else(|| anyhow!("remote lifecycle Document Page id is zero"))?;
        ensure!(
            frame_page_id == document_page_id,
            "remote lifecycle frame and Document belong to different Pages"
        );
        ensure!(
            identity.document_generation != 0 && identity.epoch != 0,
            "remote lifecycle contains a zero generation"
        );
        Ok(Self {
            frame: crate::runtime::RendererFrameToken {
                page_id: frame_page_id,
            },
            document: crate::runtime::RendererDocumentToken::from_wire_parts(
                document_page_id,
                identity.document_generation,
            )
            .ok_or_else(|| anyhow!("remote lifecycle Document identity is zero"))?,
            epoch: crate::runtime::RendererLifecycleEpoch(identity.epoch),
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteFrameTokenWire {
    endpoint: RemoteEndpointWire,
    root_document: RemoteDocumentLifecycleWire,
    browsing_context_id: u64,
}

impl From<RendererRemoteFrameToken> for RemoteFrameTokenWire {
    fn from(frame: RendererRemoteFrameToken) -> Self {
        Self {
            endpoint: frame.endpoint.into(),
            root_document: frame.root_document.into(),
            browsing_context_id: frame.browsing_context_id.value(),
        }
    }
}

impl TryFrom<RemoteFrameTokenWire> for RendererRemoteFrameToken {
    type Error = anyhow::Error;

    fn try_from(frame: RemoteFrameTokenWire) -> Result<Self> {
        ensure!(
            frame.browsing_context_id != 0,
            "remote frame browsing-context id is zero"
        );
        Ok(Self {
            endpoint: frame.endpoint.try_into()?,
            root_document: frame.root_document.try_into()?,
            browsing_context_id: crate::browsing_context_model::BrowsingContextId::nested(
                frame.browsing_context_id,
            ),
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum RemoteNavigationKindWire {
    Assign,
    Replace,
}

impl From<RendererRemoteWindowProxyNavigationKind> for RemoteNavigationKindWire {
    fn from(kind: RendererRemoteWindowProxyNavigationKind) -> Self {
        match kind {
            RendererRemoteWindowProxyNavigationKind::Assign => Self::Assign,
            RendererRemoteWindowProxyNavigationKind::Replace => Self::Replace,
        }
    }
}

impl From<RemoteNavigationKindWire> for RendererRemoteWindowProxyNavigationKind {
    fn from(kind: RemoteNavigationKindWire) -> Self {
        match kind {
            RemoteNavigationKindWire::Assign => Self::Assign,
            RemoteNavigationKindWire::Replace => Self::Replace,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "kind")]
enum RemoteWindowDocumentSourceWire {
    RootFrame,
    ChildFrame {
        frame_id: String,
        local_window_id: u64,
        document_id: u64,
    },
}

impl From<RendererWindowDocumentSource> for RemoteWindowDocumentSourceWire {
    fn from(source: RendererWindowDocumentSource) -> Self {
        match source {
            RendererWindowDocumentSource::RootFrame => Self::RootFrame,
            RendererWindowDocumentSource::ChildFrame {
                frame_id,
                local_window_id,
                document_id,
            } => Self::ChildFrame {
                frame_id,
                local_window_id,
                document_id,
            },
        }
    }
}

impl TryFrom<RemoteWindowDocumentSourceWire> for RendererWindowDocumentSource {
    type Error = anyhow::Error;

    fn try_from(source: RemoteWindowDocumentSourceWire) -> Result<Self> {
        Ok(match source {
            RemoteWindowDocumentSourceWire::RootFrame => Self::RootFrame,
            RemoteWindowDocumentSourceWire::ChildFrame {
                frame_id,
                local_window_id,
                document_id,
            } => {
                validate_short_string(&frame_id, "source frame id")?;
                ensure!(
                    local_window_id != 0 && document_id != 0,
                    "remote navigation child source contains a zero identity"
                );
                Self::ChildFrame {
                    frame_id,
                    local_window_id,
                    document_id,
                }
            }
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "kind")]
enum RemoteTopLevelNavigationSourceCauseWire {
    Window {
        root_document: RemoteDocumentLifecycleWire,
        window: RemoteWindowDocumentSourceWire,
    },
    BrowserContext,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteTopLevelNavigationSourceWire {
    cause: RemoteTopLevelNavigationSourceCauseWire,
    source_url: String,
    referrer_policy: Option<String>,
    suppress_referrer: bool,
}

impl RemoteTopLevelNavigationSourceWire {
    fn from_source(source: RendererTopLevelNavigationSource) -> Self {
        let cause = match (source.root_document(), source.window()) {
            (Some(root_document), Some(window)) => {
                RemoteTopLevelNavigationSourceCauseWire::Window {
                    root_document: root_document.into(),
                    window: window.clone().into(),
                }
            }
            (None, None) if source.is_browser_context() => {
                RemoteTopLevelNavigationSourceCauseWire::BrowserContext
            }
            _ => unreachable!("top-level navigation source has inconsistent cause fields"),
        };
        Self {
            cause,
            source_url: source.source_url().to_owned(),
            referrer_policy: source.referrer_policy().map(str::to_owned),
            suppress_referrer: source.suppresses_referrer(),
        }
    }

    fn into_source(self) -> Result<RendererTopLevelNavigationSource> {
        validate_url_string(&self.source_url, "navigation source URL")?;
        if let Some(policy) = self.referrer_policy.as_deref() {
            validate_short_string(policy, "referrer policy")?;
        }
        Ok(match self.cause {
            RemoteTopLevelNavigationSourceCauseWire::Window {
                root_document,
                window,
            } => RendererTopLevelNavigationSource::new(
                root_document.try_into()?,
                window.try_into()?,
                self.source_url,
                self.referrer_policy,
                self.suppress_referrer,
            ),
            RemoteTopLevelNavigationSourceCauseWire::BrowserContext => {
                RendererTopLevelNavigationSource::browser_context(
                    self.source_url,
                    self.referrer_policy,
                    self.suppress_referrer,
                )
            }
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteChildNavigationRequestWire {
    url: String,
    method: String,
    body: Option<String>,
    request_headers: Vec<(String, String)>,
    initiator_url: Option<String>,
    document_referrer: Option<String>,
}

impl From<ChildBrowsingContextNavigationRequest> for RemoteChildNavigationRequestWire {
    fn from(request: ChildBrowsingContextNavigationRequest) -> Self {
        let initiator_url = request.initiator_url().map(ToString::to_string);
        let document_referrer = request.document_referrer().map(str::to_owned);
        Self {
            url: request.url.to_string(),
            method: request.method,
            body: request.body.map(|body| BASE64_STANDARD_NO_PAD.encode(body)),
            request_headers: request.request_headers,
            initiator_url,
            document_referrer,
        }
    }
}

impl TryFrom<RemoteChildNavigationRequestWire> for ChildBrowsingContextNavigationRequest {
    type Error = anyhow::Error;

    fn try_from(request: RemoteChildNavigationRequestWire) -> Result<Self> {
        let url = parse_url_string(&request.url, "child navigation URL")?;
        validate_short_string(&request.method, "child navigation method")?;
        ensure!(
            http::Method::from_bytes(request.method.as_bytes()).is_ok(),
            "remote child navigation method is invalid"
        );
        ensure!(
            request.request_headers.len() <= MAX_REMOTE_WINDOW_PROXY_HEADERS,
            "remote child navigation contains too many headers"
        );
        for (name, value) in &request.request_headers {
            validate_short_string(name, "child navigation header name")?;
            validate_short_string(value, "child navigation header value")?;
            ensure!(
                http::HeaderName::from_bytes(name.as_bytes()).is_ok()
                    && http::HeaderValue::from_str(value).is_ok(),
                "remote child navigation contains an invalid header"
            );
        }
        ensure!(
            request.initiator_url.is_some() == request.document_referrer.is_some(),
            "remote child navigation has a partial source carrier"
        );
        let body = request
            .body
            .map(|body| {
                BASE64_STANDARD_NO_PAD
                    .decode(body)
                    .map_err(|_| anyhow!("remote child navigation body is not valid base64"))
            })
            .transpose()?;
        let mut decoded = ChildBrowsingContextNavigationRequest::new(
            url,
            request.method,
            body,
            request.request_headers,
        );
        if let (Some(initiator_url), Some(document_referrer)) =
            (request.initiator_url, request.document_referrer)
        {
            validate_short_string(&document_referrer, "child document referrer")?;
            decoded = decoded.with_wire_navigation_source(
                parse_url_string(&initiator_url, "child navigation initiator URL")?,
                document_referrer,
            );
        }
        Ok(decoded)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteWindowProxySourceWire {
    endpoint: RemoteEndpointWire,
    page: RemotePageWire,
    serialized_origin: String,
    frame: Option<RemoteFrameTokenWire>,
}

impl From<RendererRemoteWindowProxySource> for RemoteWindowProxySourceWire {
    fn from(source: RendererRemoteWindowProxySource) -> Self {
        Self {
            endpoint: source.endpoint.into(),
            page: source.page.into(),
            serialized_origin: source.serialized_origin,
            frame: source.frame.map(Into::into),
        }
    }
}

impl TryFrom<RemoteWindowProxySourceWire> for RendererRemoteWindowProxySource {
    type Error = anyhow::Error;

    fn try_from(source: RemoteWindowProxySourceWire) -> Result<Self> {
        validate_serialized_origin(&source.serialized_origin, "message source origin")?;
        let endpoint = source.endpoint.try_into()?;
        let page: RendererResolvedPopupTarget = source.page.try_into()?;
        let frame: Option<RendererRemoteFrameToken> =
            source.frame.map(TryInto::try_into).transpose()?;
        if let Some(frame) = frame {
            ensure!(
                frame.endpoint == endpoint,
                "remote message source frame belongs to another endpoint"
            );
            ensure!(
                frame.root_document.frame.page_id == page.page_id()
                    && frame.root_document.document.page_id == page.page_id(),
                "remote message source frame belongs to another Page"
            );
        }
        Ok(RendererRemoteWindowProxySource {
            endpoint,
            page,
            serialized_origin: source.serialized_origin,
            frame,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "kind")]
enum RemoteJavaScriptUrlSourceWorldWire {
    Main,
    Isolated { grants_universal_access: bool },
}

impl From<RendererRemoteJavaScriptUrlSourceWorld> for RemoteJavaScriptUrlSourceWorldWire {
    fn from(world: RendererRemoteJavaScriptUrlSourceWorld) -> Self {
        match world {
            RendererRemoteJavaScriptUrlSourceWorld::Main => Self::Main,
            RendererRemoteJavaScriptUrlSourceWorld::Isolated {
                grants_universal_access,
            } => Self::Isolated {
                grants_universal_access,
            },
        }
    }
}

impl From<RemoteJavaScriptUrlSourceWorldWire> for RendererRemoteJavaScriptUrlSourceWorld {
    fn from(world: RemoteJavaScriptUrlSourceWorldWire) -> Self {
        match world {
            RemoteJavaScriptUrlSourceWorldWire::Main => Self::Main,
            RemoteJavaScriptUrlSourceWorldWire::Isolated {
                grants_universal_access,
            } => Self::Isolated {
                grants_universal_access,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoteJavaScriptUrlSourceWire {
    navigation_source: RemoteTopLevelNavigationSourceWire,
    window_source: RemoteWindowProxySourceWire,
    opaque_origin_nonce: Option<u64>,
    document_domain: Option<String>,
    world: RemoteJavaScriptUrlSourceWorldWire,
}

impl From<RendererRemoteJavaScriptUrlSource> for RemoteJavaScriptUrlSourceWire {
    fn from(source: RendererRemoteJavaScriptUrlSource) -> Self {
        Self {
            navigation_source: RemoteTopLevelNavigationSourceWire::from_source(
                source.navigation_source,
            ),
            window_source: source.window_source.into(),
            opaque_origin_nonce: source
                .opaque_origin_nonce
                .map(moli_storage_key::OpaqueOriginNonce::get),
            document_domain: source.document_domain,
            world: source.world.into(),
        }
    }
}

impl TryFrom<RemoteJavaScriptUrlSourceWire> for RendererRemoteJavaScriptUrlSource {
    type Error = anyhow::Error;

    fn try_from(source: RemoteJavaScriptUrlSourceWire) -> Result<Self> {
        let window_source: RendererRemoteWindowProxySource = source.window_source.try_into()?;
        let opaque_origin_nonce = source
            .opaque_origin_nonce
            .map(|nonce| {
                ensure!(
                    nonce != 0,
                    "remote javascript URL opaque-origin nonce is zero"
                );
                Ok(moli_storage_key::OpaqueOriginNonce::new(nonce))
            })
            .transpose()?;
        ensure!(
            (window_source.serialized_origin() == "null") == opaque_origin_nonce.is_some(),
            "remote javascript URL opaque-origin identity disagrees with its serialized origin"
        );
        ensure!(
            window_source.serialized_origin() != "null" || source.document_domain.is_none(),
            "opaque remote javascript URL source cannot carry document.domain"
        );
        if let Some(domain) = source.document_domain.as_deref() {
            validate_short_string(domain, "javascript URL source document.domain")?;
        }
        Ok(RendererRemoteJavaScriptUrlSource::new(
            source.navigation_source.into_source()?,
            window_source,
            opaque_origin_nonce,
            source.document_domain,
            source.world.into(),
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "kind")]
enum RemoteCommandKindWire {
    Navigate {
        navigation_kind: RemoteNavigationKindWire,
        url: String,
        source: RemoteTopLevelNavigationSourceWire,
    },
    NavigateJavaScriptUrl {
        navigation_kind: RemoteNavigationKindWire,
        url: String,
        source: Box<RemoteJavaScriptUrlSourceWire>,
    },
    NavigateFrame {
        navigation_kind: RemoteNavigationKindWire,
        request: Box<RemoteChildNavigationRequestWire>,
        scheduler_id: Option<u64>,
    },
    NavigateFrameJavaScriptUrl {
        navigation_kind: RemoteNavigationKindWire,
        request: Box<RemoteChildNavigationRequestWire>,
        source: Box<RemoteJavaScriptUrlSourceWire>,
        scheduler_id: Option<u64>,
    },
    CancelFrameNavigation {
        scheduler_id: u64,
    },
    PostMessage {
        source: RemoteWindowProxySourceWire,
        payload: RemoteStructuredCloneWirePayload,
        intended_target_origin: Option<String>,
    },
    Focus,
    Close,
}

impl RemoteCommandKindWire {
    fn from_kind(kind: RendererRemoteWindowProxyCommandKind) -> Result<Self> {
        Ok(match kind {
            RendererRemoteWindowProxyCommandKind::Navigate { kind, url, source } => {
                Self::Navigate {
                    navigation_kind: kind.into(),
                    url,
                    source: RemoteTopLevelNavigationSourceWire::from_source(source),
                }
            }
            RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl { kind, url, source } => {
                Self::NavigateJavaScriptUrl {
                    navigation_kind: kind.into(),
                    url,
                    source: Box::new((*source).into()),
                }
            }
            RendererRemoteWindowProxyCommandKind::NavigateFrame {
                kind,
                request,
                scheduler_id,
            } => Self::NavigateFrame {
                navigation_kind: kind.into(),
                request: Box::new((*request).into()),
                scheduler_id: scheduler_id.map(RendererRemoteFrameNavigationId::value),
            },
            RendererRemoteWindowProxyCommandKind::NavigateFrameJavaScriptUrl {
                kind,
                request,
                source,
                scheduler_id,
            } => Self::NavigateFrameJavaScriptUrl {
                navigation_kind: kind.into(),
                request: Box::new((*request).into()),
                source: Box::new((*source).into()),
                scheduler_id: scheduler_id.map(RendererRemoteFrameNavigationId::value),
            },
            RendererRemoteWindowProxyCommandKind::CancelFrameNavigation { scheduler_id } => {
                Self::CancelFrameNavigation {
                    scheduler_id: scheduler_id.value(),
                }
            }
            RendererRemoteWindowProxyCommandKind::PostMessage(message) => Self::PostMessage {
                source: message.source.into(),
                payload: message.payload.into_remote_wire()?,
                intended_target_origin: message.intended_target_origin,
            },
            RendererRemoteWindowProxyCommandKind::Focus => Self::Focus,
            RendererRemoteWindowProxyCommandKind::Close => Self::Close,
        })
    }

    fn into_kind(self) -> Result<RendererRemoteWindowProxyCommandKind> {
        Ok(match self {
            Self::Navigate {
                navigation_kind,
                url,
                source,
            } => {
                validate_url_string(&url, "target navigation URL")?;
                RendererRemoteWindowProxyCommandKind::Navigate {
                    kind: navigation_kind.into(),
                    url,
                    source: source.into_source()?,
                }
            }
            Self::NavigateJavaScriptUrl {
                navigation_kind,
                url,
                source,
            } => {
                validate_url_string(&url, "target javascript navigation URL")?;
                RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl {
                    kind: navigation_kind.into(),
                    url,
                    source: Box::new((*source).try_into()?),
                }
            }
            Self::NavigateFrame {
                navigation_kind,
                request,
                scheduler_id,
            } => RendererRemoteWindowProxyCommandKind::NavigateFrame {
                kind: navigation_kind.into(),
                request: Box::new((*request).try_into()?),
                scheduler_id: scheduler_id
                    .map(|id| {
                        RendererRemoteFrameNavigationId::from_wire(id)
                            .ok_or_else(|| anyhow!("remote frame navigation scheduler id is zero"))
                    })
                    .transpose()?,
            },
            Self::NavigateFrameJavaScriptUrl {
                navigation_kind,
                request,
                source,
                scheduler_id,
            } => RendererRemoteWindowProxyCommandKind::NavigateFrameJavaScriptUrl {
                kind: navigation_kind.into(),
                request: Box::new((*request).try_into()?),
                source: Box::new((*source).try_into()?),
                scheduler_id: scheduler_id
                    .map(|id| {
                        RendererRemoteFrameNavigationId::from_wire(id).ok_or_else(|| {
                            anyhow!("remote frame javascript navigation scheduler id is zero")
                        })
                    })
                    .transpose()?,
            },
            Self::CancelFrameNavigation { scheduler_id } => {
                RendererRemoteWindowProxyCommandKind::CancelFrameNavigation {
                    scheduler_id: RendererRemoteFrameNavigationId::from_wire(scheduler_id)
                        .ok_or_else(|| {
                            anyhow!("remote frame navigation cancellation id is zero")
                        })?,
                }
            }
            Self::PostMessage {
                source,
                payload,
                intended_target_origin,
            } => {
                if let Some(origin) = intended_target_origin.as_deref() {
                    validate_serialized_origin(origin, "intended target origin")?;
                }
                let source: RendererRemoteWindowProxySource = source.try_into()?;
                let payload = V8StructuredClonePayload::from_remote_wire(payload)?;
                if payload.metadata.contains_wasm_module {
                    ensure!(
                        payload.metadata.sender_origin.as_deref()
                            == Some(source.serialized_origin()),
                        "remote Wasm message source origin disagrees with its clone metadata"
                    );
                    ensure!(
                        payload.metadata.sender_agent_cluster
                            == Some(
                                crate::structured_clone::RuntimeMessageAgentCluster::WindowOrDedicatedWorker
                            ),
                        "remote Window Wasm message claims another agent-cluster kind"
                    );
                }
                RendererRemoteWindowProxyCommandKind::PostMessage(Box::new(
                    RendererRemoteWindowProxyMessage {
                        source,
                        payload,
                        intended_target_origin,
                    },
                ))
            }
            Self::Focus => RendererRemoteWindowProxyCommandKind::Focus,
            Self::Close => RendererRemoteWindowProxyCommandKind::Close,
        })
    }
}

fn validate_short_string(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() <= MAX_REMOTE_WINDOW_PROXY_STRING_BYTES && !value.contains('\0'),
        "RemoteWindowProxy {label} is invalid"
    );
    Ok(())
}

fn validate_serialized_origin(value: &str, label: &str) -> Result<()> {
    validate_short_string(value, label)?;
    if value == "null" {
        return Ok(());
    }
    let url = Url::parse(value)
        .with_context(|| format!("RemoteWindowProxy {label} is not a serialized origin"))?;
    ensure!(
        moli_url::origin_ascii_serialization(&url) == value,
        "RemoteWindowProxy {label} is not a canonical serialized origin"
    );
    Ok(())
}

fn validate_url_string(value: &str, label: &str) -> Result<()> {
    let _ = parse_url_string(value, label)?;
    Ok(())
}

fn parse_url_string(value: &str, label: &str) -> Result<Url> {
    ensure!(
        value.len() <= MAX_REMOTE_WINDOW_PROXY_URL_BYTES && !value.contains('\0'),
        "RemoteWindowProxy {label} exceeds the URL limit"
    );
    Url::parse(value).with_context(|| format!("RemoteWindowProxy {label} is not an absolute URL"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_endpoint() -> TopLevelWindowProxyEndpointId {
        TopLevelWindowProxyEndpointId::from_wire_parts(7, 9).expect("test endpoint")
    }

    fn test_page() -> RendererResolvedPopupTarget {
        RendererResolvedPopupTarget::from_wire_parts(11, 13).expect("test Page")
    }

    fn test_source_page() -> RendererResolvedPopupTarget {
        RendererResolvedPopupTarget::from_wire_parts(17, 19).expect("test source Page")
    }

    fn test_document(
        page: RendererResolvedPopupTarget,
    ) -> crate::runtime::RendererDocumentLifecycleIdentity {
        let page_id = page.page_id();
        crate::runtime::RendererDocumentLifecycleIdentity {
            frame: crate::runtime::RendererFrameToken { page_id },
            document: crate::runtime::RendererDocumentToken::new_for_testing(page_id, 23),
            epoch: crate::runtime::RendererLifecycleEpoch(29),
        }
    }

    fn test_javascript_source(
        world: RendererRemoteJavaScriptUrlSourceWorld,
    ) -> RendererRemoteJavaScriptUrlSource {
        let page = test_source_page();
        let endpoint =
            TopLevelWindowProxyEndpointId::from_wire_parts(7, 31).expect("test source endpoint");
        RendererRemoteJavaScriptUrlSource::new(
            RendererTopLevelNavigationSource::new(
                test_document(page),
                RendererWindowDocumentSource::RootFrame,
                "https://source.test/document".to_owned(),
                Some("strict-origin-when-cross-origin".to_owned()),
                false,
            ),
            RendererRemoteWindowProxySource::new(endpoint, page, "https://source.test".to_owned()),
            None,
            Some("source.test".to_owned()),
            world,
        )
    }

    #[test]
    fn remote_window_proxy_wire_rejects_unknown_versions_and_fields() {
        let page = test_page();
        let command = RendererRemoteWindowProxyCommand::focus(
            test_endpoint(),
            page,
            RendererRemoteWindowProxyChannel::allocate(page),
        )
        .expect("test focus command should encode");
        let mut value: serde_json::Value =
            serde_json::from_slice(&command.wire_bytes).expect("wire JSON");
        value["version"] = serde_json::json!(REMOTE_WINDOW_PROXY_WIRE_VERSION + 1);
        let unsupported = serde_json::to_vec(&value).expect("mutated wire JSON");
        assert!(
            RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(unsupported).is_err()
        );

        value["version"] = serde_json::json!(REMOTE_WINDOW_PROXY_WIRE_VERSION);
        value["unexpectedCapability"] = serde_json::json!(true);
        let unknown = serde_json::to_vec(&value).expect("mutated wire JSON");
        assert!(RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(unknown).is_err());

        let mut mismatched_route = command.clone();
        let mut value: serde_json::Value =
            serde_json::from_slice(&mismatched_route.wire_bytes).expect("wire JSON");
        value["targetPage"]["pageId"] = serde_json::json!(17);
        mismatched_route.wire_bytes =
            Arc::from(serde_json::to_vec(&value).expect("mutated route wire JSON"));
        assert!(
            mismatched_route.decode_kind().is_err(),
            "a valid wire body must not replace the browser-validated route header"
        );

        let source = RendererRemoteWindowProxySource::new(
            test_endpoint(),
            page,
            "https://source.test".to_owned(),
        );
        let message = RendererRemoteWindowProxyCommand::post_message(
            test_endpoint(),
            page,
            RendererRemoteWindowProxyChannel::allocate(page),
            source,
            V8StructuredClonePayload::default(),
            None,
        )
        .expect("test message command should encode");
        let mut value: serde_json::Value =
            serde_json::from_slice(&message.wire_bytes).expect("message wire JSON");
        value["command"]["source"]["serializedOrigin"] =
            serde_json::json!("https://source.test/not-an-origin");
        let forged_origin = serde_json::to_vec(&value).expect("mutated origin wire JSON");
        assert!(
            RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(forged_origin).is_err()
        );
    }

    #[test]
    fn remote_window_proxy_wire_round_trips_process_neutral_clone_attachments() {
        let page = test_page();
        let mut payload = V8StructuredClonePayload::default();
        payload.base.wire_bytes = vec![0xff, 0, 3, 7];
        payload.base.transferred_array_buffers.push(
            crate::structured_clone::TransferredArrayBuffer {
                transfer_id: 1,
                bytes: vec![1, 2, 3, 4],
            },
        );
        payload.base.transferred_message_ports.push(41);
        let source = RendererRemoteWindowProxySource::new(
            test_endpoint(),
            page,
            "https://source.test".to_owned(),
        );
        let command = RendererRemoteWindowProxyCommand::post_message(
            test_endpoint(),
            page,
            RendererRemoteWindowProxyChannel::allocate(page),
            source,
            payload,
            Some("https://target.test".to_owned()),
        )
        .expect("test attachment message should encode");
        let decoded = command.kind_for_testing();
        let RendererRemoteWindowProxyCommandKind::PostMessage(message) = decoded else {
            panic!("expected postMessage wire command")
        };
        assert_eq!(message.payload.base.wire_bytes, vec![0xff, 0, 3, 7]);
        assert_eq!(
            message.payload.base.transferred_array_buffers[0].bytes,
            vec![1, 2, 3, 4]
        );
        assert_eq!(message.payload.base.transferred_message_ports, vec![41]);

        let mut value: serde_json::Value =
            serde_json::from_slice(&command.wire_bytes).expect("message wire JSON");
        value["command"]["payload"]["transferredMessagePorts"] = serde_json::json!([41, 41]);
        let duplicate_port = serde_json::to_vec(&value).expect("mutated port wire JSON");
        assert!(
            RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(duplicate_port).is_err()
        );
    }

    #[test]
    fn remote_javascript_navigation_wire_is_typed_and_route_qualified() {
        let target_page = test_page();
        let target_channel = RendererRemoteWindowProxyChannel::allocate(target_page);
        let top = RendererRemoteWindowProxyCommand::navigate_javascript_url(
            test_endpoint(),
            target_page,
            target_channel,
            RendererRemoteWindowProxyNavigationKind::Replace,
            "javascript:void(globalThis.remoteTop = true)".to_owned(),
            test_javascript_source(RendererRemoteJavaScriptUrlSourceWorld::Isolated {
                grants_universal_access: false,
            }),
        )
        .expect("test javascript navigation should encode");
        let RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl { kind, url, source } =
            top.kind_for_testing()
        else {
            panic!("typed top-level javascript URL must round-trip its command kind")
        };
        assert_eq!(kind, RendererRemoteWindowProxyNavigationKind::Replace);
        assert_eq!(url, "javascript:void(globalThis.remoteTop = true)");
        assert_eq!(
            source.world(),
            RendererRemoteJavaScriptUrlSourceWorld::Isolated {
                grants_universal_access: false
            }
        );
        assert_eq!(source.document_domain(), Some("source.test"));

        let opaque_nonce = moli_storage_key::OpaqueOriginNonce::new(33);
        let opaque = RendererRemoteWindowProxyCommand::navigate_javascript_url(
            test_endpoint(),
            target_page,
            target_channel,
            RendererRemoteWindowProxyNavigationKind::Assign,
            "javascript:void(globalThis.remoteOpaque = true)".to_owned(),
            RendererRemoteJavaScriptUrlSource::new(
                RendererTopLevelNavigationSource::new(
                    test_document(test_source_page()),
                    RendererWindowDocumentSource::RootFrame,
                    "about:blank".to_owned(),
                    None,
                    false,
                ),
                RendererRemoteWindowProxySource::new(
                    TopLevelWindowProxyEndpointId::from_wire_parts(7, 31)
                        .expect("test source endpoint"),
                    test_source_page(),
                    "null".to_owned(),
                ),
                Some(opaque_nonce),
                None,
                RendererRemoteJavaScriptUrlSourceWorld::Main,
            ),
        )
        .expect("test opaque javascript navigation should encode");
        assert!(matches!(
            opaque.kind_for_testing(),
            RendererRemoteWindowProxyCommandKind::NavigateJavaScriptUrl { source, .. }
                if source.opaque_origin_nonce() == Some(opaque_nonce)
                    && source.document_domain().is_none()
        ));

        let target_frame = RendererRemoteFrameToken {
            endpoint: test_endpoint(),
            root_document: test_document(target_page),
            browsing_context_id: crate::browsing_context_model::BrowsingContextId::nested(37),
        };
        let frame = RendererRemoteWindowProxyCommand::navigate_frame_javascript_url(
            target_frame,
            target_page,
            target_channel,
            RendererRemoteWindowProxyNavigationKind::Assign,
            ChildBrowsingContextNavigationRequest::new(
                Url::parse("javascript:void(globalThis.remoteFrame = true)")
                    .expect("test javascript URL"),
                "GET".to_owned(),
                None,
                Vec::new(),
            ),
            test_javascript_source(RendererRemoteJavaScriptUrlSourceWorld::Main),
            Some(RendererRemoteFrameNavigationId::allocate()),
        )
        .expect("test frame javascript navigation should encode");
        assert_eq!(frame.target_frame(), Some(target_frame));
        assert!(matches!(
            frame.kind_for_testing(),
            RendererRemoteWindowProxyCommandKind::NavigateFrameJavaScriptUrl {
                source,
                scheduler_id: Some(_),
                ..
            } if source.world() == RendererRemoteJavaScriptUrlSourceWorld::Main
        ));
    }

    #[test]
    fn remote_javascript_navigation_wire_rejects_generic_and_forged_sources() {
        let target_page = test_page();
        let channel = RendererRemoteWindowProxyChannel::allocate(target_page);
        let ordinary = RendererRemoteWindowProxyCommand::navigate(
            test_endpoint(),
            target_page,
            channel,
            RendererRemoteWindowProxyNavigationKind::Assign,
            "https://target.test/ordinary".to_owned(),
            RendererTopLevelNavigationSource::browser_context(
                "https://source.test/worker".to_owned(),
                None,
                false,
            ),
        )
        .expect("test ordinary navigation should encode");
        let mut ordinary_value: serde_json::Value =
            serde_json::from_slice(&ordinary.wire_bytes).expect("ordinary wire JSON");
        ordinary_value["command"]["url"] =
            serde_json::json!("javascript:void(globalThis.forged = true)");
        assert!(
            RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(
                serde_json::to_vec(&ordinary_value).expect("mutated ordinary wire JSON")
            )
            .is_err(),
            "the generic navigation variant must not smuggle a javascript URL"
        );

        let typed = RendererRemoteWindowProxyCommand::navigate_javascript_url(
            test_endpoint(),
            target_page,
            channel,
            RendererRemoteWindowProxyNavigationKind::Assign,
            "javascript:void(0)".to_owned(),
            test_javascript_source(RendererRemoteJavaScriptUrlSourceWorld::Main),
        )
        .expect("test typed javascript navigation should encode");
        let mut group_value: serde_json::Value =
            serde_json::from_slice(&typed.wire_bytes).expect("typed wire JSON");
        group_value["command"]["source"]["windowSource"]["endpoint"]["browsingContextGroupId"] =
            serde_json::json!(41);
        assert!(
            RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(
                serde_json::to_vec(&group_value).expect("mutated source-group wire JSON")
            )
            .is_err(),
            "a javascript URL source from another browsing-context group must be rejected"
        );

        let mut opaque_value: serde_json::Value =
            serde_json::from_slice(&typed.wire_bytes).expect("typed wire JSON");
        opaque_value["command"]["source"]["opaqueOriginNonce"] = serde_json::json!(43);
        assert!(
            RendererRemoteWindowProxyCommand::from_wire_bytes_for_testing(
                serde_json::to_vec(&opaque_value).expect("mutated opaque-source wire JSON")
            )
            .is_err(),
            "a tuple source must not acquire a forged opaque-origin nonce"
        );
    }

    #[test]
    fn remote_child_navigation_wire_rejects_oversized_header_fields() {
        let wire = RemoteChildNavigationRequestWire {
            url: "https://target.test/path".to_owned(),
            method: "POST".to_owned(),
            body: None,
            request_headers: vec![(
                "X-Test".to_owned(),
                "x".repeat(MAX_REMOTE_WINDOW_PROXY_STRING_BYTES + 1),
            )],
            initiator_url: None,
            document_referrer: None,
        };
        assert!(
            ChildBrowsingContextNavigationRequest::try_from(wire).is_err(),
            "one oversized header must not bypass the per-field wire limit"
        );
    }

    #[test]
    fn remote_window_proxy_outbound_author_data_is_rejected_without_panicking() {
        let page = test_page();
        let oversized_url = format!(
            "https://target.test/{}",
            "x".repeat(MAX_REMOTE_WINDOW_PROXY_URL_BYTES)
        );
        let error = RendererRemoteWindowProxyCommand::navigate(
            test_endpoint(),
            page,
            RendererRemoteWindowProxyChannel::allocate(page),
            RendererRemoteWindowProxyNavigationKind::Assign,
            oversized_url,
            RendererTopLevelNavigationSource::browser_context(
                "https://source.test/document".to_owned(),
                None,
                false,
            ),
        )
        .expect_err("an author-controlled oversized URL must be rejected");
        assert!(error.to_string().contains("URL limit"));

        let route = RendererRemoteWindowProxyRoute {
            id: allocate_remote_window_proxy_command_id(),
            target_endpoint: test_endpoint(),
            target_page: page,
            target_channel: RendererRemoteWindowProxyChannel::allocate(page),
            target_frame: None,
        };
        let mut payload = V8StructuredClonePayload::default();
        payload.base.wire_bytes = vec![7; 256];
        let wire = RemoteWindowProxyCommandWire::from_command(
            route,
            RendererRemoteWindowProxyCommandKind::PostMessage(Box::new(
                RendererRemoteWindowProxyMessage {
                    source: RendererRemoteWindowProxySource::new(
                        test_endpoint(),
                        page,
                        "https://source.test".to_owned(),
                    ),
                    payload,
                    intended_target_origin: None,
                },
            )),
        )
        .expect("bounded test payload should form a wire command");
        let encoded = serde_json::to_vec(&wire).expect("test wire should encode");
        let error = encode_remote_window_proxy_wire(&wire, encoded.len() - 1)
            .expect_err("an encoded payload beyond the configured cap must be rejected");
        assert!(error.to_string().contains("transport byte limit"));
        assert_eq!(
            encode_remote_window_proxy_wire(&wire, encoded.len())
                .expect("the exact byte limit should admit the payload"),
            encoded
        );
    }
}
