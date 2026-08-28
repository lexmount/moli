use std::{
    collections::HashMap,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::{BrowserConfig, storage_partition::StoragePartitionState};
use crate::{
    RendererOutputItem, RendererOutputResidenceIdentity, RendererOutputTransportMessage,
    RendererOutputTransportReceiver, RendererOwnerAction,
    network::{ResourceRequestClient, new_shared_web_storage_store},
    page::{
        DocumentStartScript, Page, RendererPendingPopupActivation,
        RendererPopupNewTargetDisposition, RendererRemoteWindowProxyCommand,
        RendererServiceWorkerClientsOpenWindowContinuation, RendererTopLevelNavigationRequest,
    },
    renderer::{JsRuntime, RendererOwnerCommand, RendererPageCommand, RendererPageReply},
};

const ABOUT_BLANK_DOCUMENT_HTML: &str = "<!doctype html><html><head></head><body></body></html>";
const OWNER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct StandalonePageResidence {
    owner_local_host_id: moli_renderer_v8::RendererOwnerLocalHostId,
    page_id: moli_renderer_v8::PageId,
}

impl StandalonePageResidence {
    fn new(
        owner_local_host_id: moli_renderer_v8::RendererOwnerLocalHostId,
        page_id: moli_renderer_v8::PageId,
    ) -> Self {
        Self {
            owner_local_host_id,
            page_id,
        }
    }

    fn from_output_residence(residence: RendererOutputResidenceIdentity) -> Option<Self> {
        match residence {
            RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            } => Some(Self::new(owner_local_host_id, page_id)),
            RendererOutputResidenceIdentity::SharedWorker { .. }
            | RendererOutputResidenceIdentity::ServiceWorker { .. } => None,
        }
    }
}

#[derive(Clone)]
struct StandaloneAuxiliaryPageEnvironment {
    js_runtime: JsRuntime,
    loader: ResourceRequestClient,
    partition: Arc<StoragePartitionState>,
    document_start_scripts: Vec<DocumentStartScript>,
    wpt_extensions_enabled: bool,
}

impl StandaloneAuxiliaryPageEnvironment {
    fn new(
        js_runtime: JsRuntime,
        loader: ResourceRequestClient,
        partition: Arc<StoragePartitionState>,
        config: &BrowserConfig,
    ) -> Self {
        Self {
            js_runtime,
            loader,
            partition,
            document_start_scripts: config
                .document_start_scripts()
                .iter()
                .cloned()
                .map(|source| DocumentStartScript {
                    registry_key: None,
                    source,
                    world_name: None,
                    has_bidi_channel_argument: false,
                    browser_internal: false,
                    bidi_channel_handoffs: Vec::new(),
                })
                .collect(),
            wpt_extensions_enabled: config.wpt_extensions_enabled(),
        }
    }
}

