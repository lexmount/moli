use std::{
    future::IntoFuture,
    net::SocketAddr,
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use anyhow::Result;
use anyhow::{Context, anyhow};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Request, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{MethodRouter, any, get, post, put},
    serve::ListenerExt,
};
use moli_cookie_jar::StoredCookie;
use moli_core::{
    LayoutConfiguration, LayoutPolicy, OptionalResourceFetchMask,
    runtime::{NavigationRuntimeConfig, storage_partition::StoragePartitionState},
};
use moli_fetch::FetchConfig;
#[cfg(test)]
use moli_protocol::DEFAULT_CDP_TAB_TARGET_ID;
use moli_protocol::{CdpInitialStoragePartition, DEFAULT_CDP_PAGE_TARGET_ID};
use tokio::net::TcpListener;
use tower::Layer;
use tracing::info;

pub use crate::config::ServerConfig;
pub use instance_control::{BrowserInstanceControl, InstanceCommand};

mod cdp;
mod cdp_agent_host;
mod cdp_owner;
#[cfg(unix)]
mod cdp_pipe;
mod cdp_socket;
mod instance_control;
mod instance_shutdown;
mod protocol_local_executor;
mod tcp_options;
mod trailing_slash;
mod webdriver_bidi;
mod webdriver_classic;
mod webdriver_files;

use cdp_agent_host::SharedCdpAgentHostDirectory;
use cdp_owner::SharedCdpOwnerRegistry;
use webdriver_bidi::SharedBidiSessionRegistry;
use webdriver_classic::SharedClassicSessionRegistry;

const DEFAULT_BROWSER_ID: &str = "moli-browser";
const DEFAULT_TARGET_ID: &str = DEFAULT_CDP_PAGE_TARGET_ID;
#[cfg(test)]
const DEFAULT_TAB_TARGET_ID: &str = DEFAULT_CDP_TAB_TARGET_ID;
const DEFAULT_TARGET_URL: &str = "about:blank";
const JSON_DISCOVERY_PATH: &str = "/json";

#[derive(Debug, Clone)]
pub struct ProtocolServer {
    config: ServerConfig,
    storage_partition: Arc<StoragePartitionState>,
    navigation_runtime_config: NavigationRuntimeConfig,
}

impl ProtocolServer {
    pub fn new(config: ServerConfig) -> Self {
        Self::new_with_initial_cookies(config, Vec::new())
    }

    pub fn new_with_initial_cookies(
        config: ServerConfig,
        initial_cookies: Vec<StoredCookie>,
    ) -> Self {
        let storage_partition = Arc::new(
            StoragePartitionState::open(None).expect("temporary storage partition should open"),
        );
        storage_partition
            .import_cookies(initial_cookies)
            .expect("initial cookie import should succeed");
        Self::new_with_storage_partition(config, storage_partition)
    }

    pub fn new_with_storage_partition(
        config: ServerConfig,
        storage_partition: Arc<StoragePartitionState>,
    ) -> Self {
        Self::new_with_storage_partition_fetch_config_and_resource_loading(
            config,
            storage_partition,
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
            true,
        )
    }

    pub fn new_with_storage_partition_fetch_config_and_resource_loading(
        config: ServerConfig,
        storage_partition: Arc<StoragePartitionState>,
        fetch_config: FetchConfig,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
    ) -> Self {
        Self::new_with_storage_partition_and_runtime_config(
            config,
            storage_partition,
            NavigationRuntimeConfig::new(
                fetch_config,
                optional_resource_fetch_mask,
                subframe_loading_enabled,
                LayoutPolicy::default(),
            ),
        )
    }

    pub fn new_with_storage_partition_and_runtime_config(
        config: ServerConfig,
        storage_partition: Arc<StoragePartitionState>,
        mut navigation_runtime_config: NavigationRuntimeConfig,
    ) -> Self {
        if navigation_runtime_config
            .fetch_config()
            .http_cache_dir()
            .is_none()
            && let Some(http_cache_root) = storage_partition.http_cache_root()
        {
            navigation_runtime_config
                .fetch_config_mut()
                .set_http_cache_dir(Some(http_cache_root.display().to_string()));
        }
        Self {
            config,
            storage_partition,
            navigation_runtime_config,
        }
    }

