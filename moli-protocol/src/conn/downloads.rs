use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use http::HeaderName;
use moli_core::network::ResourceRequestClient;
use moli_core::page::RendererPendingDownloadActivation;
use moli_fetch::{FetchCancelHandle, Request};
use moli_header_field::{split_outside_quoted_strings, unquote_parameter_value};
use moli_web_mime::response_headers_indicate_attachment_download;
use parking_lot::Mutex;
use sanitize_filename::Options;
use tokio::io::AsyncWriteExt;
use url::Url;

use super::{
    BackgroundProtocolEvent, BrowserDownloadBehaviorSettings, CdpConnection,
    CommandDispatchContext, CompletedDownloadBody, CompletedDownloadBodyArtifact,
    NavigationDispatchState, output::BackgroundEventSender,
};

#[derive(Clone, Default)]
pub(crate) struct SharedDownloadRegistry {
    inner: Arc<Mutex<HashMap<String, DownloadRecord>>>,
}

#[derive(Debug, Clone)]
struct DownloadRecord {
    state: DownloadLifecycle,
    artifact_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
enum DownloadLifecycle {
    Active(FetchCancelHandle),
    Completed,
    Canceled,
}

impl SharedDownloadRegistry {
    fn insert_active(&self, guid: String, cancel_handle: FetchCancelHandle) {
        self.with_mut(|downloads| {
            downloads.insert(
                guid,
                DownloadRecord {
                    state: DownloadLifecycle::Active(cancel_handle),
                    artifact_path: None,
                },
            );
        });
    }

    fn mark_completed(&self, guid: &str, artifact_path: PathBuf) {
        self.with_mut(|downloads| match downloads.get_mut(guid) {
            Some(record) => {
                record.state = DownloadLifecycle::Completed;
                record.artifact_path = Some(artifact_path);
            }
            None => {
                downloads.insert(
                    guid.to_owned(),
                    DownloadRecord {
                        state: DownloadLifecycle::Completed,
                        artifact_path: Some(artifact_path),
                    },
                );
            }
        });
    }

    fn mark_canceled(&self, guid: &str) {
        self.with_mut(|downloads| match downloads.get_mut(guid) {
            Some(record) => {
                record.state = DownloadLifecycle::Canceled;
            }
            None => {
                downloads.insert(
                    guid.to_owned(),
                    DownloadRecord {
                        state: DownloadLifecycle::Canceled,
                        artifact_path: None,
                    },
                );
            }
        });
    }

    fn cancel(&self, guid: &str) -> CancelDownloadOutcome {
        self.with_mut(|downloads| match downloads.get_mut(guid) {
            Some(DownloadRecord {
                state: DownloadLifecycle::Active(cancel_handle),
                ..
            }) => {
                cancel_handle.cancel();
                CancelDownloadOutcome::Handled
            }
            Some(_) => CancelDownloadOutcome::AlreadyTerminal,
            None => CancelDownloadOutcome::NotFound,
        })
    }

    fn open_artifact(&self, guid: &str) -> OpenDownloadArtifactOutcome {
        self.with_mut(|downloads| match downloads.get(guid) {
            Some(DownloadRecord {
                state: DownloadLifecycle::Completed,
                artifact_path: Some(path),
            }) => OpenDownloadArtifactOutcome::Ready(path.clone()),
            Some(DownloadRecord {
                state: DownloadLifecycle::Active(_),
                ..
            }) => OpenDownloadArtifactOutcome::InProgress,
            Some(_) => OpenDownloadArtifactOutcome::NotAvailable,
            None => OpenDownloadArtifactOutcome::NotFound,
        })
    }

    fn with_mut<T>(&self, f: impl FnOnce(&mut HashMap<String, DownloadRecord>) -> T) -> T {
        let mut downloads = self.inner.lock();
        f(&mut downloads)
    }
}

enum CancelDownloadOutcome {
    Handled,
    AlreadyTerminal,
    NotFound,
}

enum OpenDownloadArtifactOutcome {
    Ready(PathBuf),
    InProgress,
    NotAvailable,
    NotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString)]
#[strum(serialize_all = "camelCase")]
enum DownloadBehavior {
    Default,
    Deny,
    Allow,
    AllowAndName,
}

impl DownloadBehavior {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn allows_download(self) -> bool {
        matches!(self, Self::Allow | Self::AllowAndName)
    }

    fn names_artifact_by_guid(self) -> bool {
        self == Self::AllowAndName
    }

    fn is_canceled_without_download(self) -> bool {
        matches!(self, Self::Default | Self::Deny)
    }
}

struct PreparedDownloadActivation {
    frame_id: String,
    request: Request,
    loader: ResourceRequestClient,
    download_root: String,
    guid: String,
    behavior: String,
    event_route: DownloadEventRoute,
    suggested_filename_hint: Option<String>,
    cancel_handle: FetchCancelHandle,
    registry: SharedDownloadRegistry,
}

struct PreparedNavigationDownload {
    frame_id: String,
    response_url: Url,
    response_headers: Vec<(String, String)>,
    response_body: CompletedDownloadBody,
    download_root: String,
    guid: String,
    behavior: String,
    event_route: DownloadEventRoute,
    registry: SharedDownloadRegistry,
}

struct PendingDownloadOwnerContext {
    browser_context_id: String,
    frame_id: String,
    request_headers: Vec<(String, String)>,
    initiator_url: Option<Url>,
}

#[derive(Debug)]
struct DownloadEventRoute {
    browser_observers: Vec<BrowserDownloadObserver>,
    automation_events_enabled: bool,
    page_observers: Vec<PageDownloadObserver>,
}

#[derive(Debug)]
struct BrowserDownloadObserver {
    session_id: Option<String>,
    subscription_generation: u64,
}

#[derive(Debug)]
struct PageDownloadObserver {
    session_id: Option<String>,
    subscription_generation: u64,
}

impl DownloadEventRoute {
    fn has_observers(&self) -> bool {
        self.automation_events_enabled
            || !self.browser_observers.is_empty()
            || !self.page_observers.is_empty()
    }
}