/// Browser-level owner for auxiliary Pages created by the direct Browser API.
///
/// Protocol servers already have a target scheduler that consumes renderer
/// owner actions. A direct `Browser::fetch*` call has no protocol target, so it
/// needs the same ownership boundary without manufacturing a DevTools model.
/// The owner thread hosts a Tokio `LocalSet`: each auxiliary actor can retain
/// its non-Send `Page` handle while the ingress continues admitting nested
/// popup, focus, close, and RemoteWindowProxy actions from every Page stream.
pub(super) struct StandaloneAuxiliaryPageOwner {
    shutdown_tx: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl StandaloneAuxiliaryPageOwner {
    pub(super) fn start(
        js_runtime: JsRuntime,
        loader: ResourceRequestClient,
        partition: Arc<StoragePartitionState>,
        config: &BrowserConfig,
    ) -> Result<Self> {
        let (output_tx, output_rx) = crate::renderer_output_transport_channel();
        js_runtime.set_renderer_output_transport_sender(output_tx);
        let environment =
            StandaloneAuxiliaryPageEnvironment::new(js_runtime.clone(), loader, partition, config);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let owner_id = js_runtime.renderer_owner_id_for_diagnostics();
        let thread = thread::Builder::new()
            .name(format!("moli-standalone-page-owner-{owner_id}"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_tx.send(Err(error.to_string()));
                        tracing::error!(
                            error = %error,
                            owner_id,
                            "failed to build standalone auxiliary Page owner runtime"
                        );
                        return;
                    }
                };
                let local = tokio::task::LocalSet::new();
                if startup_tx.send(Ok(())).is_err() {
                    return;
                }
                local.block_on(
                    &runtime,
                    run_standalone_auxiliary_owner(environment, output_rx, shutdown_rx),
                );
            })
            .context("failed to spawn standalone auxiliary Page owner thread")?;
        match startup_rx
            .recv()
            .context("standalone auxiliary Page owner exited during startup")?
        {
            Ok(()) => {}
            Err(error) => {
                let _ = thread.join();
                return Err(anyhow!(
                    "failed to initialize standalone auxiliary Page owner runtime: {error}"
                ));
            }
        }
        Ok(Self {
            shutdown_tx: Some(shutdown_tx),
            thread: Some(thread),
        })
    }

    pub(super) fn shutdown_and_join(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(thread) = self.thread.take()
            && let Err(payload) = thread.join()
        {
            tracing::error!(
                panic = ?payload,
                "standalone auxiliary Page owner thread panicked"
            );
        }
    }
}

enum StandaloneAuxiliaryPageCommand {
    Navigate {
        request: RendererTopLevelNavigationRequest,
        navigation_history_entry_seed: Option<Box<moli_page_types::NavigationHistoryEntrySeed>>,
        navigation_referrer: Option<String>,
        continuation: Option<RendererServiceWorkerClientsOpenWindowContinuation>,
    },
    RemoteWindowProxy(RendererRemoteWindowProxyCommand),
    SetFocus {
        active: bool,
        focused: bool,
    },
    CloseAccepted(crate::RendererTopLevelCloseSource),
    CloseNetworkDrained(crate::RendererTopLevelCloseSource),
    CloseUnloadAcknowledged,
    Shutdown,
}

struct StandaloneAuxiliaryPageInit {
    residence: StandalonePageResidence,
    page_reservation: moli_renderer_v8::RendererPageReservationToken,
    navigation_initiator_url: Option<Url>,
    initial_document_referrer: Option<String>,
    initial_top_level_browsing_context_name: Option<String>,
    auxiliary_browsing_context_policy:
        Option<moli_renderer_v8::RendererAuxiliaryBrowsingContextPolicy>,
    session_storage_store: Option<crate::network::SharedWebStorageStore>,
    initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
    initial_navigation: Option<StandaloneAuxiliaryPageCommand>,
}

async fn run_standalone_auxiliary_owner(
    environment: StandaloneAuxiliaryPageEnvironment,
    mut output_rx: RendererOutputTransportReceiver,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let (retired_tx, mut retired_rx) = mpsc::unbounded_channel();
    let mut routes = HashMap::<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >::new();

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => break,
            Some(residence) = retired_rx.recv() => {
                routes.remove(&residence);
            }
            message = output_rx.recv() => {
                let Some(message) = message else { break };
                admit_renderer_output_message(
                    &environment,
                    &mut routes,
                    &retired_tx,
                    message,
                );
            }
        }
    }

    for route in routes.values() {
        let _ = route.send(StandaloneAuxiliaryPageCommand::Shutdown);
    }
    let wait_for_retirement = async {
        while !routes.is_empty() {
            let Some(residence) = retired_rx.recv().await else {
                break;
            };
            routes.remove(&residence);
        }
    };
    let _ = tokio::time::timeout(OWNER_SHUTDOWN_TIMEOUT, wait_for_retirement).await;
}