    pub async fn serve(&self) -> Result<()> {
        self.serve_with_control(BrowserInstanceControl::new()).await
    }

    /// Serve one instance with a protocol-independent lifecycle control handle.
    /// Use a fresh handle for each instance; shutdown requests are persistent.
    pub async fn serve_with_control(&self, control: BrowserInstanceControl) -> Result<()> {
        let listener = TcpListener::bind(self.config.bind_target())
            .await
            .with_context(|| {
                anyhow!(
                    "failed to bind protocol server to {}:{}",
                    self.config.host,
                    self.config.port
                )
            })?;
        let addr = listener
            .local_addr()
            .context("failed to read bound protocol socket address")?;

        info!(%addr, "protocol server listening");

        let app_state = AppState::new_with_storage_partition_and_runtime_config(
            addr,
            self.storage_partition.clone(),
            self.navigation_runtime_config.clone(),
            self.config.screencast_interval_ms,
            control,
        )?;

        serve_listener(listener, app_state).await
    }
}

/// Upper bound on draining and flushing one instance after it is asked to stop.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// Shared by the executable and instance-lifecycle integration tests.
async fn serve_listener(listener: TcpListener, app_state: AppState) -> Result<()> {
    let control = app_state.control.clone();
    let cdp_owner_registry = app_state.cdp_owner_registry.clone();
    let classic_session_registry = app_state.classic_session_registry.clone();
    let storage_partition = app_state.storage_partition.clone();
    let listener =
        listener.tap_io(|stream| tcp_options::configure_accepted_protocol_stream(stream));
    let stop_accepting = control.clone();
    let service = axum::ServiceExt::<Request<Body>>::into_make_service(build_router(app_state));
    let serving = axum::serve(listener, service)
        .with_graceful_shutdown(async move { stop_accepting.shutdown_requested().await })
        .into_future();
    tokio::pin!(serving);
    let completed = tokio::select! {
        result = &mut serving => Some(result),
        _ = control.shutdown_requested() => None,
    };
    control.request(InstanceCommand::Shutdown);
    tokio::time::timeout(SHUTDOWN_BUDGET, async {
        let (_, result) = tokio::join!(cdp_owner_registry.shutdown(), async {
            // Drain in-flight HTTP requests before taking the final Classic
            // session set. New requests and upgraded sockets are rejected.
            match completed {
                Some(result) => result,
                None => (&mut serving).await,
            }
            .context("protocol server failed")?;
            classic_session_registry.shutdown().await?;
            control.drained().await;
            tokio::task::spawn_blocking(move || storage_partition.shutdown())
                .await
                .context("storage shutdown worker stopped")??;
            Ok(())
        });
        result
    })
    .await
    .context("protocol server shutdown timed out; storage may be incomplete")?
}

// Matchit accepts an empty final parameter when a literal slash follows it.
// Keep that legacy match without duplicating endpoint declarations.
trait ProtocolRoutes {
    fn protocol_route(self, path: &str, route: MethodRouter<AppState>) -> Self;
}

impl ProtocolRoutes for Router<AppState> {
    #[inline(never)]
    fn protocol_route(self, path: &str, route: MethodRouter<AppState>) -> Self {
        if path.ends_with('}') {
            self.route(path, route.clone())
                .route(&format!("{path}/"), route)
        } else {
            self.route(path, route)
        }
    }
}