impl CdpConnection {
    pub(crate) async fn handle_pending_download_activation_background_events_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
        activation: RendererPendingDownloadActivation,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        self.handle_pending_download_activation_with_event_route_async(
            out,
            session_id,
            activation,
            true,
            command_context,
        )
        .await
    }

    pub(crate) async fn handle_pending_download_activation_inline_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
        activation: RendererPendingDownloadActivation,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        self.handle_pending_download_activation_with_event_route_async(
            out,
            session_id,
            activation,
            false,
            command_context,
        )
        .await
    }

    async fn handle_pending_download_activation_with_event_route_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
        activation: RendererPendingDownloadActivation,
        allow_background_events: bool,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        if let Some(events) =
            self.denied_pending_download_activation_events(session_id, &activation)?
        {
            if allow_background_events && let Some(sender) = self.background_event_sender() {
                if command_context.response_flush().is_active() {
                    command_context.extend_post_response_events(events);
                } else {
                    send_background_download_events(&sender, events);
                }
            } else {
                out.extend(events);
            }
            return Ok(());
        }

        if activation.response.is_some() {
            let Some(prepared) =
                self.prepare_prefetched_download_activation(session_id, activation)?
            else {
                return Ok(());
            };
            if allow_background_events && let Some(sender) = self.background_event_sender() {
                let response_flush = command_context.response_flush().receiver();
                tokio::spawn(async move {
                    if !wait_for_command_response_flush(response_flush).await {
                        return;
                    }
                    for event in complete_navigation_download_async(prepared).await {
                        let _ = sender.send(event);
                    }
                });
                return Ok(());
            }

            for event in complete_navigation_download_async(prepared).await {
                out.push(event);
            }
            return Ok(());
        }

        let Some(prepared) = self.prepare_pending_download_activation(session_id, activation)?
        else {
            return Ok(());
        };

        prepared
            .registry
            .insert_active(prepared.guid.clone(), prepared.cancel_handle.clone());

        if allow_background_events && let Some(sender) = self.background_event_sender() {
            let emit_early_start = can_emit_early_download_start(&prepared);
            let response_flush = command_context.response_flush().receiver();
            if emit_early_start {
                let events = start_download_events(&prepared);
                if response_flush.is_some() {
                    command_context.extend_post_response_events(events);
                } else {
                    send_background_download_events(&sender, events);
                }
            }
            tokio::spawn(async move {
                if !wait_for_command_response_flush(response_flush).await {
                    return;
                }
                let events = if emit_early_start {
                    complete_download_activation_async(
                        prepared,
                        DownloadActivationStartEvents::AlreadyEmitted(sender.clone()),
                    )
                    .await
                } else {
                    complete_download_activation_async(
                        prepared,
                        DownloadActivationStartEvents::SendToBackground(sender.clone()),
                    )
                    .await
                };
                for event in events {
                    let _ = sender.send(event);
                }
            });
            return Ok(());
        }

        for event in complete_download_activation_async(
            prepared,
            DownloadActivationStartEvents::ReturnToCaller,
        )
        .await
        {
            out.push(event);
        }

        Ok(())
    }

    pub(crate) async fn handle_navigation_download_response_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        state: &NavigationDispatchState,
        final_url: Url,
        body_artifact: CompletedDownloadBodyArtifact,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        let Some(prepared) = self.prepare_navigation_download(state, final_url, body_artifact)?
        else {
            return Ok(());
        };

        if let Some(sender) = self.background_event_sender() {
            let response_flush = command_context.response_flush().receiver();
            tokio::spawn(async move {
                if !wait_for_command_response_flush(response_flush).await {
                    return;
                }
                for event in complete_navigation_download_async(prepared).await {
                    let _ = sender.send(event);
                }
            });
            return Ok(());
        }

        for event in complete_navigation_download_async(prepared).await {
            out.push(event);
        }

        Ok(())
    }

    fn prepare_pending_download_activation(
        &mut self,
        session_id: Option<&str>,
        activation: RendererPendingDownloadActivation,
    ) -> Result<Option<PreparedDownloadActivation>, String> {
        let Some(owner) = self.pending_download_owner_context(session_id) else {
            return Ok(None);
        };
        let Some(settings) =
            self.effective_download_behavior_for_browser_context(Some(&owner.browser_context_id))
        else {
            return Ok(None);
        };

        let Some(download_root) = settings.download_path.clone() else {
            return Ok(None);
        };

        let mut request = Request::get(&activation.url)
            .map_err(|error| format!("invalid download url: {error}"))?;
        request.request_headers = owner.request_headers;
        request = request.with_top_level_navigation_cookie_context();
        if let Some(ref initiator_url) = owner.initiator_url {
            request = request.with_initiator_url(initiator_url);
        }

        let loader = self.ensure_resource_request_client()?.clone();
        let guid = generate_download_guid()?;
        let cancel_handle = FetchCancelHandle::new();

        Ok(Some(PreparedDownloadActivation {
            frame_id: owner.frame_id,
            request,
            loader,
            download_root,
            guid,
            behavior: settings.behavior,
            event_route: self.download_event_route(session_id, settings.automation_events_enabled),
            suggested_filename_hint: activation.suggested_filename,
            cancel_handle,
            registry: self.download_registry.clone(),
        }))
    }

    fn denied_pending_download_activation_events(
        &mut self,
        session_id: Option<&str>,
        activation: &RendererPendingDownloadActivation,
    ) -> Result<Option<Vec<BackgroundProtocolEvent>>, String> {
        let Some(owner) = self.pending_download_owner_context(session_id) else {
            return Ok(None);
        };
        let settings = self
            .download_behavior
            .effective_for_browser_context(Some(&owner.browser_context_id));
        if !DownloadBehavior::parse(settings.behavior.as_str())
            .is_some_and(DownloadBehavior::is_canceled_without_download)
        {
            return Ok(None);
        }
        let event_route = self.download_event_route(session_id, settings.automation_events_enabled);
        if !event_route.has_observers() {
            return Ok(Some(Vec::new()));
        }

        let response_url = activation
            .response
            .as_ref()
            .map(|response| response.final_url.as_str())
            .unwrap_or(activation.url.as_str());
        let suggested_filename = activation
            .response
            .as_ref()
            .and_then(|response| filename_from_headers(&response.headers))
            .or_else(|| {
                activation
                    .suggested_filename
                    .as_deref()
                    .and_then(non_empty_filename)
                    .map(str::to_owned)
            })
            .or_else(|| {
                Url::parse(response_url)
                    .ok()
                    .and_then(|url| filename_from_url(&url))
            })
            .unwrap_or_else(|| "download".to_owned());
        let guid = generate_download_guid()?;
        Ok(Some(denied_download_activation_events(
            &event_route,
            &owner.frame_id,
            &guid,
            response_url,
            &suggested_filename,
        )))
    }

    fn prepare_prefetched_download_activation(
        &mut self,
        session_id: Option<&str>,
        activation: RendererPendingDownloadActivation,
    ) -> Result<Option<PreparedNavigationDownload>, String> {
        let Some(owner) = self.pending_download_owner_context(session_id) else {
            return Ok(None);
        };
        let Some(settings) =
            self.effective_download_behavior_for_browser_context(Some(&owner.browser_context_id))
        else {
            return Ok(None);
        };

        let Some(response) = activation.response else {
            return Ok(None);
        };
        let Some(download_root) = settings.download_path.clone() else {
            return Ok(None);
        };

        let response_url = Url::parse(&response.final_url)
            .or_else(|_| Url::parse(&activation.url))
            .map_err(|error| format!("invalid download url: {error}"))?;
        let response_headers = response.headers;
        let response_body = CompletedDownloadBody::Buffered(response.body);

        Ok(Some(PreparedNavigationDownload {
            frame_id: owner.frame_id,
            response_url,
            response_headers,
            response_body,
            download_root,
            guid: generate_download_guid()?,
            behavior: settings.behavior,
            event_route: self.download_event_route(session_id, settings.automation_events_enabled),
            registry: self.download_registry.clone(),
        }))
    }

    fn prepare_navigation_download(
        &mut self,
        state: &NavigationDispatchState,
        final_url: Url,
        body_artifact: CompletedDownloadBodyArtifact,
    ) -> Result<Option<PreparedNavigationDownload>, String> {
        let owner_context_id = state
            .session_id
            .as_deref()
            .and_then(|session_id| {
                self.target_owner_identity_for_session(Some(session_id))
                    .map(|(browser_context_id, _)| browser_context_id)
            })
            .or_else(|| self.browser_context.as_ref().map(|bc| bc.id.clone()));
        let Some(settings) =
            self.effective_download_behavior_for_browser_context(owner_context_id.as_deref())
        else {
            return Ok(None);
        };

        let Some(download_root) = settings.download_path.clone() else {
            return Ok(None);
        };

        let (response_body, response_headers) = body_artifact.into_parts();

        Ok(Some(PreparedNavigationDownload {
            // Navigation may complete after another frontend has changed the active target.
            // The dispatch snapshot is the authority for the frame that initiated this download.
            frame_id: state.frame_id.clone(),
            response_url: final_url,
            response_headers,
            response_body,
            download_root,
            guid: generate_download_guid()?,
            behavior: settings.behavior,
            event_route: self.download_event_route(
                state.session_id.as_deref(),
                settings.automation_events_enabled,
            ),
            registry: self.download_registry.clone(),
        }))
    }

    pub(crate) fn cancel_download(&self, guid: &str) -> Result<(), String> {
        match self.download_registry.cancel(guid) {
            CancelDownloadOutcome::Handled => Ok(()),
            CancelDownloadOutcome::AlreadyTerminal => {
                Err("Download item is no longer active".to_owned())
            }
            CancelDownloadOutcome::NotFound => {
                Err("No download item found for the given GUID".to_owned())
            }
        }
    }

    pub(crate) fn start_open_download_as_stream(
        &self,
        guid: &str,
    ) -> Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, String> {
        let artifact_path = match self.download_registry.open_artifact(guid) {
            OpenDownloadArtifactOutcome::Ready(path) => path,
            OpenDownloadArtifactOutcome::InProgress => {
                return Err("Download item is not completed yet".to_owned());
            }
            OpenDownloadArtifactOutcome::NotAvailable => {
                return Err("Download item has no readable artifact".to_owned());
            }
            OpenDownloadArtifactOutcome::NotFound => {
                return Err("No download item found for the given GUID".to_owned());
            }
        };

        let artifact_path_for_read = artifact_path.clone();
        let artifact_path_label = artifact_path.display().to_string();
        Ok(tokio::task::spawn_blocking(move || {
            fs::read(&artifact_path_for_read)
                .map_err(|_| format!("Download artifact not found: {artifact_path_label}"))
        }))
    }

    pub(crate) fn finish_open_download_as_stream(&mut self, bytes: Vec<u8>) -> String {
        self.open_global_io_stream(bytes)
    }

    fn pending_download_owner_context(
        &self,
        session_id: Option<&str>,
    ) -> Option<PendingDownloadOwnerContext> {
        let (browser_context_id, target_id) = self.target_owner_identity_for_session(session_id)?;
        let browser_context = self.browser_context_by_id(&browser_context_id)?;
        let frame_id = target_id
            .clone()
            .or_else(|| browser_context.active_target_id_owned())
            .unwrap_or_else(|| "FRAME-0".to_owned());
        let request_headers = match target_id.as_deref() {
            Some(target_id) if browser_context.active_target_id() != Some(target_id) => {
                browser_context
                    .parked_page_session_state(target_id)
                    .map(|state| state.network_policy.extra_headers().to_vec())
                    .unwrap_or_default()
            }
            _ => browser_context.effective_extra_headers(),
        };
        let initiator_url = self
            .runtime_session_owner_slot(session_id)
            .ok()
            .and_then(|slot| slot.loaded_page().map(|page| page.final_url().clone()));
        Some(PendingDownloadOwnerContext {
            browser_context_id,
            frame_id,
            request_headers,
            initiator_url,
        })
    }

    fn effective_download_behavior_for_browser_context(
        &self,
        browser_context_id: Option<&str>,
    ) -> Option<BrowserDownloadBehaviorSettings> {
        let settings = self
            .download_behavior
            .effective_for_browser_context(browser_context_id);
        if !DownloadBehavior::parse(settings.behavior.as_str())
            .is_some_and(DownloadBehavior::allows_download)
        {
            return None;
        }
        Some(settings)
    }

    fn download_event_route(
        &self,
        session_id: Option<&str>,
        automation_events_enabled: bool,
    ) -> DownloadEventRoute {
        let page_observers = self
            .page_event_session_ids_for_session_owner(session_id)
            .into_iter()
            .filter_map(|event_session_id| {
                self.page_domain_subscription_generation_for_session_owner(
                    event_session_id.as_deref(),
                )
                .map(|subscription_generation| PageDownloadObserver {
                    session_id: event_session_id,
                    subscription_generation,
                })
            })
            .collect();
        DownloadEventRoute {
            browser_observers: self
                .download_behavior
                .browser_event_observers()
                .into_iter()
                .map(
                    |(session_id, subscription_generation)| BrowserDownloadObserver {
                        session_id,
                        subscription_generation,
                    },
                )
                .collect(),
            automation_events_enabled: automation_events_enabled
                || self.download_behavior.webdriver_bidi_events_enabled,
            page_observers,
        }
    }
}