fn admit_renderer_output_message(
    environment: &StandaloneAuxiliaryPageEnvironment,
    routes: &mut HashMap<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >,
    retired_tx: &mpsc::UnboundedSender<StandalonePageResidence>,
    message: RendererOutputTransportMessage,
) {
    let RendererOutputTransportMessage::Publication(publication) = message else {
        return;
    };
    let source_residence =
        StandalonePageResidence::from_output_residence(publication.cursor().stream().residence());
    for record in publication.into_records() {
        let (_, item) = record.into_parts();
        let RendererOutputItem::OwnerAction(action) = item else {
            continue;
        };
        admit_renderer_owner_action(environment, routes, retired_tx, source_residence, action);
    }
}

fn admit_renderer_owner_action(
    environment: &StandaloneAuxiliaryPageEnvironment,
    routes: &mut HashMap<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >,
    retired_tx: &mpsc::UnboundedSender<StandalonePageResidence>,
    source_residence: Option<StandalonePageResidence>,
    action: RendererOwnerAction,
) {
    match action {
        RendererOwnerAction::Popup(activation) => {
            tracing::trace!(
                target: "moli_standalone_auxiliary",
                ?source_residence,
                url = activation.url(),
                target_name = activation.target_name(),
                "admitting standalone auxiliary popup action"
            );
            admit_popup_activation(environment, routes, retired_tx, activation);
        }
        RendererOwnerAction::RemoteWindowProxy(command) => {
            let target = command.target_page();
            let residence =
                StandalonePageResidence::new(target.owner_local_host_id(), target.page_id());
            tracing::trace!(
                target: "moli_standalone_auxiliary",
                ?source_residence,
                ?residence,
                locally_owned = routes.contains_key(&residence),
                "routing standalone RemoteWindowProxy action"
            );
            if routes.contains_key(&residence) {
                send_to_route(
                    routes,
                    residence,
                    StandaloneAuxiliaryPageCommand::RemoteWindowProxy(command),
                    "RemoteWindowProxy",
                );
            } else {
                dispatch_external_page_owner_command(
                    environment,
                    residence,
                    RendererPageCommand::DispatchRemoteWindowProxyCommand(command),
                    "RemoteWindowProxy",
                );
            }
        }
        RendererOwnerAction::TopLevelFocus(target) => {
            let focused =
                StandalonePageResidence::new(target.owner_local_host_id(), target.page_id());
            for (residence, route) in routes.iter() {
                let is_focused = *residence == focused;
                let _ = route.send(StandaloneAuxiliaryPageCommand::SetFocus {
                    active: is_focused,
                    focused: is_focused,
                });
            }
            if !routes.contains_key(&focused) {
                dispatch_external_page_owner_command(
                    environment,
                    focused,
                    RendererPageCommand::SetTopLevelPageFocus {
                        active: true,
                        focused: true,
                    },
                    "focus externally held target Page",
                );
            }
            if let Some(source_residence) = source_residence
                && source_residence != focused
                && !routes.contains_key(&source_residence)
            {
                dispatch_external_page_owner_command(
                    environment,
                    source_residence,
                    RendererPageCommand::SetTopLevelPageFocus {
                        active: false,
                        focused: false,
                    },
                    "unfocus externally held source Page",
                );
            }
        }
        RendererOwnerAction::TopLevelClose(source) => {
            send_to_source_route(
                routes,
                source_residence,
                StandaloneAuxiliaryPageCommand::CloseAccepted(source),
                "accepted close",
            );
        }
        RendererOwnerAction::TopLevelCloseNetworkDrained(source) => {
            send_to_source_route(
                routes,
                source_residence,
                StandaloneAuxiliaryPageCommand::CloseNetworkDrained(source),
                "close network-drained barrier",
            );
        }
        RendererOwnerAction::TopLevelCloseUnloadAck(_) => {
            send_to_source_route(
                routes,
                source_residence,
                StandaloneAuxiliaryPageCommand::CloseUnloadAcknowledged,
                "close unload ACK",
            );
        }
        RendererOwnerAction::TopLevelLocationNavigation(navigation) => {
            let Some(source_residence) = source_residence else {
                tracing::debug!("standalone top-level location navigation had no Page residence");
                return;
            };
            route_navigation_command(
                environment,
                routes,
                source_residence,
                StandaloneAuxiliaryPageCommand::Navigate {
                    request: navigation.request().clone(),
                    navigation_history_entry_seed: navigation
                        .navigation_history_entry_seed()
                        .cloned()
                        .map(Box::new),
                    navigation_referrer: None,
                    continuation: None,
                },
                "top-level location navigation",
            );
        }
        RendererOwnerAction::TopLevelHistoryTraversal(traversal) => {
            let Some(source_residence) = source_residence else {
                tracing::debug!("standalone top-level history traversal had no Page residence");
                return;
            };
            let delta = traversal.delta();
            let Some((destination_url, navigation_history_entry_seed)) =
                traversal.into_cross_document_destination()
            else {
                tracing::trace!(
                    ?source_residence,
                    delta,
                    "ignoring standalone history traversal without an exact cross-Document destination"
                );
                return;
            };
            route_navigation_command(
                environment,
                routes,
                source_residence,
                StandaloneAuxiliaryPageCommand::Navigate {
                    request: RendererTopLevelNavigationRequest::get(destination_url),
                    navigation_history_entry_seed: Some(Box::new(navigation_history_entry_seed)),
                    navigation_referrer: None,
                    continuation: None,
                },
                "top-level history traversal",
            );
        }
        RendererOwnerAction::Download(_)
        | RendererOwnerAction::FileChooser(_)
        | RendererOwnerAction::JavaScriptDialog(_)
        | RendererOwnerAction::ChildFrameTree { .. }
        | RendererOwnerAction::ChildFrameDocumentOpened { .. }
        | RendererOwnerAction::ChildFrameDocumentNetwork { .. }
        | RendererOwnerAction::ChildFrameLoad { .. }
        | RendererOwnerAction::SameDocumentNavigation(_)
        | RendererOwnerAction::SubresourceFetchPause { .. }
        | RendererOwnerAction::SubresourceContinue { .. }
        | RendererOwnerAction::DetachedParserScriptFetchPause { .. }
        | RendererOwnerAction::SharedWorkerTargetLifecycle(_)
        | RendererOwnerAction::ServiceWorkerTargetLifecycle(_)
        | RendererOwnerAction::DedicatedWorkerTargetLifecycle(_) => {}
    }
}