fn build_router(app_state: AppState) -> trailing_slash::TrailingSlash<Router> {
    let control = app_state.control.clone();
    let router = Router::new()
        .protocol_route("/status", get(webdriver_classic::webdriver_classic_status))
        .protocol_route(JSON_DISCOVERY_PATH, any(cdp::json_list))
        .protocol_route("/json/list", any(cdp::json_list))
        .protocol_route("/json/version", get(cdp::json_version))
        .protocol_route("/json/protocol", get(cdp::json_protocol))
        .protocol_route("/json/new", put(cdp::json_new_target))
        .protocol_route("/json/activate/{target_id}", get(cdp::json_activate_target))
        .protocol_route("/json/close/{target_id}", get(cdp::json_close_target))
        .protocol_route(
            "/devtools/browser/{browser_id}",
            get(cdp::ws_browser_upgrade_handler),
        )
        .protocol_route(
            "/devtools/page/{target_id}",
            get(cdp::ws_target_upgrade_handler),
        )
        .protocol_route(
            "/session",
            get(webdriver_bidi::ws_bidi_session_upgrade_handler)
                .post(webdriver_classic::webdriver_classic_new_session),
        )
        .protocol_route(
            "/session/{session_id}",
            get(webdriver_bidi::ws_bidi_existing_session_upgrade_handler)
                .delete(webdriver_classic::webdriver_classic_delete_session),
        )
        .protocol_route(
            "/session/{session_id}/url",
            get(webdriver_classic::webdriver_classic_get_url)
                .post(webdriver_classic::webdriver_classic_navigate),
        )
        .protocol_route(
            "/session/{session_id}/title",
            get(webdriver_classic::webdriver_classic_get_title),
        )
        .protocol_route(
            "/session/{session_id}/timeouts",
            get(webdriver_classic::webdriver_classic_get_timeouts)
                .post(webdriver_classic::webdriver_classic_set_timeouts),
        )
        .protocol_route(
            "/session/{session_id}/source",
            get(webdriver_classic::webdriver_classic_get_source),
        )
        .protocol_route(
            "/session/{session_id}/screenshot",
            get(webdriver_classic::webdriver_classic_take_screenshot),
        )
        .protocol_route(
            "/session/{session_id}/print",
            post(webdriver_classic::webdriver_classic_print_page),
        )
        .protocol_route(
            "/session/{session_id}/file",
            post(webdriver_classic::webdriver_classic_upload_file),
        )
        .protocol_route(
            "/session/{session_id}/se/file",
            post(webdriver_classic::webdriver_classic_upload_file),
        )
        .protocol_route(
            "/session/{session_id}/se/files",
            get(webdriver_classic::webdriver_classic_get_downloadable_files)
                .post(webdriver_classic::webdriver_classic_download_file)
                .delete(webdriver_classic::webdriver_classic_delete_downloadable_files),
        )
        .protocol_route(
            "/session/{session_id}/moli/service-workers",
            get(webdriver_classic::webdriver_classic_get_service_workers),
        )
        .protocol_route(
            "/session/{session_id}/window",
            get(webdriver_classic::webdriver_classic_get_window)
                .post(webdriver_classic::webdriver_classic_switch_window)
                .delete(webdriver_classic::webdriver_classic_close_window),
        )
        .protocol_route(
            "/session/{session_id}/window/handles",
            get(webdriver_classic::webdriver_classic_get_window_handles),
        )
        .protocol_route(
            "/session/{session_id}/window/rect",
            get(webdriver_classic::webdriver_classic_get_window_rect)
                .post(webdriver_classic::webdriver_classic_set_window_rect),
        )
        .protocol_route(
            "/session/{session_id}/window/maximize",
            post(webdriver_classic::webdriver_classic_maximize_window),
        )
        .protocol_route(
            "/session/{session_id}/window/minimize",
            post(webdriver_classic::webdriver_classic_minimize_window),
        )
        .protocol_route(
            "/session/{session_id}/window/fullscreen",
            post(webdriver_classic::webdriver_classic_fullscreen_window),
        )
        .protocol_route(
            "/session/{session_id}/window/new",
            post(webdriver_classic::webdriver_classic_new_window),
        )
        .protocol_route(
            "/session/{session_id}/frame",
            post(webdriver_classic::webdriver_classic_switch_frame),
        )
        .protocol_route(
            "/session/{session_id}/frame/parent",
            post(webdriver_classic::webdriver_classic_switch_parent_frame),
        )
        .protocol_route(
            "/session/{session_id}/alert/text",
            get(webdriver_classic::webdriver_classic_get_alert_text)
                .post(webdriver_classic::webdriver_classic_send_alert_text),
        )
        .protocol_route(
            "/session/{session_id}/alert/accept",
            post(webdriver_classic::webdriver_classic_accept_alert),
        )
        .protocol_route(
            "/session/{session_id}/alert/dismiss",
            post(webdriver_classic::webdriver_classic_dismiss_alert),
        )
        .protocol_route(
            "/session/{session_id}/refresh",
            post(webdriver_classic::webdriver_classic_refresh),
        )
        .protocol_route(
            "/session/{session_id}/back",
            post(webdriver_classic::webdriver_classic_back),
        )
        .protocol_route(
            "/session/{session_id}/forward",
            post(webdriver_classic::webdriver_classic_forward),
        )
        .protocol_route(
            "/session/{session_id}/execute/sync",
            post(webdriver_classic::webdriver_classic_execute_sync),
        )
        .protocol_route(
            "/session/{session_id}/execute/async",
            post(webdriver_classic::webdriver_classic_execute_async),
        )
        .protocol_route(
            "/session/{session_id}/element",
            post(webdriver_classic::webdriver_classic_find_element),
        )
        .protocol_route(
            "/session/{session_id}/elements",
            post(webdriver_classic::webdriver_classic_find_elements),
        )
        .protocol_route(
            "/session/{session_id}/element/active",
            get(webdriver_classic::webdriver_classic_get_active_element),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/element",
            post(webdriver_classic::webdriver_classic_find_child_element),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/elements",
            post(webdriver_classic::webdriver_classic_find_child_elements),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/shadow",
            get(webdriver_classic::webdriver_classic_get_element_shadow_root),
        )
        .protocol_route(
            "/session/{session_id}/shadow/{shadow_root_id}/element",
            post(webdriver_classic::webdriver_classic_find_shadow_element),
        )
        .protocol_route(
            "/session/{session_id}/shadow/{shadow_root_id}/elements",
            post(webdriver_classic::webdriver_classic_find_shadow_elements),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/attribute/{name}",
            get(webdriver_classic::webdriver_classic_get_element_attribute),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/text",
            get(webdriver_classic::webdriver_classic_get_element_text),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/name",
            get(webdriver_classic::webdriver_classic_get_element_tag_name),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/equals/{other_element_id}",
            get(webdriver_classic::webdriver_classic_element_equals),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/enabled",
            get(webdriver_classic::webdriver_classic_is_element_enabled),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/displayed",
            get(webdriver_classic::webdriver_classic_is_element_displayed),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/selected",
            get(webdriver_classic::webdriver_classic_is_element_selected),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/rect",
            get(webdriver_classic::webdriver_classic_get_element_rect),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/screenshot",
            get(webdriver_classic::webdriver_classic_take_element_screenshot),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/css/{property_name}",
            get(webdriver_classic::webdriver_classic_get_element_css_value),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/computedlabel",
            get(webdriver_classic::webdriver_classic_get_element_computed_label),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/computedrole",
            get(webdriver_classic::webdriver_classic_get_element_computed_role),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/property/{name}",
            get(webdriver_classic::webdriver_classic_get_element_property),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/clear",
            post(webdriver_classic::webdriver_classic_clear_element),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/click",
            post(webdriver_classic::webdriver_classic_click_element),
        )
        .protocol_route(
            "/session/{session_id}/element/{element_id}/value",
            post(webdriver_classic::webdriver_classic_send_keys_to_element),
        )
        .protocol_route(
            "/session/{session_id}/actions",
            post(webdriver_classic::webdriver_classic_perform_actions)
                .delete(webdriver_classic::webdriver_classic_release_actions),
        )
        .protocol_route(
            "/session/{session_id}/cookie",
            get(webdriver_classic::webdriver_classic_get_cookies)
                .post(webdriver_classic::webdriver_classic_add_cookie)
                .delete(webdriver_classic::webdriver_classic_delete_all_cookies),
        )
        .protocol_route(
            "/session/{session_id}/cookie/{name}",
            get(webdriver_classic::webdriver_classic_get_named_cookie)
                .delete(webdriver_classic::webdriver_classic_delete_cookie),
        )
        .with_state(app_state)
        .layer(middleware::from_fn(
            classic_webdriver_response_headers_middleware,
        ))
        .layer(middleware::from_fn_with_state(control, instance_admission));
    // The outer make-service wrapper bypasses Router's own eager finalization.
    let router = router.with_state(());
    trailing_slash::TrailingSlashLayer::new(&[JSON_DISCOVERY_PATH]).layer(router)
}