enum DownloadActivationStartEvents {
    AlreadyEmitted(BackgroundEventSender),
    ReturnToCaller,
    SendToBackground(BackgroundEventSender),
}

impl DownloadActivationStartEvents {
    fn progress_sender(&self) -> Option<&BackgroundEventSender> {
        match self {
            Self::AlreadyEmitted(sender) => Some(sender),
            Self::SendToBackground(sender) => Some(sender),
            Self::ReturnToCaller => None,
        }
    }
}

async fn complete_download_activation_async(
    prepared: PreparedDownloadActivation,
    start_events: DownloadActivationStartEvents,
) -> Vec<BackgroundProtocolEvent> {
    // Active downloads can be arbitrarily large, so stream chunks directly into
    // the artifact instead of materializing a RawResponse body in memory.
    let mut response = match prepared
        .loader
        .fetch_raw_stream_with_cancel(prepared.request.clone(), prepared.cancel_handle.clone())
        .await
    {
        Ok(response) => response,
        Err(_) => {
            prepared.registry.mark_canceled(&prepared.guid);
            return download_activation_failed_before_response_events(&prepared, &start_events);
        }
    };

    let suggested_filename = filename_from_headers(&response.headers)
        .or_else(|| prepared.suggested_filename_hint.clone())
        .or_else(|| filename_from_url(&response.final_url))
        .unwrap_or_else(|| pending_download_filename(&prepared));
    let artifact_name = artifact_file_name(
        prepared.behavior.as_str(),
        &prepared.guid,
        &suggested_filename,
    );
    let artifact_path = Path::new(&prepared.download_root).join(&artifact_name);
    let partial_path = partial_artifact_path(&artifact_path);
    let expected_total_bytes = content_length_from_headers(&response.headers);
    let mut events = emit_download_activation_start(
        &prepared,
        &start_events,
        &response.final_url,
        &suggested_filename,
    );

    if let Err(_error) = tokio::fs::create_dir_all(&prepared.download_root).await {
        prepared.registry.mark_canceled(&prepared.guid);
        let _ = response.finish().await;
        events.extend(terminal_download_events(&prepared, None, None, true));
        return events;
    }

    let mut file = match tokio::fs::File::create(&partial_path).await {
        Ok(file) => file,
        Err(_error) => {
            prepared.registry.mark_canceled(&prepared.guid);
            let _ = response.finish().await;
            events.extend(terminal_download_events(&prepared, None, None, true));
            return events;
        }
    };

    let mut total_bytes = 0_u64;
    while let Some(chunk) = response.next_chunk().await {
        total_bytes = total_bytes.saturating_add(chunk.len() as u64);
        if file.write_all(&chunk).await.is_err() {
            prepared.cancel_handle.cancel();
            prepared.registry.mark_canceled(&prepared.guid);
            let _ = response.finish().await;
            let _ = tokio::fs::remove_file(&partial_path).await;
            events.extend(terminal_download_events(
                &prepared,
                Some(total_bytes),
                None,
                true,
            ));
            return events;
        }
        let progress_events =
            progress_download_events(&prepared, total_bytes, expected_total_bytes);
        if let Some(sender) = start_events.progress_sender() {
            send_background_download_events(sender, progress_events);
        } else {
            events.extend(progress_events);
        }
    }

    if response.finish().await.is_err() || file.flush().await.is_err() {
        prepared.registry.mark_canceled(&prepared.guid);
        let _ = tokio::fs::remove_file(&partial_path).await;
        events.extend(terminal_download_events(
            &prepared,
            Some(total_bytes),
            None,
            true,
        ));
        return events;
    }
    drop(file);

    if finalize_download_artifact(&partial_path, &artifact_path)
        .await
        .is_err()
    {
        prepared.registry.mark_canceled(&prepared.guid);
        let _ = tokio::fs::remove_file(&partial_path).await;
        events.extend(terminal_download_events(
            &prepared,
            Some(total_bytes),
            None,
            true,
        ));
        return events;
    }

    prepared
        .registry
        .mark_completed(&prepared.guid, artifact_path.clone());
    events.extend(terminal_download_events(
        &prepared,
        Some(total_bytes),
        Some(artifact_path),
        false,
    ));
    events
}