fn dispatch_external_page_owner_command(
    environment: &StandaloneAuxiliaryPageEnvironment,
    residence: StandalonePageResidence,
    command: RendererPageCommand,
    operation: &'static str,
) {
    let renderer_owner = environment.js_runtime.renderer_owner_handle();
    let pending = enqueue_external_page_owner_command(&renderer_owner, residence, command);
    tokio::task::spawn_local(async move {
        match await_external_page_owner_command(pending).await {
            Ok(output) => tracing::trace!(
                target: "moli_standalone_auxiliary",
                ?residence,
                operation,
                reply = ?std::mem::discriminant(output.completion().reply()),
                "standalone browser-owner Page dispatch completed"
            ),
            Err(error) => tracing::debug!(
                ?residence,
                operation,
                error = %error,
                "standalone browser-owner Page dispatch failed"
            ),
        }
    });
}

fn dispatch_external_navigation_command(
    environment: &StandaloneAuxiliaryPageEnvironment,
    residence: StandalonePageResidence,
    request: RendererTopLevelNavigationRequest,
    navigation_history_entry_seed: Option<Box<moli_page_types::NavigationHistoryEntrySeed>>,
    navigation_referrer: Option<String>,
    continuation: Option<RendererServiceWorkerClientsOpenWindowContinuation>,
    operation: &'static str,
) {
    let request = apply_navigation_referrer(request, navigation_referrer.as_deref());
    let renderer_owner = environment.js_runtime.renderer_owner_handle();
    let pending = enqueue_external_page_owner_command(
        &renderer_owner,
        residence,
        RendererPageCommand::FollowTopLevelNavigationInStandaloneAdapter {
            request,
            navigation_history_entry_seed,
        },
    );
    tokio::task::spawn_local(async move {
        let output = match await_external_page_owner_command(pending).await {
            Ok(output) => output,
            Err(error) => {
                if let Some(continuation) = continuation {
                    continuation.resolve_null();
                }
                tracing::debug!(
                    ?residence,
                    operation,
                    error = %error,
                    "standalone navigation of externally held Page failed"
                );
                return;
            }
        };
        match output.completion().reply() {
            RendererPageReply::Bool(true) => {
                if let Some(continuation) = continuation {
                    continuation.resolve_for_committed_page(
                        residence.page_id,
                        output.completion().page_state().service_worker_client_id,
                    );
                }
            }
            RendererPageReply::Bool(false) => {
                if let Some(continuation) = continuation {
                    continuation.resolve_null();
                }
            }
            reply => {
                if let Some(continuation) = continuation {
                    continuation.resolve_null();
                }
                tracing::warn!(
                    ?residence,
                    operation,
                    reply = ?std::mem::discriminant(reply),
                    "standalone navigation of externally held Page returned an unexpected reply"
                );
            }
        }
    });
}