async fn instance_admission(
    State(control): State<BrowserInstanceControl>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(_connection) = control.connection() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    next.run(request).await
}

async fn classic_webdriver_response_headers_middleware(
    request: Request<Body>,
    next: Next,
) -> Response {
    let should_apply = is_classic_webdriver_http_path(request.uri().path())
        && !is_websocket_upgrade_request(&request);
    let mut response = next.run(request).await;
    if should_apply && is_classic_webdriver_json_response(&response) {
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    response
}

fn is_classic_webdriver_json_response(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split_once(';')
                .map_or(value, |(media_type, _)| media_type)
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
}

fn is_classic_webdriver_http_path(path: &str) -> bool {
    path == "/status"
        || path == "/status/"
        || path == "/session"
        || path == "/session/"
        || path.starts_with("/session/")
}

fn is_websocket_upgrade_request(request: &Request<Body>) -> bool {
    request
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
}

#[derive(Clone)]
struct AppState {
    control: BrowserInstanceControl,
    debugger_enabled: bool,
    browser_ws_url: String,
    page_ws_url: String,
    bidi_ws_url: String,
    bidi_session_registry: SharedBidiSessionRegistry,
    classic_session_registry: SharedClassicSessionRegistry,
    cdp_agent_host_directory: SharedCdpAgentHostDirectory,
    cdp_owner_registry: SharedCdpOwnerRegistry,
    devtools_frontend_url: String,
    storage_partition: Arc<StoragePartitionState>,
    fetch_config: FetchConfig,
    optional_resource_fetch_mask: OptionalResourceFetchMask,
    subframe_loading_enabled: bool,
    layout_configuration: LayoutConfiguration,
}

impl AppState {
    #[cfg(test)]
    fn new_with_storage_partition(
        addr: SocketAddr,
        storage_partition: Arc<StoragePartitionState>,
        fetch_config: FetchConfig,
        optional_resource_fetch_mask: OptionalResourceFetchMask,
        subframe_loading_enabled: bool,
    ) -> anyhow::Result<Self> {
        Self::new_with_storage_partition_and_runtime_config(
            addr,
            storage_partition,
            NavigationRuntimeConfig::new(
                fetch_config,
                optional_resource_fetch_mask,
                subframe_loading_enabled,
                LayoutPolicy::default(),
            ),
            crate::config::DEFAULT_SCREENCAST_INTERVAL_MS,
            BrowserInstanceControl::new(),
        )
    }

    fn new_with_storage_partition_and_runtime_config(
        addr: SocketAddr,
        storage_partition: Arc<StoragePartitionState>,
        navigation_runtime_config: NavigationRuntimeConfig,
        screencast_interval_ms: u32,
        control: BrowserInstanceControl,
    ) -> anyhow::Result<Self> {
        let cdp_agent_host_directory = SharedCdpAgentHostDirectory::default();
        let cdp_target_id_allocator = Arc::new(AtomicU64::new(0));
        let cdp_tab_target_id_allocator = Arc::new(AtomicU64::new(0));
        let cdp_owner_registry = SharedCdpOwnerRegistry::new(
            cdp_agent_host_directory.clone(),
            cdp_target_id_allocator,
            cdp_tab_target_id_allocator,
            storage_partition.clone(),
            navigation_runtime_config.clone(),
            screencast_interval_ms,
            control.clone(),
        );
        Ok(Self {
            control,
            browser_ws_url: format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"),
            debugger_enabled: navigation_runtime_config.debugger_enabled(),
            page_ws_url: format!("ws://{addr}/devtools/page/{DEFAULT_TARGET_ID}"),
            bidi_ws_url: format!("ws://{addr}/session"),
            bidi_session_registry: SharedBidiSessionRegistry::default(),
            classic_session_registry: SharedClassicSessionRegistry::default(),
            cdp_agent_host_directory,
            cdp_owner_registry,
            devtools_frontend_url: format!(
                "/devtools/inspector.html?ws={addr}/devtools/page/{DEFAULT_TARGET_ID}"
            ),
            storage_partition,
            fetch_config: navigation_runtime_config.fetch_config().clone(),
            optional_resource_fetch_mask: navigation_runtime_config.optional_resource_fetch_mask(),
            subframe_loading_enabled: navigation_runtime_config.subframe_loading_enabled(),
            layout_configuration: navigation_runtime_config.layout_configuration(),
        })
    }

    fn initial_storage_partition(&self) -> CdpInitialStoragePartition {
        CdpInitialStoragePartition::from_storage_partition(&self.storage_partition)
    }
}

#[cfg(test)]
mod tests;