fn download_activation_failed_before_response_events(
    prepared: &PreparedDownloadActivation,
    start_events: &DownloadActivationStartEvents,
) -> Vec<BackgroundProtocolEvent> {
    match start_events {
        DownloadActivationStartEvents::AlreadyEmitted(_) => {
            terminal_download_events(prepared, None, None, true)
        }
        DownloadActivationStartEvents::ReturnToCaller
        | DownloadActivationStartEvents::SendToBackground(_) => {
            let suggested_filename = pending_download_filename(prepared);
            let mut events = deferred_start_download_events(
                prepared,
                prepared.request.url.as_str(),
                &suggested_filename,
            );
            events.extend(terminal_download_events(prepared, None, None, true));
            events
        }
    }
}

fn emit_download_activation_start(
    prepared: &PreparedDownloadActivation,
    start_events: &DownloadActivationStartEvents,
    final_url: &Url,
    suggested_filename: &str,
) -> Vec<BackgroundProtocolEvent> {
    match start_events {
        DownloadActivationStartEvents::AlreadyEmitted(_) => Vec::new(),
        DownloadActivationStartEvents::ReturnToCaller => {
            deferred_start_download_events(prepared, final_url.as_str(), suggested_filename)
        }
        DownloadActivationStartEvents::SendToBackground(sender) => {
            let mut events =
                deferred_start_download_events(prepared, final_url.as_str(), suggested_filename);
            events.extend(in_progress_download_events(prepared));
            send_background_download_events(sender, events);
            Vec::new()
        }
    }
}

fn send_background_download_events(
    sender: &BackgroundEventSender,
    events: Vec<BackgroundProtocolEvent>,
) {
    for event in events {
        let _ = sender.send(event);
    }
}

async fn wait_for_command_response_flush(
    mut receiver: Option<tokio::sync::watch::Receiver<bool>>,
) -> bool {
    let Some(receiver) = receiver.as_mut() else {
        return true;
    };
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            return false;
        }
    }
    true
}

async fn complete_navigation_download_async(
    mut prepared: PreparedNavigationDownload,
) -> Vec<BackgroundProtocolEvent> {
    let suggested_filename = filename_from_headers(&prepared.response_headers)
        .or_else(|| filename_from_url(&prepared.response_url))
        .unwrap_or_else(|| "download".to_owned());
    let artifact_name = artifact_file_name(
        prepared.behavior.as_str(),
        &prepared.guid,
        &suggested_filename,
    );
    let artifact_path = Path::new(&prepared.download_root).join(&artifact_name);

    let mut events = build_navigation_download_will_begin_event(&prepared, &suggested_filename);
    events.extend(build_navigation_download_in_progress_event(&prepared));

    let response_body = std::mem::replace(
        &mut prepared.response_body,
        CompletedDownloadBody::Buffered(Vec::new()),
    );
    match write_navigation_download_body_async(
        &prepared.download_root,
        &artifact_path,
        response_body,
    )
    .await
    {
        Ok(total_bytes) => {
            prepared
                .registry
                .mark_completed(&prepared.guid, artifact_path.clone());
            events.extend(build_navigation_download_terminal_event(
                &prepared,
                Some(total_bytes),
                Some(artifact_path),
                false,
            ));
        }
        Err(_) => {
            prepared.registry.mark_canceled(&prepared.guid);
            events.extend(build_navigation_download_terminal_event(
                &prepared, None, None, true,
            ));
        }
    }

    events
}

async fn write_navigation_download_body_async(
    download_root: &str,
    artifact_path: &Path,
    body: CompletedDownloadBody,
) -> Result<u64, String> {
    match body {
        CompletedDownloadBody::Buffered(body) => {
            let total_bytes = body.len() as u64;
            let partial_path = partial_artifact_path(artifact_path);
            write_download_artifact_async(download_root, &partial_path, &body).await?;
            finalize_download_artifact(&partial_path, artifact_path).await?;
            Ok(total_bytes)
        }
        CompletedDownloadBody::Streaming(mut response) => {
            tokio::fs::create_dir_all(download_root)
                .await
                .map_err(|error| error.to_string())?;
            let partial_path = partial_artifact_path(artifact_path);
            let mut file = tokio::fs::File::create(&partial_path)
                .await
                .map_err(|error| error.to_string())?;
            let mut total_bytes = 0_u64;
            while let Some(chunk) = response.next_chunk().await {
                total_bytes = total_bytes.saturating_add(chunk.len() as u64);
                if let Err(error) = file.write_all(&chunk).await {
                    let _ = tokio::fs::remove_file(&partial_path).await;
                    return Err(error.to_string());
                }
            }
            if let Err(error) = response.finish().await {
                let _ = tokio::fs::remove_file(&partial_path).await;
                return Err(error.to_string());
            }
            if let Err(error) = file.flush().await {
                let _ = tokio::fs::remove_file(&partial_path).await;
                return Err(error.to_string());
            }
            drop(file);
            finalize_download_artifact(&partial_path, artifact_path).await?;
            Ok(total_bytes)
        }
    }
}

async fn write_download_artifact_async(
    download_root: &str,
    artifact_path: &Path,
    body: &[u8],
) -> Result<(), String> {
    tokio::fs::create_dir_all(download_root)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::write(artifact_path, body)
        .await
        .map_err(|error| error.to_string())
}