fn enqueue_external_page_owner_command(
    renderer_owner: &crate::renderer::RendererOwnerHandle,
    residence: StandalonePageResidence,
    command: RendererPageCommand,
) -> Result<oneshot::Receiver<Result<crate::renderer::RendererOwnerReply>>> {
    // Owner output records are admitted synchronously in stream order. Put
    // the command on the renderer queue in that same stack; a spawned waiter
    // must not become the owner of ingress ordering.
    renderer_owner.enqueue_command_with_reply(RendererOwnerCommand::RunBrowserOwnerPageCommand {
        owner_local_host_id: residence.owner_local_host_id,
        page_id: residence.page_id,
        command,
    })
}

async fn await_external_page_owner_command(
    pending: Result<oneshot::Receiver<Result<crate::renderer::RendererOwnerReply>>>,
) -> Result<crate::page::RendererCommandTurnOutput> {
    let reply = pending?
        .await
        .map_err(|_| anyhow!("render runtime reply channel closed"))??;
    match reply {
        crate::renderer::RendererOwnerReply::AsyncPageCommandRan(output) => Ok(*output),
        _ => Err(anyhow!(
            "renderer owner returned non-page-command reply for standalone browser-owner action"
        )),
    }
}

fn admit_popup_activation(
    environment: &StandaloneAuxiliaryPageEnvironment,
    routes: &mut HashMap<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >,
    retired_tx: &mpsc::UnboundedSender<StandalonePageResidence>,
    activation: RendererPendingPopupActivation,
) {
    let (
        _source,
        _disposition,
        _popup_id,
        _requested_url,
        destination_request,
        _reports_requested_url_without_destination,
        target_name,
        navigation_referrer,
        initial_document_referrer,
        _document_referrer,
        pending_auxiliary_page,
        resolved_target_page,
        new_target_disposition,
        auxiliary_browsing_context_policy,
        service_worker_clients_open_window_continuation,
        session_storage_store,
        initial_empty_document_storage_key,
    ) = activation.into_parts();

    let navigation = destination_request.map(|request| StandaloneAuxiliaryPageCommand::Navigate {
        request,
        navigation_history_entry_seed: None,
        navigation_referrer,
        continuation: service_worker_clients_open_window_continuation,
    });

    if let Some(target) = resolved_target_page {
        let residence =
            StandalonePageResidence::new(target.owner_local_host_id(), target.page_id());
        if let Some(navigation) = navigation {
            route_navigation_command(
                environment,
                routes,
                residence,
                navigation,
                "named popup target reuse",
            );
        }
        return;
    }

    let Some(pending_auxiliary_page) = pending_auxiliary_page else {
        tracing::warn!(
            target_name,
            "standalone popup activation had neither a new Page reservation nor an exact existing target"
        );
        return;
    };
    let page_reservation = pending_auxiliary_page.page_reservation();
    let residence =
        StandalonePageResidence::new(page_reservation.local_host_id(), page_reservation.page_id());
    if routes.contains_key(&residence) {
        tracing::warn!(
            ?residence,
            "duplicate standalone auxiliary Page reservation"
        );
        return;
    }

    let navigation_initiator_url = navigation
        .as_ref()
        .and_then(|navigation| match navigation {
            StandaloneAuxiliaryPageCommand::Navigate { request, .. } => request
                .source()
                .and_then(|source| Url::parse(source.source_url()).ok()),
            _ => None,
        })
        .or_else(|| {
            initial_document_referrer
                .as_deref()
                .and_then(|raw| Url::parse(raw).ok())
        });
    let initial_top_level_browsing_context_name = new_target_disposition
        .is_some_and(RendererPopupNewTargetDisposition::carries_initial_name)
        .then_some(target_name);
    let init = StandaloneAuxiliaryPageInit {
        residence,
        page_reservation,
        navigation_initiator_url,
        initial_document_referrer,
        initial_top_level_browsing_context_name,
        auxiliary_browsing_context_policy,
        session_storage_store,
        initial_empty_document_storage_key,
        initial_navigation: navigation,
    };
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    routes.insert(residence, command_tx);
    tracing::trace!(
        target: "moli_standalone_auxiliary",
        ?residence,
        has_initial_navigation = init.initial_navigation.is_some(),
        "starting standalone auxiliary Page actor"
    );
    let environment = environment.clone();
    let retired_tx = retired_tx.clone();
    tokio::task::spawn_local(async move {
        run_auxiliary_page_actor(environment, init, command_rx, retired_tx).await;
    });
}

fn route_navigation_command(
    environment: &StandaloneAuxiliaryPageEnvironment,
    routes: &HashMap<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >,
    residence: StandalonePageResidence,
    command: StandaloneAuxiliaryPageCommand,
    operation: &'static str,
) {
    if routes.contains_key(&residence) {
        send_to_route(routes, residence, command, operation);
        return;
    }
    let StandaloneAuxiliaryPageCommand::Navigate {
        request,
        navigation_history_entry_seed,
        navigation_referrer,
        continuation,
    } = command
    else {
        unreachable!("standalone external navigation route requires a navigation command")
    };
    dispatch_external_navigation_command(
        environment,
        residence,
        request,
        navigation_history_entry_seed,
        navigation_referrer,
        continuation,
        operation,
    );
}

fn send_to_source_route(
    routes: &HashMap<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >,
    source_residence: Option<StandalonePageResidence>,
    command: StandaloneAuxiliaryPageCommand,
    operation: &'static str,
) {
    let Some(source_residence) = source_residence else {
        tracing::debug!(operation, "standalone owner action had no Page residence");
        return;
    };
    send_to_route(routes, source_residence, command, operation);
}

fn send_to_route(
    routes: &HashMap<
        StandalonePageResidence,
        mpsc::UnboundedSender<StandaloneAuxiliaryPageCommand>,
    >,
    residence: StandalonePageResidence,
    command: StandaloneAuxiliaryPageCommand,
    operation: &'static str,
) {
    let Some(route) = routes.get(&residence) else {
        tracing::debug!(
            ?residence,
            operation,
            "standalone Page route is not locally owned"
        );
        return;
    };
    if route.send(command).is_err() {
        tracing::debug!(
            ?residence,
            operation,
            "standalone Page actor already retired"
        );
    }
}