async fn finalize_download_artifact(
    partial_path: &Path,
    artifact_path: &Path,
) -> Result<(), String> {
    tokio::fs::rename(partial_path, artifact_path)
        .await
        .map_err(|error| error.to_string())
}

fn start_download_events(prepared: &PreparedDownloadActivation) -> Vec<BackgroundProtocolEvent> {
    let suggested_filename = pending_download_filename(prepared);
    let mut events = download_will_begin_events(
        &prepared.event_route,
        &prepared.frame_id,
        &prepared.guid,
        prepared.request.url.as_str(),
        &suggested_filename,
    );
    events.extend(download_progress_events(
        &prepared.event_route,
        &prepared.guid,
        "inProgress",
        0,
        0,
        None,
    ));
    events
}

fn can_emit_early_download_start(prepared: &PreparedDownloadActivation) -> bool {
    prepared
        .suggested_filename_hint
        .as_deref()
        .and_then(non_empty_filename)
        .is_some()
}

fn pending_download_filename(prepared: &PreparedDownloadActivation) -> String {
    prepared
        .suggested_filename_hint
        .as_deref()
        .and_then(non_empty_filename)
        .map(str::to_owned)
        .or_else(|| filename_from_url(&prepared.request.url))
        .unwrap_or_else(|| "download".to_owned())
}

fn deferred_start_download_events(
    prepared: &PreparedDownloadActivation,
    response_url: &str,
    suggested_filename: &str,
) -> Vec<BackgroundProtocolEvent> {
    download_will_begin_events(
        &prepared.event_route,
        &prepared.frame_id,
        &prepared.guid,
        response_url,
        suggested_filename,
    )
}

fn denied_download_activation_events(
    event_route: &DownloadEventRoute,
    frame_id: &str,
    guid: &str,
    url: &str,
    suggested_filename: &str,
) -> Vec<BackgroundProtocolEvent> {
    let mut events =
        download_will_begin_events(event_route, frame_id, guid, url, suggested_filename);
    events.extend(download_progress_events(
        event_route,
        guid,
        "canceled",
        0,
        0,
        None,
    ));
    events
}

fn in_progress_download_events(
    prepared: &PreparedDownloadActivation,
) -> Vec<BackgroundProtocolEvent> {
    download_progress_events(
        &prepared.event_route,
        &prepared.guid,
        "inProgress",
        0,
        0,
        None,
    )
}

fn progress_download_events(
    prepared: &PreparedDownloadActivation,
    received_bytes: u64,
    total_bytes: Option<u64>,
) -> Vec<BackgroundProtocolEvent> {
    download_progress_events(
        &prepared.event_route,
        &prepared.guid,
        "inProgress",
        received_bytes,
        total_bytes.unwrap_or(0),
        None,
    )
}

fn terminal_download_events(
    prepared: &PreparedDownloadActivation,
    total_bytes: Option<u64>,
    artifact_path: Option<PathBuf>,
    canceled: bool,
) -> Vec<BackgroundProtocolEvent> {
    let total_bytes = total_bytes.unwrap_or(0);
    let file_path = artifact_path.map(|path| path.to_string_lossy().into_owned());
    download_progress_events(
        &prepared.event_route,
        &prepared.guid,
        if canceled { "canceled" } else { "completed" },
        total_bytes,
        total_bytes,
        file_path.as_deref(),
    )
}

fn build_navigation_download_will_begin_event(
    prepared: &PreparedNavigationDownload,
    suggested_filename: &str,
) -> Vec<BackgroundProtocolEvent> {
    download_will_begin_events(
        &prepared.event_route,
        &prepared.frame_id,
        &prepared.guid,
        prepared.response_url.as_str(),
        suggested_filename,
    )
}

fn build_navigation_download_in_progress_event(
    prepared: &PreparedNavigationDownload,
) -> Vec<BackgroundProtocolEvent> {
    download_progress_events(
        &prepared.event_route,
        &prepared.guid,
        "inProgress",
        0,
        0,
        None,
    )
}

fn build_navigation_download_terminal_event(
    prepared: &PreparedNavigationDownload,
    total_bytes: Option<u64>,
    artifact_path: Option<PathBuf>,
    canceled: bool,
) -> Vec<BackgroundProtocolEvent> {
    let total_bytes = total_bytes.unwrap_or(0);
    let file_path = artifact_path.map(|path| path.to_string_lossy().into_owned());
    download_progress_events(
        &prepared.event_route,
        &prepared.guid,
        if canceled { "canceled" } else { "completed" },
        total_bytes,
        total_bytes,
        file_path.as_deref(),
    )
}

fn download_will_begin_events(
    event_route: &DownloadEventRoute,
    frame_id: &str,
    guid: &str,
    url: &str,
    suggested_filename: &str,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = event_route
        .page_observers
        .iter()
        .map(|observer| {
            BackgroundProtocolEvent::page_download_will_begin(
                observer.session_id.as_deref(),
                observer.subscription_generation,
                frame_id,
                guid,
                url,
                suggested_filename,
            )
        })
        .collect::<Vec<_>>();
    for observer in &event_route.browser_observers {
        events.push(download_will_begin_event(
            observer.session_id.as_deref(),
            Some(observer.subscription_generation),
            frame_id,
            guid,
            url,
            suggested_filename,
        ));
    }
    if event_route.automation_events_enabled {
        events.push(BackgroundProtocolEvent::automation_download_will_begin(
            frame_id,
            guid,
            url,
            suggested_filename,
        ));
    }
    events
}

fn download_progress_events(
    event_route: &DownloadEventRoute,
    guid: &str,
    state: &str,
    received_bytes: u64,
    total_bytes: u64,
    file_path: Option<&str>,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = event_route
        .page_observers
        .iter()
        .map(|observer| {
            BackgroundProtocolEvent::page_download_progress(
                observer.session_id.as_deref(),
                observer.subscription_generation,
                guid,
                state,
                received_bytes,
                total_bytes,
            )
        })
        .collect::<Vec<_>>();
    for observer in &event_route.browser_observers {
        events.push(download_progress_event(
            observer.session_id.as_deref(),
            Some(observer.subscription_generation),
            guid,
            state,
            received_bytes,
            total_bytes,
            file_path,
        ));
    }
    if event_route.automation_events_enabled {
        events.push(BackgroundProtocolEvent::automation_download_progress(
            guid,
            state,
            received_bytes,
            total_bytes,
            file_path,
        ));
    }
    events
}

fn download_will_begin_event(
    session_id: Option<&str>,
    subscription_generation: Option<u64>,
    frame_id: &str,
    guid: &str,
    url: &str,
    suggested_filename: &str,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::browser_download_will_begin(
        session_id,
        subscription_generation,
        frame_id,
        guid,
        url,
        suggested_filename,
    )
}

fn download_progress_event(
    session_id: Option<&str>,
    subscription_generation: Option<u64>,
    guid: &str,
    state: &str,
    received_bytes: u64,
    total_bytes: u64,
    file_path: Option<&str>,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::browser_download_progress(
        session_id,
        subscription_generation,
        guid,
        state,
        received_bytes,
        total_bytes,
        file_path,
    )
}

fn generate_download_guid() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    moli_crypto::fill_secure_random(&mut bytes)
        .map_err(|error| format!("failed to generate download GUID: {error}"))?;
    Ok(format_download_guid(bytes))
}