async fn run_auxiliary_page_actor(
    environment: StandaloneAuxiliaryPageEnvironment,
    mut init: StandaloneAuxiliaryPageInit,
    mut command_rx: mpsc::UnboundedReceiver<StandaloneAuxiliaryPageCommand>,
    retired_tx: mpsc::UnboundedSender<StandalonePageResidence>,
) {
    let residence = init.residence;
    let initial_navigation = init.initial_navigation.take();
    let (mut page, initially_closing) = match create_auxiliary_initial_page(&environment, init)
        .await
    {
        Ok(created) => created,
        Err(error) => {
            tracing::warn!(?residence, error = %error, "failed to adopt standalone auxiliary Page");
            let _ = retired_tx.send(residence);
            return;
        }
    };

    if let Some(initial_navigation) = initial_navigation {
        if initially_closing {
            reject_initial_navigation_for_closing_page(initial_navigation);
        } else {
            apply_auxiliary_page_command(&mut page, initial_navigation).await;
        }
    }

    while let Some(command) = command_rx.recv().await {
        if matches!(command, StandaloneAuxiliaryPageCommand::Shutdown) {
            break;
        }
        if apply_auxiliary_page_command(&mut page, command).await {
            if let Err(error) = page.close_async().await {
                tracing::debug!(
                    ?residence,
                    error = %error,
                    "failed to retire standalone auxiliary Page after unload"
                );
            }
            let _ = retired_tx.send(residence);
            return;
        }
    }

    if let Err(error) = page.close_async().await {
        tracing::debug!(?residence, error = %error, "failed to close standalone auxiliary Page");
    }
    let _ = retired_tx.send(residence);
}

fn reject_initial_navigation_for_closing_page(command: StandaloneAuxiliaryPageCommand) {
    match command {
        StandaloneAuxiliaryPageCommand::Navigate { continuation, .. } => {
            if let Some(continuation) = continuation {
                continuation.resolve_null();
            }
        }
        StandaloneAuxiliaryPageCommand::RemoteWindowProxy(_)
        | StandaloneAuxiliaryPageCommand::SetFocus { .. }
        | StandaloneAuxiliaryPageCommand::CloseAccepted(_)
        | StandaloneAuxiliaryPageCommand::CloseNetworkDrained(_)
        | StandaloneAuxiliaryPageCommand::CloseUnloadAcknowledged
        | StandaloneAuxiliaryPageCommand::Shutdown => {
            debug_assert!(false, "only navigation may be staged as an initial command");
        }
    }
}

/// Returns true when the command completed terminal Page teardown.
async fn apply_auxiliary_page_command(
    page: &mut Page,
    command: StandaloneAuxiliaryPageCommand,
) -> bool {
    match command {
        StandaloneAuxiliaryPageCommand::Navigate {
            request,
            navigation_history_entry_seed,
            navigation_referrer,
            continuation,
        } => {
            let request = apply_navigation_referrer(request, navigation_referrer.as_deref());
            let result = page
                .follow_top_level_navigation_in_standalone_adapter_async(
                    request,
                    navigation_history_entry_seed,
                )
                .await;
            match result {
                Ok(true) => {
                    if let Some(continuation) = continuation {
                        continuation.resolve_for_committed_page(
                            page.renderer_page_id(),
                            page.service_worker_client_id(),
                        );
                    }
                }
                Ok(false) => {
                    if let Some(continuation) = continuation {
                        continuation.resolve_null();
                    }
                }
                Err(error) => {
                    if let Some(continuation) = continuation {
                        continuation.resolve_null();
                    }
                    tracing::debug!(
                        page_id = page.page_id(),
                        error = %error,
                        "standalone auxiliary navigation failed"
                    );
                }
            }
        }
        StandaloneAuxiliaryPageCommand::RemoteWindowProxy(command) => {
            let result = async {
                let pending = page.start_remote_window_proxy_command(command)?;
                let completion = pending.wait().await?;
                page.finish_remote_window_proxy_command(completion)
            }
            .await;
            if let Err(error) = result {
                tracing::debug!(
                    page_id = page.page_id(),
                    error = %error,
                    "standalone RemoteWindowProxy dispatch failed"
                );
            }
        }
        StandaloneAuxiliaryPageCommand::SetFocus { active, focused } => {
            if let Err(error) = page.set_top_level_page_focus_async(active, focused).await {
                tracing::debug!(
                    page_id = page.page_id(),
                    error = %error,
                    "standalone auxiliary focus update failed"
                );
            }
        }
        StandaloneAuxiliaryPageCommand::CloseAccepted(source) => {
            if let Err(error) = page.stop_document_lifecycle_async().await {
                tracing::debug!(
                    page_id = page.page_id(),
                    error = %error,
                    "standalone auxiliary close network cancellation failed"
                );
            }
            if let Err(error) = page
                .acknowledge_browser_page_close_network_drained_async(source)
                .await
            {
                tracing::debug!(
                    page_id = page.page_id(),
                    error = %error,
                    "standalone auxiliary close network ACK failed"
                );
            }
        }
        StandaloneAuxiliaryPageCommand::CloseNetworkDrained(source) => {
            if let Err(error) = page.dispatch_browser_page_close_unload_async(source).await {
                tracing::debug!(
                    page_id = page.page_id(),
                    error = %error,
                    "standalone auxiliary unload dispatch failed"
                );
            }
        }
        StandaloneAuxiliaryPageCommand::CloseUnloadAcknowledged => {
            return true;
        }
        StandaloneAuxiliaryPageCommand::Shutdown => unreachable!("shutdown is handled by actor"),
    }
    false
}

fn apply_navigation_referrer(
    request: RendererTopLevelNavigationRequest,
    navigation_referrer: Option<&str>,
) -> RendererTopLevelNavigationRequest {
    if request.source().is_some() {
        return request;
    }
    request.with_explicit_navigation_referrer_header(navigation_referrer)
}

async fn create_auxiliary_initial_page(
    environment: &StandaloneAuxiliaryPageEnvironment,
    init: StandaloneAuxiliaryPageInit,
) -> Result<(Page, bool)> {
    let about_blank = Url::parse("about:blank").expect("about:blank must parse");
    let renderer_owner = environment.js_runtime.renderer_owner_handle();
    let session_storage = init
        .session_storage_store
        .unwrap_or_else(new_shared_web_storage_store);
    let mut request = renderer_owner.build_create_html_page_request_with_env(
        init.page_reservation,
        about_blank.clone(),
        init.navigation_initiator_url,
        false,
        0,
        200,
        vec![("content-type".to_owned(), "text/html".to_owned())],
        &environment.loader,
        moli_renderer_v8::RendererWebStorageHandles::new(
            environment.partition.web_storage_store(),
            session_storage,
        ),
        about_blank,
        ABOUT_BLANK_DOCUMENT_HTML.to_owned(),
        environment.document_start_scripts.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        None,
        false,
        false,
        1.0,
        Default::default(),
        None,
        false,
        Vec::new(),
        false,
        None,
        moli_renderer_v8::PageVmInitStage::Load,
    );
    request.initial_document_referrer = init.initial_document_referrer;
    request.initial_top_level_browsing_context_name = init.initial_top_level_browsing_context_name;
    request.auxiliary_browsing_context_policy = init.auxiliary_browsing_context_policy;
    request.top_level_storage_key = init.initial_empty_document_storage_key;
    request.indexed_db_manager = Some(environment.partition.weak_indexed_db_manager());
    request.storage_bucket_store = Some(environment.partition.storage_bucket_store());
    request.wpt_extensions_enabled = environment.wpt_extensions_enabled;
    request.top_level_navigation_dispatch =
        moli_renderer_v8::RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter;
    let reply = renderer_owner
        .dispatch_command(RendererOwnerCommand::CreateHtmlPage(Box::new(request)))
        .await
        .context("failed to build standalone auxiliary initial empty Document")?;
    let (handle, page_state, diagnostics, page_creation_artifacts, pending_download) =
        renderer_owner
            .materialize_page_created_reply_parts(reply)
            .context("failed to materialize standalone auxiliary Page")?;
    if pending_download.is_some() {
        return Err(anyhow!(
            "standalone auxiliary initial empty Document unexpectedly produced a download"
        ));
    }
    Ok((
        Page::from_attached_handle_with_creation_artifacts(
            handle,
            page_state,
            page_creation_artifacts,
        ),
        diagnostics.top_level_browsing_context_closing,
    ))
}