fn format_download_guid(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn artifact_file_name(behavior: &str, guid: &str, suggested_filename: &str) -> String {
    if DownloadBehavior::parse(behavior).is_some_and(DownloadBehavior::names_artifact_by_guid) {
        return guid.to_owned();
    }
    sanitize_filename(suggested_filename)
}

fn partial_artifact_path(artifact_path: &Path) -> PathBuf {
    let file_name = artifact_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    artifact_path.with_file_name(format!("{file_name}.crdownload"))
}

fn content_length_from_headers(headers: &[(String, String)]) -> Option<u64> {
    headers
        .iter()
        .find(|(name, _)| header_name_is(name, &HeaderName::from_static("content-length")))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
}

pub(crate) fn response_headers_indicate_download(headers: &[(String, String)]) -> bool {
    response_headers_indicate_attachment_download(headers)
}

fn filename_from_headers(headers: &[(String, String)]) -> Option<String> {
    for (name, value) in headers {
        if !header_name_is(name, &HeaderName::from_static("content-disposition")) {
            continue;
        }
        if let Some(filename) = filename_from_content_disposition(value) {
            return Some(filename);
        }
    }
    None
}

fn filename_from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    let mut saw_extended = false;

    // A `;` inside a quoted string does not start a new parameter. Splitting on
    // every `;` let text inside a quoted `filename` be read as a parameter of
    // its own, so a site that echoes an attacker-supplied name into the header
    // could smuggle a `filename*` and choose the extension the file is saved
    // under.
    for part in split_outside_quoted_strings(value, ';').into_iter().skip(1) {
        let part = part.trim();
        if let Some(raw) = strip_parameter_name(part, "filename*") {
            saw_extended = true;
            extended = decode_extended_filename(raw);
        } else if let Some(raw) = strip_parameter_name(part, "filename") {
            plain = Some(unquote_parameter_value(raw.trim()).into_owned());
        }
    }

    if extended.is_some() {
        return extended;
    }
    if saw_extended && plain.is_none() {
        return None;
    }

    plain
        .as_deref()
        .and_then(non_empty_filename)
        .map(sanitize_filename)
}

/// Strips a case-insensitive `name=` prefix from one parameter.
fn strip_parameter_name<'a>(part: &'a str, name: &str) -> Option<&'a str> {
    let rest = part
        .get(..name.len())?
        .eq_ignore_ascii_case(name)
        .then(|| &part[name.len()..])?;
    rest.trim_start().strip_prefix('=')
}

fn decode_extended_filename(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_matches('"');
    let mut parts = raw.splitn(3, '\'');
    let charset = parts.next().unwrap_or_default();
    let _language = parts.next();
    let encoded = parts.next().unwrap_or(raw);
    let decoded = percent_decode_bytes(encoded)?;

    let filename = if charset.is_empty() || charset.eq_ignore_ascii_case("utf-8") {
        String::from_utf8(decoded).ok()?
    } else if charset.eq_ignore_ascii_case("iso-8859-1")
        || charset.eq_ignore_ascii_case("latin1")
        || charset.eq_ignore_ascii_case("latin-1")
    {
        decoded.into_iter().map(char::from).collect()
    } else {
        return None;
    };

    non_empty_filename(&filename).map(sanitize_filename)
}

fn percent_decode_bytes(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return None;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Some(percent_encoding::percent_decode(bytes).collect())
}

fn filename_from_url(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .and_then(non_empty_filename)
        .map(sanitize_filename)
}

fn non_empty_filename(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn sanitize_filename(value: &str) -> String {
    let component = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(non_empty_filename)
        .unwrap_or("download");
    let sanitized = sanitize_filename::sanitize_with_options(
        component,
        Options {
            windows: true,
            truncate: true,
            replacement: "",
        },
    );
    non_empty_filename(&sanitized)
        .unwrap_or("download")
        .to_owned()
}

fn header_name_is(candidate: &str, expected: &HeaderName) -> bool {
    HeaderName::from_bytes(candidate.as_bytes()).is_ok_and(|candidate| candidate == *expected)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use moli_fetch::FetchCancelHandle;

    use crate::{
        conn::{BackgroundProtocolEvent, CdpConnection},
        devtools_runtime::AutomationEvent,
    };

    use super::{
        BrowserDownloadObserver, DownloadBehavior, DownloadEventRoute, DownloadLifecycle,
        DownloadRecord, OpenDownloadArtifactOutcome, PageDownloadObserver, SharedDownloadRegistry,
        artifact_file_name, content_length_from_headers, download_progress_event,
        download_progress_events, download_will_begin_event, download_will_begin_events,
        filename_from_content_disposition, format_download_guid, generate_download_guid,
        partial_artifact_path, response_headers_indicate_download, sanitize_filename,
    };

    #[test]
    fn download_behavior_parses_cdp_tokens_with_existing_case_sensitivity() {
        assert_eq!(
            DownloadBehavior::parse("default"),
            Some(DownloadBehavior::Default)
        );
        assert_eq!(
            DownloadBehavior::parse("deny"),
            Some(DownloadBehavior::Deny)
        );
        assert_eq!(
            DownloadBehavior::parse("allow"),
            Some(DownloadBehavior::Allow)
        );
        assert_eq!(
            DownloadBehavior::parse("allowAndName"),
            Some(DownloadBehavior::AllowAndName)
        );
        assert_eq!(DownloadBehavior::parse("allowandname"), None);
        assert_eq!(DownloadBehavior::parse("unknown"), None);
    }

    #[test]
    fn download_behavior_helpers_preserve_allow_and_naming_policy() {
        assert!(!DownloadBehavior::Default.allows_download());
        assert!(!DownloadBehavior::Deny.allows_download());
        assert!(DownloadBehavior::Allow.allows_download());
        assert!(DownloadBehavior::AllowAndName.allows_download());

        assert!(!DownloadBehavior::Allow.names_artifact_by_guid());
        assert!(DownloadBehavior::AllowAndName.names_artifact_by_guid());
    }

    #[test]
    fn download_guid_uses_random_uuid_v4_shape() {
        let guid = generate_download_guid().expect("secure random download GUID");
        assert_eq!(guid.len(), 36);
        assert_eq!(
            guid.chars()
                .enumerate()
                .filter_map(|(index, character)| (character == '-').then_some(index))
                .collect::<Vec<_>>(),
            [8, 13, 18, 23]
        );
        assert_eq!(guid.as_bytes()[14], b'4');
        assert!(matches!(guid.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert!(
            guid.chars()
                .filter(|character| *character != '-')
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
    }

    #[test]
    fn download_guid_formatter_sets_uuid_version_and_variant_bits() {
        assert_eq!(
            format_download_guid([0; 16]),
            "00000000-0000-4000-8000-000000000000"
        );
        assert_eq!(
            format_download_guid([u8::MAX; 16]),
            "ffffffff-ffff-4fff-bfff-ffffffffffff"
        );
    }

    #[test]
    fn browser_download_will_begin_event_is_protocol_only() {
        let event = download_will_begin_event(
            None,
            None,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Browser.downloadWillBegin");
        assert_eq!(message["params"]["frameId"], "FRAME-download");
        assert_eq!(message["params"]["guid"], "GUID-download");
        assert_eq!(message["params"]["url"], "https://example.test/report.txt");
        assert_eq!(message["params"]["suggestedFilename"], "report.txt");
        assert_eq!(automation_event, None);
    }

    #[test]
    fn browser_download_progress_event_is_protocol_only() {
        let event = download_progress_event(
            None,
            None,
            "GUID-download",
            "completed",
            512,
            512,
            Some("/tmp/report.txt"),
        );

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Browser.downloadProgress");
        assert_eq!(message["params"]["guid"], "GUID-download");
        assert_eq!(message["params"]["state"], "completed");
        assert_eq!(message["params"]["receivedBytes"], 512);
        assert_eq!(message["params"]["totalBytes"], 512);
        assert_eq!(message["params"]["filePath"], "/tmp/report.txt");
        assert_eq!(automation_event, None);
    }

    #[test]
    fn automation_download_events_are_sidecar_only() {
        let will_begin = BackgroundProtocolEvent::automation_download_will_begin(
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        assert!(!will_begin.has_protocol_wire_message());
        assert_eq!(
            will_begin.download_will_begin_frame_id(),
            Some("FRAME-download")
        );
        let (_, automation_event) = will_begin.into_parts();
        let Some(AutomationEvent::BrowserDownloadWillBegin(event)) = automation_event else {
            panic!("expected typed Browser.downloadWillBegin automation event");
        };
        assert_eq!(event.frame_id.as_str(), "FRAME-download");
        assert_eq!(event.guid, "GUID-download");
        assert_eq!(event.url, "https://example.test/report.txt");
        assert_eq!(event.suggested_filename, "report.txt");

        let progress = BackgroundProtocolEvent::automation_download_progress(
            "GUID-download",
            "completed",
            512,
            512,
            Some("/tmp/report.txt"),
        );
        assert!(!progress.has_protocol_wire_message());
        let (_, automation_event) = progress.into_parts();
        let Some(AutomationEvent::BrowserDownloadProgress(event)) = automation_event else {
            panic!("expected typed Browser.downloadProgress automation event");
        };
        assert_eq!(event.guid, "GUID-download");
        assert_eq!(event.state, "completed");
        assert_eq!(event.received_bytes, 512);
        assert_eq!(event.total_bytes, 512);
        assert_eq!(event.file_path.as_deref(), Some("/tmp/report.txt"));
    }

    #[test]
    fn download_events_fan_out_to_page_and_browser_before_automation() {
        let route = DownloadEventRoute {
            browser_observers: vec![
                BrowserDownloadObserver {
                    session_id: Some("SID-browser-a".to_owned()),
                    subscription_generation: 8,
                },
                BrowserDownloadObserver {
                    session_id: Some("SID-browser-b".to_owned()),
                    subscription_generation: 9,
                },
            ],
            automation_events_enabled: true,
            page_observers: vec![PageDownloadObserver {
                session_id: Some("SID-page".to_owned()),
                subscription_generation: 7,
            }],
        };

        let events = download_will_begin_events(
            &route,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].0["method"], "Page.downloadWillBegin");
        assert_eq!(parts[0].0["sessionId"], "SID-page");
        assert_eq!(parts[0].0["params"]["frameId"], "FRAME-download");
        assert_eq!(parts[0].1, None);
        assert_eq!(parts[1].0["method"], "Browser.downloadWillBegin");
        assert_eq!(parts[1].0["sessionId"], "SID-browser-a");
        assert_eq!(parts[1].1, None);
        assert_eq!(parts[2].0["method"], "Browser.downloadWillBegin");
        assert_eq!(parts[2].0["sessionId"], "SID-browser-b");
        assert_eq!(parts[2].1, None);
        assert_eq!(parts[3].0["method"], "Moli.automationOnly");
        assert!(matches!(
            parts[3].1,
            Some(AutomationEvent::BrowserDownloadWillBegin(_))
        ));
    }

    #[test]
    fn stale_browser_observer_does_not_suppress_automation_download_event() {
        let mut conn = CdpConnection::new();
        conn.download_behavior
            .set_browser_events_enabled_for_session(Some("SID-browser"), true);
        let generation = conn.download_behavior.browser_event_observers()[0].1;
        let route = DownloadEventRoute {
            browser_observers: vec![BrowserDownloadObserver {
                session_id: Some("SID-browser".to_owned()),
                subscription_generation: generation,
            }],
            automation_events_enabled: true,
            page_observers: Vec::new(),
        };
        let events = download_will_begin_events(
            &route,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        assert_eq!(events.len(), 2);
        assert!(events[0].route_is_current(&conn));
        assert!(events[1].route_is_current(&conn));

        conn.download_behavior
            .set_browser_events_enabled_for_session(Some("SID-browser"), false);

        assert!(!events[0].route_is_current(&conn));
        assert!(
            events[1].route_is_current(&conn),
            "a stale CDP observer must not suppress internal WebDriver/BiDi delivery"
        );
        assert_eq!(
            events[1].download_will_begin_frame_id(),
            Some("FRAME-download")
        );
    }

    #[test]
    fn page_download_progress_omits_browser_only_file_path() {
        let route = DownloadEventRoute {
            browser_observers: vec![BrowserDownloadObserver {
                session_id: Some("SID-browser".to_owned()),
                subscription_generation: 12,
            }],
            automation_events_enabled: false,
            page_observers: vec![PageDownloadObserver {
                session_id: Some("SID-page".to_owned()),
                subscription_generation: 11,
            }],
        };

        let events = download_progress_events(
            &route,
            "GUID-download",
            "completed",
            512,
            512,
            Some("/tmp/report.txt"),
        );
        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(parts[0].0["method"], "Page.downloadProgress");
        assert!(
            parts[0].0["params"].get("filePath").is_none(),
            "Page.downloadProgress does not expose Browser.downloadProgress.filePath"
        );
        assert_eq!(parts[0].1, None);
        assert_eq!(parts[1].0["method"], "Browser.downloadProgress");
        assert_eq!(parts[1].0["sessionId"], "SID-browser");
        assert_eq!(parts[1].0["params"]["filePath"], "/tmp/report.txt");
        assert_eq!(parts[1].1, None);
    }

    #[test]
    fn page_download_events_do_not_depend_on_browser_events_enabled() {
        let route = DownloadEventRoute {
            browser_observers: Vec::new(),
            automation_events_enabled: false,
            page_observers: vec![PageDownloadObserver {
                session_id: Some("SID-page".to_owned()),
                subscription_generation: 13,
            }],
        };

        let events = download_will_begin_events(
            &route,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        assert_eq!(events.len(), 1);
        let message = events
            .into_iter()
            .next()
            .expect("Page observer should receive the download event")
            .into_protocol_message();
        assert_eq!(message["method"], "Page.downloadWillBegin");
        assert_eq!(message["sessionId"], "SID-page");
    }

    #[test]
    fn artifact_file_name_uses_guid_only_for_allow_and_name_behavior() {
        assert_eq!(
            artifact_file_name("allowAndName", "GUID-1", "../report.txt"),
            "GUID-1"
        );
        assert_eq!(
            artifact_file_name("allow", "GUID-1", "../report.txt"),
            "report.txt"
        );
        assert_eq!(
            artifact_file_name("unknown", "GUID-1", "../report.txt"),
            "report.txt"
        );
    }

    #[test]
    fn partial_artifact_path_appends_crdownload_to_final_name() {
        assert_eq!(
            partial_artifact_path(&PathBuf::from("/tmp/report.txt")),
            PathBuf::from("/tmp/report.txt.crdownload")
        );
    }

    #[tokio::test]
    async fn finalize_download_artifact_preserves_existing_artifact_when_rename_fails() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "moli-cdp-download-finalize-{}-{nonce}",
            std::process::id()
        ));
        tokio::fs::create_dir_all(&root)
            .await
            .expect("test temp dir should be created");
        let artifact_path = root.join("artifact.bin");
        let missing_partial_path = root.join("missing.crdownload");

        tokio::fs::write(&artifact_path, b"previous artifact")
            .await
            .expect("existing artifact should be written");

        let result = super::finalize_download_artifact(&missing_partial_path, &artifact_path).await;

        assert!(result.is_err());
        assert_eq!(
            tokio::fs::read(&artifact_path)
                .await
                .expect("existing artifact should remain readable"),
            b"previous artifact"
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[test]
    fn content_length_from_headers_parses_case_insensitive_header_name() {
        assert_eq!(
            content_length_from_headers(&[("Content-Length".to_owned(), "42".to_owned())]),
            Some(42)
        );
        assert_eq!(
            content_length_from_headers(&[("content-length".to_owned(), "bad".to_owned())]),
            None
        );
    }

    #[test]
    fn content_disposition_ignores_filename_star_inside_a_quoted_filename() {
        // RFC 6266: the whole quoted string is the `filename` value, and there
        // is no `filename*` parameter here at all. Reading the inner text as
        // one let a site that echoes an attacker-supplied name into the header
        // choose the extension the file is saved under.
        let filename = filename_from_content_disposition(
            "attachment; filename=\"a;filename*=UTF-8''evil.exe\"",
        );

        // The saved name is the quoted string itself, with `*` removed by
        // the Windows-safe sanitizer rather than by the parameter scan.
        assert_ne!(filename.as_deref(), Some("evil.exe"));
        assert_eq!(filename.as_deref(), Some("a;filename=UTF-8''evil.exe"));
    }

    #[test]
    fn content_disposition_still_reads_a_real_filename_star_after_a_quoted_filename() {
        let filename = filename_from_content_disposition(
            "attachment; filename=\"plain;name.txt\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt",
        );

        assert_eq!(filename.as_deref(), Some("中文.txt"));
    }

    #[test]
    fn content_disposition_prefers_filename_star_when_present() {
        let filename = filename_from_content_disposition(
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt",
        );

        assert_eq!(filename.as_deref(), Some("中文.txt"));
    }

    #[test]
    fn content_disposition_falls_back_to_plain_filename_when_extended_decode_fails() {
        let filename = filename_from_content_disposition(
            "attachment; filename=\"fallback.txt\"; filename*=UTF-8''%ZZbroken",
        );

        assert_eq!(filename.as_deref(), Some("fallback.txt"));
    }

    #[test]
    fn content_disposition_rejects_invalid_extended_filename_without_plain_fallback() {
        let filename = filename_from_content_disposition("attachment; filename*=UTF-8''%ZZbroken");

        assert_eq!(filename, None);
    }

    #[test]
    fn response_headers_indicate_download_uses_web_mime_attachment_helper() {
        assert!(response_headers_indicate_download(&[(
            "Content-Disposition".to_owned(),
            "attachment; filename=\"report.txt\"".to_owned(),
        )]));
        assert!(!response_headers_indicate_download(&[(
            "Content-Disposition".to_owned(),
            "inline; filename=\"report.txt\"".to_owned(),
        )]));
    }

    #[test]
    fn sanitize_filename_strips_path_components() {
        assert_eq!(sanitize_filename("../nested/report.txt"), "report.txt");
    }

    #[test]
    fn sanitize_filename_removes_reserved_filename_characters() {
        assert_eq!(sanitize_filename("report?.txt"), "report.txt");
        assert_eq!(sanitize_filename("CON"), "download");
    }

    #[test]
    fn cancel_reports_completed_download_as_already_terminal() {
        let registry = SharedDownloadRegistry::default();
        registry.insert_active("DOWNLOAD-1".to_owned(), FetchCancelHandle::new());
        registry.mark_completed("DOWNLOAD-1", PathBuf::from("/tmp/download"));

        assert!(matches!(
            registry.cancel("DOWNLOAD-1"),
            super::CancelDownloadOutcome::AlreadyTerminal
        ));
    }

    #[test]
    fn cancel_reports_canceled_download_as_already_terminal() {
        let registry = SharedDownloadRegistry::default();
        registry.insert_active("DOWNLOAD-2".to_owned(), FetchCancelHandle::new());
        registry.mark_canceled("DOWNLOAD-2");

        assert!(matches!(
            registry.cancel("DOWNLOAD-2"),
            super::CancelDownloadOutcome::AlreadyTerminal
        ));
    }

    #[test]
    fn cancel_does_not_mutate_completed_download_record() {
        let registry = SharedDownloadRegistry::default();
        registry.with_mut(|downloads| {
            downloads.insert(
                "DOWNLOAD-3".to_owned(),
                DownloadRecord {
                    state: DownloadLifecycle::Completed,
                    artifact_path: Some(PathBuf::from("/tmp/download")),
                },
            );
        });

        let _ = registry.cancel("DOWNLOAD-3");

        registry.with_mut(|downloads| {
            let record = downloads
                .get("DOWNLOAD-3")
                .expect("completed download should remain present");
            assert!(matches!(record.state, DownloadLifecycle::Completed));
            assert_eq!(record.artifact_path, Some(PathBuf::from("/tmp/download")));
        });
    }

    #[test]
    fn open_artifact_reports_active_download_as_in_progress() {
        let registry = SharedDownloadRegistry::default();
        registry.insert_active("DOWNLOAD-5".to_owned(), FetchCancelHandle::new());

        assert!(matches!(
            registry.open_artifact("DOWNLOAD-5"),
            OpenDownloadArtifactOutcome::InProgress
        ));
    }

    #[test]
    fn open_artifact_returns_completed_artifact_path() {
        let registry = SharedDownloadRegistry::default();
        let artifact_path = PathBuf::from("/tmp/download");
        registry.with_mut(|downloads| {
            downloads.insert(
                "DOWNLOAD-6".to_owned(),
                DownloadRecord {
                    state: DownloadLifecycle::Completed,
                    artifact_path: Some(artifact_path.clone()),
                },
            );
        });

        assert!(matches!(
            registry.open_artifact("DOWNLOAD-6"),
            OpenDownloadArtifactOutcome::Ready(path) if path == artifact_path
        ));
    }

    #[test]
    fn connection_cancel_download_rejects_already_terminal_guid() {
        let conn = CdpConnection::new();
        conn.download_registry
            .insert_active("DOWNLOAD-4".to_owned(), FetchCancelHandle::new());
        conn.download_registry
            .mark_completed("DOWNLOAD-4", PathBuf::from("/tmp/download"));

        assert_eq!(
            conn.cancel_download("DOWNLOAD-4"),
            Err("Download item is no longer active".to_owned())
        );
    }
}
