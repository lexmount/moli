use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, mpsc as std_mpsc},
    thread,
};

#[cfg(any(test, feature = "test-support"))]
use std::{future::Future, pin::Pin};

use indexmap::IndexMap;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

mod activation;
mod document_lifecycle;
mod downloads;
mod javascript_dialog;
pub use activation::PendingWebContentsActivation;
mod initial_document;
mod navigation;
pub use initial_document::{BrowserCommittedInitialDocument, BrowserInitialDocumentWaiter};
mod navigation_driver;
pub use navigation_driver::{BrowserNavigationOutcome, BrowserNavigationWaiter};
mod navigation_events;
mod popup;
pub use navigation::{
    BrowserDocumentMaterialization, BrowserDocumentNavigationCommit, BrowserNavigationLoad,
    BrowserPreparedDocumentNavigation, BrowserPreparedNavigationResponse,
};

use super::{
    BrowserContext, BrowserContextId, BrowserContextStoragePartitionHandles, MainFrameSlotId,
    StoragePartitionKind, WebContentsHandle,
    web_contents::{InitialDocumentCreator, SessionStorageNamespace, WebContents},
};

const BROWSER_OWNER_STACK_SIZE: usize = 16 * 1024 * 1024;

type BrowserOperation = Box<dyn FnOnce(&mut Browser) + Send + 'static>;
type BrowserLocalOperation = Box<dyn FnOnce(&mut Browser) + 'static>;
type BrowserLocalSender = mpsc::UnboundedSender<BrowserLocalOperation>;

enum BrowserOwnerMessage {
    Execute(BrowserOperation),
    Shutdown,
}

macro_rules! forward_context_read {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty;)+) => {
        $(
            pub fn $name(&self, $($argument: $argument_type),*) -> $result {
                self.read_live(move |context| context.$name($($argument),*))
            }
        )+
    };
}

macro_rules! forward_context_update {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty;)+) => {
        $(
            pub fn $name(&self, $($argument: $argument_type),*) -> $result {
                self.update_live(move |context| context.$name($($argument),*))
            }
        )+
    };
}

macro_rules! forward_context_try_read {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty;)+) => {
        $(
            pub fn $name(&self, $($argument: $argument_type),*) -> Result<$result, String> {
                self.try_read(move |context| context.$name($($argument),*))
            }
        )+
    };
}

macro_rules! forward_context_try_update {
    ($(fn $name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty;)+) => {
        $(
            pub fn $name(&self, $($argument: $argument_type),*) -> Result<$result, String> {
                self.try_update(move |context| context.$name($($argument),*))
            }
        )+
    };
}

/// The process-local Browser aggregate.
///
/// It is intentionally neither cloneable nor exposed through shared mutable
/// access. Only the Browser owner sequence constructs and mutates it.
struct Browser {
    contexts: IndexMap<BrowserContextId, BrowserContext>,
    permission_defaults: super::PermissionDefaults,
    download_policy: super::DownloadPolicy,
    navigation_work: navigation::NavigationWorkRegistry,
    popup_admissions: popup::PopupAdmissions,
    document_decision_provider: Option<tokio::sync::watch::Receiver<()>>,
    local_sender: BrowserLocalSender,
    events: super::events::BrowserEventStream,
}

impl Browser {
    fn new(local_sender: BrowserLocalSender) -> Self {
        Self {
            contexts: IndexMap::new(),
            permission_defaults: super::PermissionDefaults::default(),
            download_policy: super::DownloadPolicy::default(),
            navigation_work: navigation::NavigationWorkRegistry::default(),
            popup_admissions: popup::PopupAdmissions::default(),
            document_decision_provider: None,
            local_sender,
            events: super::events::BrowserEventStream::default(),
        }
    }

    fn context(&self, id: BrowserContextId) -> Result<&BrowserContext, String> {
        self.contexts
            .get(&id)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())
    }

    fn context_mut(&mut self, id: BrowserContextId) -> Result<&mut BrowserContext, String> {
        self.contexts
            .get_mut(&id)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())
    }

    fn insert_context(&mut self, context: BrowserContext) -> BrowserContextId {
        let id = context.id();
        let previous = self.contexts.insert(id, context);
        debug_assert!(previous.is_none(), "BrowserContext identity must be unique");
        self.events.publish(super::BrowserEvent::ContextCreated(id));
        id
    }

    fn remove_context(&mut self, id: BrowserContextId) -> bool {
        let Some(context) = self.contexts.shift_remove(&id) else {
            return false;
        };
        self.navigation_work.remove_context(id);
        let navigations = context.navigation_snapshots().collect::<Vec<_>>();
        let dialogs = context.javascript_dialog_snapshots();
        self.events
            .publish(super::BrowserEvent::ContextDisposed(id));
        context.shutdown();
        self.publish_failed_navigations(
            navigations,
            super::NavigationFailureReason::ContextDisposed,
        );
        self.publish_closed_javascript_dialogs(dialogs);
        true
    }

    fn shutdown(&mut self) {
        self.navigation_work.clear();
        let contexts = std::mem::take(&mut self.contexts);
        for id in contexts.keys().copied() {
            self.events
                .publish(super::BrowserEvent::ContextDisposed(id));
        }
        for (_, context) in contexts {
            let navigations = context.navigation_snapshots().collect::<Vec<_>>();
            let dialogs = context.javascript_dialog_snapshots();
            context.shutdown();
            self.publish_failed_navigations(
                navigations,
                super::NavigationFailureReason::ContextDisposed,
            );
            self.publish_closed_javascript_dialogs(dialogs);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn spawn_local_context_operation<R: Send + 'static>(
        &mut self,
        id: BrowserContextId,
        operation: impl FnOnce(
            BrowserContext,
        ) -> Pin<Box<dyn Future<Output = (BrowserContext, R)> + 'static>>
        + 'static,
    ) -> Result<oneshot::Receiver<R>, String> {
        let context = self
            .contexts
            .shift_remove(&id)
            .ok_or_else(|| "BrowserContext unavailable".to_owned())?;
        let local_sender = self.local_sender.clone();
        let (completion_tx, completion) = oneshot::channel();
        tokio::task::spawn_local(async move {
            let (context, result) = operation(context).await;
            let _ = local_sender.send(Box::new(move |browser| {
                // Test-only async Context borrowing temporarily removes its
                // registry entry. Catch up the original journals before the
                // fixture reply; native callbacks during that borrow could
                // not resolve the Context. Production never removes it here.
                let progress = context
                    .web_contents_handles()
                    .filter_map(|contents| {
                        let document =
                            context.document_handle_for_web_contents(contents).ok()??;
                        let mut renderer = context
                            .document(document)
                            .ok()?
                            .page
                            .observe_document_lifecycle()?;
                        Some((document, renderer.snapshot()))
                    })
                    .collect::<Vec<_>>();
                let previous = browser.contexts.insert(id, context);
                debug_assert!(
                    previous.is_none(),
                    "BrowserContext identity must remain unique during local work"
                );
                for (document, snapshot) in progress {
                    browser.commit_document_lifecycle(document, snapshot);
                    browser.commit_javascript_dialogs(document);
                }
                let _ = completion_tx.send(result);
            }));
        });
        Ok(completion)
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct BrowserOwnerEndpoint {
    tx: mpsc::UnboundedSender<BrowserOwnerMessage>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
}

impl BrowserOwnerEndpoint {
    fn shutdown_and_join(&self) {
        let _ = self.tx.send(BrowserOwnerMessage::Shutdown);
        if let Some(join) = self.join.lock().take()
            && join.join().is_err()
        {
            tracing::error!("Browser owner thread panicked during shutdown");
        }
    }
}

impl Drop for BrowserOwnerEndpoint {
    fn drop(&mut self) {
        let _ = self.tx.send(BrowserOwnerMessage::Shutdown);
        if let Some(join) = self.join.get_mut().take()
            && join.join().is_err()
        {
            tracing::error!("Browser owner thread panicked during drop");
        }
    }
}

/// Cloneable command endpoint for the unique Browser owner sequence.
#[derive(Clone)]
pub struct BrowserHandle {
    endpoint: Arc<BrowserOwnerEndpoint>,
}

impl fmt::Debug for BrowserHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserHandle").finish_non_exhaustive()
    }
}

impl BrowserHandle {
    fn start() -> Result<Self, String> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("moli-browser-owner".to_owned())
            .stack_size(BROWSER_OWNER_STACK_SIZE)
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!(
                            "failed to build Browser owner runtime: {error}"
                        )));
                        return;
                    }
                };
                let local = tokio::task::LocalSet::new();
                let _ = ready_tx.send(Ok(()));
                local.block_on(&runtime, async move {
                    let (local_tx, mut local_rx) = mpsc::unbounded_channel();
                    let mut browser = Browser::new(local_tx);
                    loop {
                        tokio::select! {
                            message = rx.recv() => match message {
                                Some(BrowserOwnerMessage::Execute(operation)) => operation(&mut browser),
                                Some(BrowserOwnerMessage::Shutdown) | None => break,
                            },
                            operation = local_rx.recv() => {
                                if let Some(operation) = operation {
                                    operation(&mut browser);
                                }
                            }
                        }
                    }
                    browser.shutdown();
                });
            })
            .map_err(|error| format!("failed to spawn Browser owner: {error}"))?;
        ready_rx
            .recv()
            .map_err(|error| format!("Browser owner stopped during startup: {error}"))??;
        Ok(Self {
            endpoint: Arc::new(BrowserOwnerEndpoint {
                tx,
                join: Mutex::new(Some(join)),
            }),
        })
    }

    fn execute<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Browser) -> R + Send + 'static,
    ) -> Result<R, String> {
        let (result_tx, result_rx) = std_mpsc::sync_channel(1);
        self.endpoint
            .tx
            .send(BrowserOwnerMessage::Execute(Box::new(move |browser| {
                let _ = result_tx.send(operation(browser));
            })))
            .map_err(|_| "Browser owner is unavailable".to_owned())?;
        result_rx
            .recv()
            .map_err(|_| "Browser owner stopped before completing the operation".to_owned())
    }

    pub fn create_context(
        &self,
        handles: BrowserContextStoragePartitionHandles,
        kind: StoragePartitionKind,
        http_cache_root: Option<PathBuf>,
        http_cache_max_bytes: Option<u64>,
    ) -> Result<BrowserContextHandle, String> {
        let id = self.execute(move |browser| {
            browser.insert_context(BrowserContext::new(
                handles,
                kind,
                http_cache_root,
                http_cache_max_bytes,
            ))
        })?;
        Ok(BrowserContextHandle {
            browser: self.clone(),
            id,
        })
    }

    pub fn contains_context(&self, id: BrowserContextId) -> bool {
        self.execute(move |browser| browser.contexts.contains_key(&id))
            .unwrap_or(false)
    }

    pub fn context_handle(&self, id: BrowserContextId) -> Result<BrowserContextHandle, String> {
        self.execute(move |browser| browser.context(id).map(|_| ()))??;
        Ok(BrowserContextHandle {
            browser: self.clone(),
            id,
        })
    }

    pub fn web_contents_snapshot(
        &self,
        handle: WebContentsHandle,
    ) -> Result<super::WebContentsSnapshot, String> {
        self.execute(move |browser| {
            let context = browser.context(handle.context())?;
            let (_, main_frame) = context.web_contents_identity(handle)?;
            let document = context.document_handle(handle)?;
            let url = document
                .map(|document| context.document_url(document).map(|url| url.to_string()))
                .transpose()?
                .or(context.initial_document_url(handle)?)
                .unwrap_or_else(|| "about:blank".to_owned());
            Ok(super::WebContentsSnapshot {
                handle,
                main_frame,
                document,
                url,
                popup: context.web_contents(handle)?.window.popup_creation.clone(),
            })
        })?
    }

    pub fn document_commit_snapshot(
        &self,
        document: super::DocumentHandle,
    ) -> Result<super::web_contents::DocumentCommitSnapshot, String> {
        self.execute(move |browser| {
            browser
                .context(document.web_contents().context())?
                .document_commit_snapshot(document)
        })?
    }

    pub fn document_for_renderer(
        &self,
        renderer: super::RendererPageResidenceIdentity,
    ) -> Option<super::DocumentHandle> {
        self.execute(move |browser| {
            browser
                .contexts
                .values()
                .find_map(|context| context.document_for_renderer(renderer))
        })
        .ok()
        .flatten()
    }

    /// Waits for already-produced renderer progress to cross the native owner
    /// boundary. This is observation only: frontend ingress cannot apply an
    /// event, reset lifecycle state, or select a replacement Document.
    pub async fn wait_for_renderer_document_lifecycle(
        &self,
        renderer: super::RendererPageResidenceIdentity,
        event: crate::page::RendererDocumentLifecycleEvent,
    ) -> Option<super::web_contents::DocumentLifecycleEvent> {
        let (document, mut progress) = self
            .execute(move |browser| {
                let document = browser
                    .contexts
                    .values()
                    .find_map(|context| context.document_for_renderer(renderer))?;
                let host = browser
                    .context_mut(document.web_contents().context())
                    .ok()?
                    .document_mut(document)
                    .ok()?;
                Some((document, host.lifecycle.observe_committed()?))
            })
            .ok()??;
        loop {
            let snapshot = *progress.borrow_and_update();
            if snapshot.frame != event.frame || snapshot.document != event.document {
                return None;
            }
            if snapshot.sequence() >= event.sequence {
                return Some(super::web_contents::DocumentLifecycleEvent::new(
                    document.id(),
                    event,
                ));
            }
            progress.changed().await.ok()?;
        }
    }

    /// Subscribe and snapshot in the same owner turn, without a gap between
    /// observing existing Contexts and receiving their subsequent lifecycle.
    pub fn subscribe(
        &self,
    ) -> Result<(super::BrowserSnapshot, super::BrowserEventReceiver), String> {
        self.execute(|browser| {
            browser.events.subscribe(
                browser.contexts.keys().copied(),
                browser
                    .contexts
                    .values()
                    .flat_map(BrowserContext::web_contents_handles),
                browser
                    .contexts
                    .values()
                    .filter_map(BrowserContext::selected_web_contents_handle),
                browser.contexts.values().flat_map(|context| {
                    context.web_contents_handles().filter_map(|contents| {
                        context
                            .document_handle_for_web_contents(contents)
                            .ok()
                            .flatten()
                    })
                }),
                browser.contexts.values().flat_map(|context| {
                    context.web_contents_handles().filter_map(|contents| {
                        let document =
                            context.document_handle_for_web_contents(contents).ok()??;
                        Some(super::DocumentLifecycleSnapshot {
                            document,
                            lifecycle: context.document_lifecycle_snapshot(document).ok()??,
                        })
                    })
                }),
                browser
                    .contexts
                    .values()
                    .flat_map(|context| context.downloads.snapshots()),
                browser
                    .contexts
                    .values()
                    .flat_map(BrowserContext::javascript_dialog_snapshots),
                browser
                    .contexts
                    .values()
                    .flat_map(BrowserContext::navigation_snapshots),
            )
        })
    }

    pub fn remove_context(&self, id: BrowserContextId) -> Result<bool, String> {
        self.execute(move |browser| browser.remove_context(id))
    }

    pub fn close_web_contents(
        &self,
        handle: WebContentsHandle,
    ) -> Result<PendingWebContentsClose, String> {
        self.execute(move |browser| {
            let context = browser.context_mut(handle.context())?;
            let was_selected = context.selected_web_contents_handle() == Some(handle);
            let navigation = context.navigation_snapshot(handle)?;
            let dialogs = context.web_contents_javascript_dialog_snapshots(handle);
            let closing = context.close_web_contents(handle)?;
            let activated = was_selected
                .then(|| context.selected_web_contents_handle())
                .flatten();
            let surface = activated.and_then(|selected| {
                context.start_web_contents_visibility_update(selected, true).unwrap_or_else(|error| {
                    tracing::warn!(%error, "failed to activate surviving WebContents surface");
                    None
                })
            });
            browser.navigation_work.remove_web_contents(handle);
            browser.publish_failed_navigations([navigation], super::NavigationFailureReason::WebContentsClosed);
            let event = browser
                .events
                .publish(super::BrowserEvent::WebContentsClosed {
                    web_contents: handle,
                    activated,
                });
            browser.publish_closed_javascript_dialogs(dialogs);
            let (completion_tx, completion) = oneshot::channel();
            let local_sender = browser.local_sender.clone();
            tokio::task::spawn_local(async move {
                closing.close_async().await;
                let surface = match surface {
                    Some(surface) => Some(surface.wait().await),
                    None => None,
                };
                let _ = local_sender.send(Box::new(move |browser| {
                    if let Some(surface) = surface {
                        let result = browser.context_mut(handle.context())
                            .and_then(|context| context.finish_document_policy_update(surface));
                        if let Err(error) = result {
                            tracing::warn!(%error, "surviving WebContents surface update did not complete");
                        }
                    }
                    let _ = completion_tx.send(());
                }));
            });
            Ok(PendingWebContentsClose {
                completion,
                event,
            })
        })?
    }

    pub fn set_permission_default(
        &self,
        registration: crate::page::PermissionOverrideRegistration,
    ) {
        self.execute(move |browser| browser.permission_defaults.set(registration))
            .expect("live Browser owner must accept permission defaults");
    }

    pub fn clear_permission_defaults(&self) {
        self.execute(|browser| browser.permission_defaults.clear())
            .expect("live Browser owner must clear permission defaults");
    }

    pub fn permission_default_count(&self) -> usize {
        self.execute(|browser| browser.permission_defaults.override_count())
            .expect("live Browser owner must snapshot permission defaults")
    }

    pub fn permission_defaults_snapshot(&self) -> Vec<crate::page::PermissionOverrideRegistration> {
        self.execute(|browser| browser.permission_defaults.snapshot())
            .expect("live Browser owner must snapshot permission defaults")
    }
}

/// Service lifetime guard for the Browser owner sequence.
///
/// AppState keeps this value alive independently of every frontend socket.
#[derive(Clone, Debug)]
pub struct BrowserService {
    handle: BrowserHandle,
}

impl BrowserService {
    pub fn start() -> Result<Self, String> {
        Ok(Self {
            handle: BrowserHandle::start()?,
        })
    }

    pub fn handle(&self) -> BrowserHandle {
        self.handle.clone()
    }

    pub fn shutdown(&self) {
        self.handle.endpoint.shutdown_and_join();
    }
}

/// Exact Context capability backed by the Browser owner sequence.
#[derive(Clone, Debug)]
pub struct BrowserContextHandle {
    browser: BrowserHandle,
    id: BrowserContextId,
}

/// Completion of a Browser-owned WebContents teardown.
pub struct PendingWebContentsClose {
    completion: oneshot::Receiver<()>,
    /// The exact committed close occurrence, including any selected successor.
    pub event: super::BrowserEventRecord,
}

impl PendingWebContentsClose {
    pub async fn close_async(self) {
        let _ = self.completion.await;
    }
}

/// Completion of a Browser-owned Document retirement.
pub struct PendingDocumentRetirement {
    completion: oneshot::Receiver<()>,
}

impl PendingDocumentRetirement {
    pub async fn close(self) {
        let _ = self.completion.await;
    }
}

/// Initial Browser-owned state for a newly created WebContents.
///
/// Frontend target/session identities are projected only after this request
/// returns the physical handle.
#[derive(Default)]
pub struct WebContentsCreation {
    initial_document: Option<(
        String,
        Option<InitialDocumentCreator>,
        Option<moli_storage_key::MoliStorageKey>,
    )>,
    session_storage: Option<SessionStorageNamespace>,
}

impl WebContentsCreation {
    pub fn with_initial_document(
        initial_url: String,
        creator: Option<InitialDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        Self {
            initial_document: Some((initial_url, creator, storage_key)),
            session_storage: None,
        }
    }

    pub fn with_session_storage(mut self, namespace: SessionStorageNamespace) -> Self {
        self.session_storage = Some(namespace);
        self
    }

    fn build(self) -> WebContents {
        let mut contents = WebContents::default();
        if let Some((url, creator, storage_key)) = self.initial_document {
            contents.begin_initial_empty_document(url, creator, storage_key);
        }
        if let Some(namespace) = self.session_storage {
            contents.install_session_storage_namespace(namespace);
        }
        contents
    }
}

impl BrowserContextHandle {
    pub fn id(&self) -> BrowserContextId {
        self.id
    }

    pub fn is_live(&self) -> bool {
        self.browser.contains_context(self.id)
    }

    pub fn remove(&self) -> Result<bool, String> {
        self.browser.remove_context(self.id)
    }

    fn read<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&BrowserContext) -> R + Send + 'static,
    ) -> Result<R, String> {
        let id = self.id;
        self.browser
            .execute(move |browser| browser.context(id).map(operation))?
    }

    fn update<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut BrowserContext) -> R + Send + 'static,
    ) -> Result<R, String> {
        let id = self.id;
        self.browser
            .execute(move |browser| browser.context_mut(id).map(operation))?
    }

    fn read_live<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&BrowserContext) -> R + Send + 'static,
    ) -> R {
        self.read(operation)
            .expect("live BrowserContext handle must resolve in its owner")
    }

    fn update_live<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut BrowserContext) -> R + Send + 'static,
    ) -> R {
        self.update(operation)
            .expect("live BrowserContext handle must resolve in its owner")
    }

    fn try_read<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&BrowserContext) -> Result<R, String> + Send + 'static,
    ) -> Result<R, String> {
        self.read(operation)?
    }

    fn try_update<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut BrowserContext) -> Result<R, String> + Send + 'static,
    ) -> Result<R, String> {
        self.update(operation)?
    }

    pub fn create_web_contents(
        &self,
        creation: WebContentsCreation,
    ) -> Result<(WebContentsHandle, MainFrameSlotId), String> {
        let id = self.id;
        self.browser.execute(move |browser| {
            let created = browser
                .context_mut(id)?
                .register_web_contents(creation.build())?;
            browser
                .events
                .publish(super::BrowserEvent::WebContentsCreated(created.0));
            Ok(created)
        })?
    }

    pub fn web_contents_window_name(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<String>, String> {
        self.try_read(move |context| {
            context
                .web_contents_window_name(handle)
                .map(|name| name.map(str::to_owned))
        })
    }

    pub fn close_web_contents(
        &self,
        handle: WebContentsHandle,
    ) -> Result<PendingWebContentsClose, String> {
        if handle.context() != self.id {
            return Err("WebContents belongs to another BrowserContext".into());
        }
        self.browser.close_web_contents(handle)
    }

    pub fn close_all_web_contents(&self) -> Vec<PendingWebContentsClose> {
        let context = self.id;
        self.browser
            .execute(move |browser| {
                let handles = browser
                    .context_mut(context)?
                    .web_contents_handles()
                    .collect::<Vec<_>>();
                let dialogs = browser.context(context)?.javascript_dialog_snapshots();
                let navigations = browser
                    .context(context)?
                    .navigation_snapshots()
                    .collect::<Vec<_>>();
                let closing = browser.context_mut(context)?.close_all_web_contents();
                browser.publish_failed_navigations(
                    navigations,
                    super::NavigationFailureReason::WebContentsClosed,
                );
                browser.publish_closed_javascript_dialogs(dialogs);
                browser.navigation_work.remove_context(context);
                Ok::<_, String>(
                    handles
                        .into_iter()
                        .zip(closing)
                        .map(|(web_contents, closing)| {
                            let event =
                                browser
                                    .events
                                    .publish(super::BrowserEvent::WebContentsClosed {
                                        web_contents,
                                        activated: None,
                                    });
                            let (completion_tx, completion) = oneshot::channel();
                            tokio::task::spawn_local(async move {
                                closing.close_async().await;
                                let _ = completion_tx.send(());
                            });
                            PendingWebContentsClose { completion, event }
                        })
                        .collect(),
                )
            })
            .expect("live Browser owner must accept WebContents teardown")
            .expect("live BrowserContext handle must resolve in its owner")
    }

    pub fn start_document_navigation(
        &self,
        handle: WebContentsHandle,
    ) -> Result<super::NavigationId, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            // Validate the exact Context before superseding any previous request.
            browser.context(context)?.web_contents(handle)?;
            browser.start_navigation(handle)
        })?
    }

    pub fn clear_document_navigation_state(&self, handle: WebContentsHandle) -> Result<(), String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            let previous = browser.context(context)?.navigation_snapshot(handle)?;
            browser
                .context_mut(context)?
                .clear_document_navigation_state(handle)?;
            browser.navigation_work.remove_web_contents(handle);
            browser
                .publish_failed_navigations([previous], super::NavigationFailureReason::Canceled);
            Ok(())
        })?
    }

    pub fn retire_document(
        &self,
        handle: WebContentsHandle,
    ) -> Result<PendingDocumentRetirement, String> {
        self.try_update(move |context| {
            let retiring = context.retire_document(handle)?;
            let (completion_tx, completion) = oneshot::channel();
            tokio::task::spawn_local(async move {
                retiring.close().await;
                let _ = completion_tx.send(());
            });
            Ok(PendingDocumentRetirement { completion })
        })
    }

    pub fn bind_page_navigation_engines(
        &self,
        config: crate::runtime::NavigationRuntimeConfig,
        sender: Option<crate::RendererOutputTransportSender>,
    ) {
        self.update_live(move |context| context.bind_page_navigation_engines(config, sender));
    }

    pub fn set_renderer_output_transport_sender(
        &self,
        sender: crate::RendererOutputTransportSender,
    ) -> Result<(), String> {
        self.update(move |context| context.set_renderer_output_transport_sender(sender))
    }

    pub fn contains_web_contents(&self, handle: WebContentsHandle) -> bool {
        self.read(move |context| context.contains_web_contents(handle))
            .unwrap_or(false)
    }

    pub fn select_web_contents(&self, id: super::WebContentsId) -> bool {
        // Selection is committed at admission. The Browser owns the remaining
        // visibility work even when the native caller does not await its reply.
        self.browser
            .activate_web_contents(WebContentsHandle::new(self.id, id))
            .is_ok()
    }

    pub fn activate_web_contents(
        &self,
        handle: WebContentsHandle,
    ) -> Result<PendingWebContentsActivation, String> {
        if handle.context() != self.id {
            return Err("WebContents belongs to another BrowserContext".into());
        }
        self.browser.activate_web_contents(handle)
    }

    pub fn selected_web_contents_id(&self) -> Option<super::WebContentsId> {
        self.read(BrowserContext::selected_web_contents_id)
            .ok()
            .flatten()
    }

    pub fn selected_web_contents_handle(&self) -> Option<WebContentsHandle> {
        self.read(BrowserContext::selected_web_contents_handle)
            .ok()
            .flatten()
    }

    pub fn selected_web_contents_snapshot(&self) -> Option<super::WebContentsSelection> {
        self.read(BrowserContext::selected_web_contents_snapshot)
            .ok()
            .flatten()
    }

    pub fn web_contents_count(&self) -> usize {
        self.read_live(BrowserContext::web_contents_count)
    }

    pub fn web_contents_handle_for_window_id(&self, window_id: u64) -> Option<WebContentsHandle> {
        self.read_live(move |context| context.web_contents_handle_for_window_id(window_id))
    }

    pub fn web_contents_handle_for_window_name(&self, name: &str) -> Option<WebContentsHandle> {
        let name = name.to_owned();
        self.read_live(move |context| context.web_contents_handle_for_window_name(&name))
    }

    pub fn web_contents_for_renderer_popup(
        &self,
        renderer: super::RendererPageResidenceIdentity,
        popup_id: u64,
    ) -> Option<WebContentsHandle> {
        self.read(move |context| context.web_contents_for_renderer_popup(renderer, popup_id))
            .ok()
            .flatten()
    }

    pub fn clone_session_storage_namespace(
        &self,
        web_contents: super::WebContentsId,
    ) -> Option<SessionStorageNamespace> {
        self.read_live(move |context| context.clone_session_storage_namespace(web_contents))
    }

    pub fn web_contents_is_crashed(&self, handle: WebContentsHandle) -> Result<bool, String> {
        self.try_read(move |context| context.web_contents_is_crashed(handle))
    }

    pub fn set_web_contents_crashed(
        &self,
        handle: WebContentsHandle,
        crashed: bool,
    ) -> Result<(), String> {
        self.try_update(move |context| context.set_web_contents_crashed(handle, crashed))
    }

    pub fn performance_metric_snapshot(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<crate::page::RendererPerformanceMetricSnapshot>, String> {
        self.try_read(move |context| context.performance_metric_snapshot(handle))
    }

    pub fn observe_renderer_page_state(
        &self,
        handle: WebContentsHandle,
        snapshot: Arc<moli_renderer_v8::RendererPageState>,
    ) -> Result<bool, String> {
        self.try_update(move |context| context.observe_renderer_page_state(handle, &snapshot))
    }

    pub fn web_contents_window_surface(
        &self,
        handle: WebContentsHandle,
    ) -> Result<super::web_contents::WindowSurface, String> {
        self.try_read(move |context| context.web_contents_window_surface(handle))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_web_contents_window_surface(
        &self,
        handle: WebContentsHandle,
        state: Option<super::web_contents::WindowSurfaceState>,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.update_web_contents_window_surface(handle, state, width, height, x, y)
        })
    }

    pub fn set_web_contents_window_name(
        &self,
        handle: WebContentsHandle,
        name: Option<String>,
    ) -> Result<(), String> {
        self.try_update(move |context| context.set_web_contents_window_name(handle, name))
    }

    pub fn set_web_contents_opener(
        &self,
        handle: WebContentsHandle,
        opener: Option<WebContentsHandle>,
        can_access: bool,
    ) -> Result<(), String> {
        self.try_update(move |context| context.set_web_contents_opener(handle, opener, can_access))
    }

    pub fn set_web_contents_network_offline(
        &self,
        handle: WebContentsHandle,
        offline: bool,
    ) -> Result<(), String> {
        self.try_update(move |context| context.set_web_contents_network_offline(handle, offline))
    }

    pub fn web_contents_network_offline(&self, handle: WebContentsHandle) -> Result<bool, String> {
        self.try_read(move |context| context.web_contents_network_offline(handle))
    }

    pub fn web_contents_emulation_policy(
        &self,
        handle: WebContentsHandle,
    ) -> Result<super::web_contents::EmulationPolicy, String> {
        self.try_read(move |context| context.web_contents_emulation_policy(handle))
    }

    pub fn apply_web_contents_emulation_policy_change(
        &self,
        handle: WebContentsHandle,
        change: super::web_contents::EmulationPolicyChange,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.apply_web_contents_emulation_policy_change(handle, change)
        })
    }

    pub fn apply_web_contents_emulation_policy_changes(
        &self,
        handle: WebContentsHandle,
        changes: Vec<super::web_contents::EmulationPolicyChange>,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.apply_web_contents_emulation_policy_changes(handle, changes)
        })
    }

    pub fn web_contents_window_counts(&self) -> (usize, usize, usize) {
        self.read_live(BrowserContext::web_contents_window_counts)
    }

    pub fn loaded_document_count(&self) -> usize {
        self.read_live(BrowserContext::loaded_document_count)
    }

    pub fn has_pending_javascript_dialog(&self) -> bool {
        self.read(BrowserContext::has_pending_javascript_dialog)
            .unwrap_or(false)
    }

    forward_context_read! {
        fn has_loaded_document(handle: WebContentsHandle) -> bool;
        fn document_renderer_matches(handle: WebContentsHandle, renderer: super::RendererPageResidenceIdentity) -> bool;
        fn has_inflight_background_navigation() -> bool;
        fn loaded_document_renderer_owner_ids() -> std::collections::HashSet<u64>;
        fn dedicated_worker_running_isolate_count() -> usize;
        fn routes_renderer_browser_context_runtime(runtime: crate::RendererBrowserContextRuntimeId) -> bool;
        fn renderer_memory_diagnostics() -> serde_json::Value;
        fn shared_worker_runtime_diagnostics() -> crate::runtime::RendererSharedWorkerRuntimeDiagnostics;
        fn javascript_dialog_handler_enabled() -> bool;
    }

    forward_context_update! {
        fn invalidate_selected_resource_runtime() -> ();
        fn clear_permission_overrides() -> ();
        fn set_download_policy(policy: Option<super::DownloadPolicy>) -> ();
        fn set_browser_identity_override(identity: Option<moli_browser_profile::BrowserIdentityProfile>) -> ();
        fn set_default_locale_override(locale: Option<String>) -> ();
        fn set_default_timezone_override(timezone: Option<String>) -> ();
        fn set_default_network_conditions(conditions: Option<super::EmulatedNetworkConditions>) -> ();
        fn set_default_geolocation_override(geolocation: Option<super::EmulatedGeolocationOverrideState>) -> ();
        fn set_default_device_metrics(metrics: super::EmulatedDeviceMetrics) -> bool;
        fn set_default_extra_headers(headers: Vec<(String, String)>) -> ();
        fn set_network_policy(policy: super::ContextNetworkPolicy) -> ();
        fn set_context_tls_verify_host_override(enabled: bool) -> ();
    }

    forward_context_try_read! {
        fn initial_document_build_pending(handle: WebContentsHandle) -> bool;
        fn web_contents_initial_document_state(handle: WebContentsHandle) -> Option<super::web_contents::InitialDocumentSnapshot>;
        fn document_handle(handle: WebContentsHandle) -> Option<super::DocumentHandle>;
        fn web_contents_identity(handle: WebContentsHandle) -> (super::WebContentsId, super::MainFrameSlotId);
        fn resolve_history_traversal(handle: WebContentsHandle, destination: super::web_contents::HistoryTraversalDestination) -> super::web_contents::ResolvedHistoryTraversal;
        fn document_service_worker_client_id(handle: super::DocumentHandle) -> u64;
        fn document_renderer_residence(handle: super::DocumentHandle) -> super::RendererPageResidenceIdentity;
        fn web_contents_renderer_popup_sources(handle: WebContentsHandle) -> Vec<(super::RendererPageResidenceIdentity, u64)>;
        fn document_url(handle: super::DocumentHandle) -> url::Url;
        fn document_title(handle: super::DocumentHandle) -> String;
        fn document_lifecycle_snapshot(handle: super::DocumentHandle) -> Option<crate::page::RendererDocumentLifecycleSnapshot>;
        fn initial_document_url(handle: WebContentsHandle) -> Option<String>;
        fn initial_document_storage_key(handle: WebContentsHandle) -> Option<moli_storage_key::MoliStorageKey>;
        fn is_on_initial_document(handle: WebContentsHandle) -> Option<bool>;
        fn initial_document_has_pending_navigation(handle: WebContentsHandle) -> bool;
        fn navigation_history_snapshot(handle: WebContentsHandle) -> (usize, Vec<super::web_contents::PageNavigationHistoryEntry>);
        fn navigation_snapshot(handle: WebContentsHandle) -> super::NavigationSnapshot;
        fn navigation_history_entry_url(handle: WebContentsHandle, entry_id: i32) -> Option<String>;
        fn has_pending_document_navigation(handle: WebContentsHandle) -> bool;
        fn navigation_is_default(handle: WebContentsHandle) -> bool;
        fn navigation_retains(handle: WebContentsHandle, navigation: super::NavigationId) -> bool;
        fn current_document_navigation(handle: WebContentsHandle) -> Option<super::NavigationId>;
        fn committed_document_navigation(handle: WebContentsHandle) -> Option<super::NavigationId>;
        fn has_inflight_background_navigation_for_web_contents(handle: WebContentsHandle) -> bool;
        fn accepts_document_preparation(handle: WebContentsHandle, navigation: super::NavigationId, renderer: super::RendererPageResidenceIdentity) -> bool;
    }

    forward_context_try_update! {
        fn mark_next_navigation_history_replace_current(handle: WebContentsHandle) -> ();
        fn mark_next_navigation_history_traverse_to_entry(handle: WebContentsHandle, entry_id: i32) -> ();
        fn commit_same_document_navigation(handle: WebContentsHandle, document: super::DocumentId, url: url::Url, history_update: crate::page::SameDocumentHistoryUpdate) -> Option<super::web_contents::SameDocumentNavigationCommitted>;
        fn mark_renderer_crashed(handle: WebContentsHandle) -> ();
        fn begin_initial_empty_document(handle: WebContentsHandle, initial_url: String, creator: Option<super::web_contents::InitialDocumentCreator>, storage_key: Option<moli_storage_key::MoliStorageKey>) -> ();
        fn crash_web_contents_renderer_from_io(handle: WebContentsHandle) -> ();
    }

    pub fn take_navigation_request(
        &self,
        permit: super::web_contents::NavigationInterceptionPermit,
    ) -> Option<super::web_contents::ClaimedNavigationRequest> {
        self.update_live(move |context| context.take_navigation_request(permit))
    }

    pub fn take_navigation_response(
        &self,
        permit: super::web_contents::NavigationInterceptionPermit,
    ) -> Option<super::web_contents::PausedDocumentTransfer> {
        self.update_live(move |context| context.take_navigation_response(permit))
    }

    pub fn restore_navigation_response(
        &self,
        permit: super::web_contents::NavigationInterceptionPermit,
        transfer: super::web_contents::PausedDocumentTransfer,
    ) -> Result<(), Box<super::web_contents::PausedDocumentTransfer>> {
        self.update_live(move |context| context.restore_navigation_response(permit, transfer))
    }

    pub fn commit_document_title(
        &self,
        handle: WebContentsHandle,
        change: &crate::RendererDocumentTitleChanged,
    ) -> Result<Option<bool>, String> {
        let change = change.clone();
        self.try_update(move |context| context.commit_document_title(handle, &change))
    }

    pub fn arm_background_navigation_completion(
        &self,
        token: &super::NavigationId,
        additional_cancellation: Option<moli_fetch::FetchCancelHandle>,
    ) -> bool {
        let token = *token;
        self.update_live(move |context| {
            context.arm_background_navigation_completion(&token, additional_cancellation)
        })
    }

    pub fn accepts_any_pending_navigation_event(&self, token: &super::NavigationId) -> bool {
        let token = *token;
        self.read_live(move |context| context.accepts_any_pending_navigation_event(&token))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn document_navigation_cancellation_handle_for_test(
        &self,
        token: &super::NavigationId,
    ) -> Option<moli_fetch::FetchCancelHandle> {
        let token = *token;
        self.read_live(move |context| {
            context.document_navigation_cancellation_handle_for_test(&token)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn commit_pending_document_navigation_for_test(
        &self,
        handle: WebContentsHandle,
        token: &super::NavigationId,
    ) -> bool {
        let token = *token;
        self.update_live(move |context| {
            context.commit_pending_document_navigation_for_test(handle, &token)
        })
    }

    pub fn settle_background_navigation_completion(&self, token: &super::NavigationId) -> bool {
        let token = *token;
        self.update_live(move |context| context.settle_background_navigation_completion(&token))
    }

    pub fn accepts_pending_navigation(
        &self,
        handle: WebContentsHandle,
        navigation: &super::NavigationId,
    ) -> Result<bool, String> {
        let navigation = *navigation;
        self.try_read(move |context| context.accepts_pending_navigation(handle, &navigation))
    }

    pub fn accepts_document_body_completion(
        &self,
        handle: WebContentsHandle,
        navigation: &super::NavigationId,
    ) -> Result<bool, String> {
        let navigation = *navigation;
        self.try_read(move |context| context.accepts_document_body_completion(handle, &navigation))
    }

    pub fn cancel_document_navigation(
        &self,
        handle: WebContentsHandle,
        navigation: &super::NavigationId,
    ) -> Result<bool, String> {
        let navigation = *navigation;
        if handle.context() != self.id {
            return Err("WebContents belongs to another BrowserContext".into());
        }
        self.browser.execute(move |browser| {
            browser.cancel_navigation(handle, navigation, super::NavigationFailureReason::Canceled)
        })?
    }

    forward_context_read! {
        fn controlled_service_worker_window_client_ids(registration_id: u64, version_id: u64) -> Vec<u64>;
        fn set_service_worker_pause_on_start_for_version(version_id: u64, pause: bool) -> bool;
        fn close_shared_worker(instance_id: moli_shared_worker::SharedWorkerInstanceId) -> bool;
        fn close_dedicated_worker(instance_id: u64) -> bool;
        fn attach_dedicated_worker_inspector_session(instance_id: u64, session_id: Option<String>) -> bool;
        fn detach_shared_worker_inspector_session(instance_id: moli_shared_worker::SharedWorkerInstanceId, session_id: Option<String>) -> bool;
        fn detach_dedicated_worker_inspector_session(instance_id: u64, session_id: Option<String>) -> bool;
        fn detach_service_worker_inspector_session(version_id: u64, session_id: Option<String>) -> bool;
        fn run_dedicated_worker_if_waiting_for_debugger(instance_id: u64) -> bool;
        fn run_service_worker_if_waiting_for_debugger(version_id: u64) -> bool;
        fn worker_runtime_inspection_endpoint() -> crate::runtime::RendererBrowserContextRuntime;
        fn permission_override_count() -> usize;
        fn storage_partition_kind() -> super::StoragePartitionKind;
        fn storage_partition_kind_label() -> &'static str;
        fn page_storage_handles(web_contents: Option<WebContentsHandle>) -> Result<super::BrowserContextPageStorageHandles, String>;
        fn clear_http_cache() -> Result<(), String>;
        fn snapshot_cookies() -> Vec<moli_cookie_jar::StoredCookie>;
        fn web_contents_for_renderer_owner(owner: crate::RendererOwnerLocalHostId) -> Option<WebContentsHandle>;
    }

    /// Classify the partition and snapshot its cookies in one exact owner turn.
    pub fn snapshot_profile_backed_cookies(
        &self,
    ) -> Result<Option<Vec<moli_cookie_jar::StoredCookie>>, String> {
        self.read(|context| {
            (context.storage_partition_kind() == super::StoragePartitionKind::ProfileBacked)
                .then(|| context.snapshot_cookies())
        })
    }

    // Session policy retirement can follow native Context or Browser shutdown.
    // An absent owner needs no reset; never redirect it to a surviving Context.
    pub fn set_service_worker_pause_on_start(&self, pause: bool) {
        let _ = self.update(move |context| context.set_service_worker_pause_on_start(pause));
    }

    pub fn set_service_worker_related_pause_on_start_policies(
        &self,
        policies: Vec<(u64, u64, String, String)>,
    ) {
        let _ = self.update(move |context| {
            context.set_service_worker_related_pause_on_start_policies(policies)
        });
    }

    pub fn set_dedicated_worker_pause_on_start(&self, pause: bool) {
        let _ = self.update(move |context| context.set_dedicated_worker_pause_on_start(pause));
    }

    forward_context_update! {
        fn set_service_worker_inspection_attached(version_id: u64, attached: bool) -> ();
        fn set_storage_quota_override(origin: String, quota: f64) -> ();
        fn set_javascript_dialog_handler_enabled(enabled: bool) -> ();
    }

    pub fn execute_service_worker_command(
        &self,
        command: super::ServiceWorkerCommand,
    ) -> Result<(), String> {
        self.try_read(move |context| context.execute_service_worker_command(command))
    }

    pub fn download_policy(&self) -> Option<super::DownloadPolicy> {
        self.read_live(|context| context.download_policy().cloned())
    }

    pub fn browser_identity_override(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        self.read_live(|context| context.browser_identity_override().cloned())
    }

    pub fn emulation_defaults(&self) -> super::ContextEmulationDefaults {
        self.read_live(|context| context.emulation_defaults().clone())
    }

    pub fn network_policy(&self) -> super::ContextNetworkPolicy {
        self.read_live(|context| context.network_policy().clone())
    }

    pub fn observe_request_cookie_access_report(
        &self,
        request_url: &url::Url,
        request_context: moli_cookie_jar::NetworkCookieRequestContext,
    ) -> Option<moli_cookie_jar::StoredCookieQueryReport> {
        let request_url = request_url.clone();
        self.read_live(move |context| {
            context.observe_request_cookie_access_report(&request_url, request_context)
        })
    }

    pub fn clear_storage_quota_override(&self, origin: &str) {
        let origin = origin.to_owned();
        self.update_live(move |context| context.clear_storage_quota_override(&origin));
    }

    pub fn storage_quota_for_origin(&self, origin: &str) -> (f64, bool) {
        let origin = origin.to_owned();
        self.read_live(move |context| context.storage_quota_for_origin(&origin))
    }

    pub fn storage_usage_for_origin(
        &self,
        serialized_origin: &str,
    ) -> Result<super::OriginStorageUsage, String> {
        let serialized_origin = serialized_origin.to_owned();
        self.try_read(move |context| context.storage_usage_for_origin(&serialized_origin))
    }

    pub fn clear_site_data_for_origin(
        &self,
        origin: &url::Url,
        options: super::SiteDataClearOptions,
    ) -> Result<(), String> {
        let origin = origin.clone();
        self.try_update(move |context| context.clear_site_data_for_origin(&origin, options))
    }

    pub fn clear_site_data_for_storage_key(
        &self,
        storage_key: &moli_storage_key::MoliStorageKey,
        options: super::SiteDataClearOptions,
    ) -> Result<(), String> {
        let storage_key = storage_key.clone();
        self.try_update(move |context| {
            context.clear_site_data_for_storage_key(&storage_key, options)
        })
    }

    pub fn store_cookie(
        &self,
        cookie: moli_cookie_jar::StoredCookie,
        request_url: Option<&url::Url>,
        source: moli_cookie_jar::CookieSource,
    ) -> moli_cookie_jar::StoredCookieSetReport {
        let request_url = request_url.cloned();
        self.read_live(move |context| context.store_cookie(cookie, request_url.as_ref(), source))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn delete_cookies_with_partition_key(
        &self,
        name: Option<&str>,
        domain: Option<&str>,
        path: Option<&str>,
        url_host: Option<&str>,
        partition_key: Option<&moli_cookie_jar::StoredCookiePartitionKey>,
    ) {
        let name = name.map(str::to_owned);
        let domain = domain.map(str::to_owned);
        let path = path.map(str::to_owned);
        let url_host = url_host.map(str::to_owned);
        let partition_key = partition_key.cloned();
        self.update_live(move |context| {
            context.delete_cookies_with_partition_key(
                name.as_deref(),
                domain.as_deref(),
                path.as_deref(),
                url_host.as_deref(),
                partition_key.as_ref(),
            );
        });
    }

    pub fn start_download_request(
        &self,
        web_contents: WebContentsHandle,
        policy: &super::DownloadPolicy,
        client: crate::network::ResourceRequestClient,
        request: moli_fetch::Request,
        suggested_filename: Option<String>,
    ) -> Result<Option<super::DownloadObservation>, String> {
        let policy = policy.clone();
        let context = self.id;
        self.browser.execute(move |browser| {
            let admitted = browser.context_mut(context)?.start_download_request(
                web_contents,
                &policy,
                client,
                request,
                suggested_filename,
            )?;
            Ok(admitted.map(|admitted| browser.admit_download(admitted)))
        })?
    }

    pub fn deny_download(
        &self,
        web_contents: WebContentsHandle,
        url: String,
        headers: Vec<(String, String)>,
        suggested_filename: Option<String>,
    ) -> Result<super::DownloadObservation, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            let context = browser.context_mut(context)?;
            if !context.contains_web_contents(web_contents) {
                return Err("WebContents unavailable".into());
            }
            let admitted = context.downloads.deny(
                web_contents,
                &url,
                &headers,
                suggested_filename.as_deref(),
            )?;
            Ok(browser.admit_download(admitted))
        })?
    }

    pub fn start_download_response(
        &self,
        web_contents: WebContentsHandle,
        policy: &super::DownloadPolicy,
        url: url::Url,
        headers: Vec<(String, String)>,
        body: super::DownloadBody,
    ) -> Result<Option<super::DownloadObservation>, String> {
        let policy = policy.clone();
        let context = self.id;
        self.browser.execute(move |browser| {
            let admitted = browser.context_mut(context)?.start_download_response(
                web_contents,
                &policy,
                url,
                headers,
                body,
            )?;
            Ok(admitted.map(|admitted| browser.admit_download(admitted)))
        })?
    }

    pub fn cancel_download(&self, guid: &str) -> Option<Result<(), super::DownloadAccessError>> {
        let guid = guid.to_owned();
        self.read_live(move |context| context.cancel_download(&guid))
    }

    pub fn read_download_artifact(
        &self,
        guid: &str,
    ) -> Option<Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, super::DownloadAccessError>>
    {
        let guid = guid.to_owned();
        self.read_live(move |context| context.read_download_artifact(&guid))
    }

    forward_context_try_read! {
        fn web_contents_has_pending_javascript_dialog(handle: WebContentsHandle) -> bool;
        fn ensure_document_current(document: super::DocumentHandle) -> ();
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn install_document_javascript_dialog_for_test(
        &self,
        document: super::DocumentHandle,
        dialog: crate::page::RendererPendingJavaScriptDialog,
    ) -> Result<Option<super::web_contents::JavaScriptDialogKey>, String> {
        self.try_update(move |context| {
            context.install_document_javascript_dialog_for_test(document, dialog)
        })
    }

    pub fn document_javascript_dialog_snapshot(
        &self,
        document: super::DocumentHandle,
        key: super::web_contents::JavaScriptDialogKey,
    ) -> Option<super::web_contents::JavaScriptDialogSnapshot> {
        self.read_live(move |context| context.document_javascript_dialog_snapshot(document, key))
    }

    pub fn set_document_javascript_dialog_prompt_text(
        &self,
        document: super::DocumentHandle,
        key: super::web_contents::JavaScriptDialogKey,
        prompt_text: String,
    ) -> Result<(), super::web_contents::JavaScriptDialogError> {
        self.update(move |context| {
            context.set_document_javascript_dialog_prompt_text(document, key, prompt_text)
        })
        .unwrap_or(Err(super::web_contents::JavaScriptDialogError::NotFound))
    }

    pub fn finish_document_javascript_dialog(
        &self,
        document: super::DocumentHandle,
        key: super::web_contents::JavaScriptDialogKey,
        accepted: bool,
        prompt_text: Option<String>,
    ) -> Option<super::web_contents::JavaScriptDialogClosed> {
        let context = self.id;
        self.browser
            .execute(move |browser| {
                let context = browser.context_mut(context).ok()?;
                context.document_javascript_dialog_snapshot(document, key)?;
                let result =
                    context.finish_document_javascript_dialog(document, key, accepted, prompt_text);
                browser
                    .events
                    .publish(super::BrowserEvent::DialogClosed { document, key });
                result
            })
            .ok()
            .flatten()
    }

    pub fn dismiss_document_javascript_dialog(
        &self,
        document: super::DocumentHandle,
        key: super::web_contents::JavaScriptDialogKey,
    ) {
        let _ = self.finish_document_javascript_dialog(document, key, false, Some(String::new()));
    }

    forward_context_try_read! {
        fn document_handle_for_web_contents(handle: WebContentsHandle) -> Option<super::DocumentHandle>;
        fn document_commit_snapshot(document: super::DocumentHandle) -> super::web_contents::DocumentCommitSnapshot;
        fn start_set_document_content(document: super::DocumentHandle, frame_id: String, html: String) -> super::PendingSetDocumentContent;
        fn start_top_level_same_document_navigation(document: super::DocumentHandle, url: String) -> super::PendingTopLevelSameDocumentNavigation;
        fn start_capture_document_screencast_frame(document: super::DocumentHandle, request: crate::page::RendererCaptureScreencastFrameRequest) -> super::PendingCaptureDocumentScreencastFrame;
        fn start_capture_document_snapshot(document: super::DocumentHandle) -> super::PendingCaptureDocumentSnapshot;
        fn start_capture_document_image(document: super::DocumentHandle, request: crate::page::RendererCaptureScreenshotRequest) -> super::PendingCaptureDocumentImage;
        fn start_top_level_history_traversal(document: super::DocumentHandle, delta: i64) -> super::PendingTopLevelHistoryTraversal;
        fn start_navigation_history_reset(document: super::DocumentHandle) -> super::PendingNavigationHistoryReset;
        fn start_child_frame_navigation(document: super::DocumentHandle, frame_id: String, url: String) -> super::PendingChildFrameNavigation;
        fn start_child_frame_document_resource_text_search(document: super::DocumentHandle, frame_id: String, url: String, query: String, case_sensitive: bool, is_regex: bool) -> super::PendingDocumentResourceTextSearch;
        fn start_document_text_search(document: super::DocumentHandle, text: String, query: String, case_sensitive: bool, is_regex: bool) -> super::PendingDocumentResourceTextSearch;
        fn set_document_javascript_dialog_handler_enabled(document: super::DocumentHandle, enabled: bool) -> ();
        fn start_document_csp_bypass_update(document: super::DocumentHandle, bypass: bool) -> super::PendingDocumentCspBypassUpdate;
        fn document_subresource_network_records(document: super::DocumentHandle) -> Vec<crate::page::SubresourceNetworkRecord>;
        fn document_observable_output_snapshot(document: super::DocumentHandle) -> Vec<crate::page::ScriptObservableOutputItem>;
        fn start_document_lifecycle_stop(document: super::DocumentHandle) -> super::PendingDocumentLifecycleStop;
        fn start_document_diagnostics_snapshot(document: super::DocumentHandle) -> super::PendingDocumentDiagnosticsSnapshot;
        fn start_document_storage_key_snapshot(document: super::DocumentHandle) -> super::PendingDocumentStorageKeySnapshot;
        fn start_child_frame_tree_snapshot(document: super::DocumentHandle) -> super::PendingChildFrameTreeSnapshot;
        fn start_document_cookie_owner_snapshot(document: super::DocumentHandle) -> super::PendingDocumentCookieOwnerSnapshot;
        fn start_document_blob_read(document: super::DocumentHandle, uuid: String) -> super::PendingDocumentBlobRead;
        fn start_network_resource_load_preparation(document: super::DocumentHandle, frame_id: String, url: url::Url, disable_cache: bool, include_credentials: bool) -> super::PendingNetworkResourceLoadPreparation;
        fn start_app_manifest_load_preparation(document: super::DocumentHandle) -> super::PendingAppManifestLoadPreparation;
        fn start_app_manifest_publication(document: super::DocumentHandle, publication: crate::page::RendererAppManifestLoadPublication) -> super::PendingAppManifestPublication;
        fn start_document_autofill_trigger(document: super::DocumentHandle, request: crate::page::RendererAutofillTriggerRequest) -> super::PendingDocumentAutofillTrigger;
        fn start_document_input_command(document: super::DocumentHandle, command: super::PageInputCommand) -> super::PendingDocumentInputCommand;
    }

    forward_context_try_update! {
        fn finish_set_document_content(completed: super::CompletedSetDocumentContent) -> (crate::page::RendererSetDocumentContentResult, crate::page::RendererCommandTurnOutput);
        fn finish_top_level_same_document_navigation(completed: super::CompletedTopLevelSameDocumentNavigation) -> (bool, crate::page::RendererCommandTurnOutput);
        fn finish_capture_document_screencast_frame(completed: super::CompletedCaptureDocumentScreencastFrame) -> crate::page::RendererCaptureScreencastFrameReply;
        fn finish_capture_document_snapshot(completed: super::CompletedCaptureDocumentSnapshot) -> super::DocumentSnapshot;
        fn finish_capture_document_image(completed: super::CompletedCaptureDocumentImage) -> crate::page::RendererCaptureScreenshotReply;
        fn finish_top_level_history_traversal(completed: super::CompletedTopLevelHistoryTraversal) -> bool;
        fn finish_navigation_history_reset(completed: super::CompletedNavigationHistoryReset) -> bool;
        fn finish_child_frame_navigation(completed: super::CompletedChildFrameNavigation) -> (bool, crate::page::RendererCommandTurnOutput);
        fn finish_document_resource_text_search(completed: super::CompletedDocumentResourceTextSearch) -> crate::page::RendererResourceTextSearchOutcome;
        fn finish_document_csp_bypass_update(completed: super::CompletedDocumentCspBypassUpdate) -> ();
        fn finish_document_lifecycle_stop(completed: super::CompletedDocumentLifecycleStop) -> crate::page::RendererCommandTurnOutput;
        fn finish_document_diagnostics_snapshot(completed: super::CompletedDocumentDiagnosticsSnapshot) -> crate::page::RendererPageDiagnosticsSnapshot;
        fn start_document_child_frame_lifecycle_work(document: super::DocumentHandle, timeout: std::time::Duration) -> super::PendingChildFrameLifecycleWork;
        fn finish_document_child_frame_lifecycle_work(completed: super::CompletedChildFrameLifecycleWork) -> (bool, crate::page::RendererCommandTurnOutput);
        fn finish_document_storage_key_snapshot(completed: super::CompletedDocumentStorageKeySnapshot) -> String;
        fn finish_child_frame_tree_snapshot(completed: super::CompletedChildFrameTreeSnapshot) -> Vec<crate::page::ChildFrameTreeSnapshot>;
        fn finish_document_cookie_owner_snapshot(completed: super::CompletedDocumentCookieOwnerSnapshot) -> crate::page::DocumentCookieOwnerSnapshot;
        fn finish_document_blob_read(completed: super::CompletedDocumentBlobRead) -> Option<Arc<[u8]>>;
        fn finish_network_resource_load_preparation(completed: super::CompletedNetworkResourceLoadPreparation) -> crate::page::RendererNetworkResourceLoadPreparation;
        fn finish_app_manifest_load_preparation(completed: super::CompletedAppManifestLoadPreparation) -> super::BrowserAppManifestLoadPreparation;
        fn finish_app_manifest_publication(completed: super::CompletedAppManifestPublication) -> crate::page::RendererCommandTurnOutput;
        fn finish_document_autofill_trigger(completed: super::CompletedDocumentAutofillTrigger) -> crate::page::RendererAutofillTriggerOutcome;
        fn finish_document_input_command(completed: super::CompletedDocumentInputCommand) -> crate::page::RendererCommandTurnOutput;
        fn observe_document_lifetime(document: super::DocumentHandle) -> super::DocumentLifetimeObserver;
    }

    pub fn document_response_headers(
        &self,
        document: super::DocumentHandle,
    ) -> Result<Vec<(String, String)>, String> {
        self.try_read(move |context| {
            context
                .document_response_headers(document)
                .map(<[(String, String)]>::to_vec)
        })
    }

    forward_context_update! {
        fn start_document_policy_batch(document: super::DocumentHandle, updates: Vec<super::DocumentPolicyUpdate>) -> super::PendingDocumentPolicyBatch;
        fn start_document_runtime_policy_reconciliation(document: super::DocumentHandle, policy: super::DocumentRuntimePolicyReconciliation) -> super::PendingDocumentPolicyBatch;
    }

    forward_context_try_update! {
        fn start_document_policy_update(document: super::DocumentHandle, update: super::DocumentPolicyUpdate) -> super::PendingDocumentPolicyUpdate;
        fn finish_document_policy_update(completed: super::CompletedDocumentPolicyUpdate) -> ();
        fn finish_document_policy_batch(completed: super::CompletedDocumentPolicyBatch) -> ();
    }

    pub fn start_document_policy_batch_with_surface(
        &self,
        document: super::DocumentHandle,
        updates: Vec<super::DocumentPolicyUpdate>,
        foreground: bool,
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
        global_geolocation: Option<&super::EmulatedGeolocationOverrideState>,
    ) -> super::PendingDocumentPolicyBatch {
        let global_geolocation = global_geolocation.cloned();
        self.update_live(move |context| {
            context.start_document_policy_batch_with_surface(
                document,
                updates,
                foreground,
                global_network_conditions,
                global_geolocation.as_ref(),
            )
        })
    }

    pub fn start_document_page_surface_update(
        &self,
        document: super::DocumentHandle,
        foreground: bool,
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
        global_geolocation: Option<&super::EmulatedGeolocationOverrideState>,
    ) -> Result<super::PendingDocumentPolicyBatch, String> {
        let global_geolocation = global_geolocation.cloned();
        self.try_update(move |context| {
            context.start_document_page_surface_update(
                document,
                foreground,
                global_network_conditions,
                global_geolocation.as_ref(),
            )
        })
    }

    pub fn page_surface_for_web_contents(
        &self,
        handle: WebContentsHandle,
        foreground: bool,
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
        global_geolocation: Option<&super::EmulatedGeolocationOverrideState>,
    ) -> Result<super::web_contents::PageSurface, String> {
        let global_geolocation = global_geolocation.cloned();
        self.try_read(move |context| {
            context.page_surface_for_web_contents(
                handle,
                foreground,
                global_network_conditions,
                global_geolocation.as_ref(),
            )
        })
    }

    forward_context_read! {
        fn web_contents_has_navigation_engine(handle: WebContentsHandle) -> bool;
        fn web_contents_navigation_layout_policy(handle: WebContentsHandle) -> Option<crate::LayoutPolicy>;
        fn web_contents_navigation_renderer_owner_id(handle: WebContentsHandle) -> Option<u64>;
        fn web_contents_navigation_diagnostics(handle: WebContentsHandle) -> Option<crate::runtime::NavigationEngineDiagnostics>;
    }

    forward_context_try_read! {
        fn start_document_fetch_command(document: super::DocumentHandle, command: super::DocumentFetchCommand) -> super::PendingDocumentFetchCommand;
    }

    forward_context_try_update! {
        fn start_web_contents_fetch_interception_update(web_contents: WebContentsHandle, enabled: bool, resource_type: Option<crate::page::SubresourceResourceType>, accept_stale_completion: bool) -> Option<super::PendingDocumentFetchCommand>;
        fn install_web_contents_fetch_interception_policy(web_contents: WebContentsHandle, enabled: bool, resource_type: Option<crate::page::SubresourceResourceType>) -> ();
        fn finish_document_fetch_command(completed: super::CompletedDocumentFetchCommand) -> super::DocumentFetchCommandOutcome;
        fn finish_document_resource_runtime_update(completed: super::CompletedDocumentResourceRuntimeUpdate) -> ();
    }

    pub fn web_contents_navigation_fetch_config(
        &self,
        handle: WebContentsHandle,
    ) -> Option<moli_fetch::FetchConfig> {
        self.read_live(move |context| {
            context
                .web_contents_navigation_fetch_config(handle)
                .cloned()
        })
    }

    pub fn configure_selected_navigation_policy(
        &self,
        defaults: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
    ) -> Result<(), String> {
        let global_headers = global_headers.to_vec();
        self.try_update(move |context| {
            context.configure_selected_navigation_policy(
                defaults,
                &global_headers,
                global_network_conditions,
            )
        })
    }

    pub fn ensure_web_contents_resource_request_client(
        &self,
        web_contents: WebContentsHandle,
        defaults: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
    ) -> Result<crate::network::ResourceRequestClient, String> {
        let global_headers = global_headers.to_vec();
        self.try_update(move |context| {
            context.ensure_web_contents_resource_request_client(
                web_contents,
                defaults,
                &global_headers,
                global_network_conditions,
            )
        })
    }

    pub fn start_web_contents_resource_runtime_rebuild(
        &self,
        web_contents: WebContentsHandle,
        defaults: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
    ) -> Result<Option<super::PendingDocumentResourceRuntimeUpdate>, String> {
        let global_headers = global_headers.to_vec();
        self.try_update(move |context| {
            context.start_web_contents_resource_runtime_rebuild(
                web_contents,
                defaults,
                &global_headers,
                global_network_conditions,
            )
        })
    }

    forward_context_read! {
        fn selected_document_handle() -> Option<super::DocumentHandle>;
        fn selected_document_navigation_metadata() -> Option<super::DocumentNavigationMetadata>;
    }

    forward_context_try_read! {
        fn web_contents_tls_verify_host_override(handle: WebContentsHandle) -> Option<bool>;
        fn web_contents_bypass_content_security_policy(handle: WebContentsHandle) -> bool;
        fn web_contents_browser_identity_override(handle: WebContentsHandle) -> Option<moli_browser_profile::BrowserIdentityProfile>;
        fn web_contents_network_request_policy(handle: WebContentsHandle) -> super::web_contents::NetworkRequestPolicy;
        fn web_contents_locale_override(handle: WebContentsHandle) -> Option<String>;
        fn web_contents_timezone_override(handle: WebContentsHandle) -> Option<String>;
        fn web_contents_has_non_default_policy(handle: WebContentsHandle) -> bool;
        fn web_contents_opener(handle: WebContentsHandle) -> Option<(super::WebContentsId, bool)>;
    }

    forward_context_try_update! {
        fn set_web_contents_tls_verify_host_override(handle: WebContentsHandle, enabled: Option<bool>) -> ();
        fn set_web_contents_bypass_content_security_policy(handle: WebContentsHandle, bypass: bool) -> ();
        fn set_web_contents_browser_identity_override(handle: WebContentsHandle, identity: Option<moli_browser_profile::BrowserIdentityProfile>) -> ();
        fn set_web_contents_network_request_policy(handle: WebContentsHandle, policy: super::web_contents::NetworkRequestPolicy) -> ();
        fn set_web_contents_locale_override(handle: WebContentsHandle, locale: Option<String>) -> ();
        fn set_web_contents_timezone_override(handle: WebContentsHandle, timezone: Option<String>) -> ();
    }

    pub fn inherited_document_policy(
        &self,
        fetch_config: moli_fetch::FetchConfig,
        global_headers: &[(String, String)],
        global_network_conditions: Option<super::EmulatedNetworkConditions>,
        global_geolocation: Option<&super::EmulatedGeolocationOverrideState>,
    ) -> super::web_contents::InheritedDocumentPolicy {
        let id = self.id;
        let global_headers = global_headers.to_vec();
        let global_geolocation = global_geolocation.cloned();
        self.browser
            .execute(move |browser| {
                browser.context(id).map(|context| {
                    context.inherited_document_policy(
                        fetch_config,
                        &browser.permission_defaults,
                        &global_headers,
                        global_network_conditions,
                        global_geolocation.as_ref(),
                    )
                })
            })
            .expect("live Browser owner must accept a policy snapshot")
            .expect("live BrowserContext handle must resolve in its owner")
    }

    pub fn permission_snapshot(&self) -> Vec<crate::page::PermissionOverrideRegistration> {
        let id = self.id;
        self.browser
            .execute(move |browser| {
                browser
                    .context(id)
                    .map(|context| context.permission_snapshot(&browser.permission_defaults))
            })
            .expect("live Browser owner must accept a permission snapshot")
            .expect("live BrowserContext handle must resolve in its owner")
    }

    pub fn set_permission_override(
        &self,
        registration: crate::page::PermissionOverrideRegistration,
    ) -> Result<(), String> {
        let id = self.id;
        self.browser.execute(move |browser| {
            let Browser {
                contexts,
                permission_defaults,
                ..
            } = browser;
            let context = contexts
                .get_mut(&id)
                .ok_or_else(|| "BrowserContext unavailable".to_owned())?;
            context.set_permission_override(permission_defaults, registration);
            Ok(())
        })?
    }

    pub fn start_permission_update(
        &self,
    ) -> Result<Option<super::PendingContextPermissionUpdate>, String> {
        let id = self.id;
        self.browser.execute(move |browser| {
            browser
                .context(id)?
                .start_permission_update(&browser.permission_defaults)
        })?
    }

    forward_context_try_update! {
        fn finish_permission_update(completed: super::CompletedContextPermissionUpdate) -> ();
    }

    #[cfg(any(test, feature = "test-support"))]
    async fn run_local_test_operation<R: Send + 'static>(
        &self,
        operation: impl FnOnce(
            BrowserContext,
        ) -> Pin<Box<dyn Future<Output = (BrowserContext, R)> + 'static>>
        + Send
        + 'static,
    ) -> Result<R, String> {
        let id = self.id;
        let completion = self
            .browser
            .execute(move |browser| browser.spawn_local_context_operation(id, operation))??;
        completion
            .await
            .map_err(|_| "Browser owner stopped before completing local Context work".to_owned())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn document_runtime_heap_usage_for_test(
        &self,
        document: super::DocumentHandle,
    ) -> Result<crate::page::RendererRuntimeHeapUsage, String> {
        self.run_local_test_operation(move |mut context| {
            Box::pin(async move {
                let result = context.document_runtime_heap_usage_for_test(document).await;
                (context, result)
            })
        })
        .await?
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn evaluate_document_expression_for_test(
        &self,
        document: super::DocumentHandle,
        expression: &str,
        await_promise: bool,
    ) -> Result<serde_json::Value, String> {
        let expression = expression.to_owned();
        self.run_local_test_operation(move |mut context| {
            Box::pin(async move {
                let result = context
                    .evaluate_document_expression_for_test(document, &expression, await_promise)
                    .await;
                (context, result)
            })
        })
        .await?
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn reset_selected_resource_runtime_for_test(&self) -> bool {
        self.run_local_test_operation(|mut context| {
            Box::pin(async move {
                let reset = context.reset_selected_resource_runtime_for_test().await;
                (context, reset)
            })
        })
        .await
        .unwrap_or(false)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn apply_document_cookie_facade_overrides_for_test(
        &self,
        document: super::DocumentHandle,
        overrides: Option<moli_cookie_jar::BrowserCookieFacadeOverrides>,
    ) -> Result<(), String> {
        self.run_local_test_operation(move |mut context| {
            Box::pin(async move {
                let result = context
                    .apply_document_cookie_facade_overrides_for_test(document, overrides)
                    .await;
                (context, result)
            })
        })
        .await?
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn capture_document_policy_for_test(
        &self,
        handle: WebContentsHandle,
        inherited: super::web_contents::InheritedDocumentPolicy,
        final_url: &url::Url,
    ) -> Result<crate::runtime::PreparedDocumentPagePolicy, String> {
        let final_url = final_url.clone();
        self.try_update(move |context| {
            context.capture_document_policy_for_test(handle, inherited, &final_url)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn renderer_runtime_id_for_test(&self) -> crate::RendererBrowserContextRuntimeId {
        self.read_live(BrowserContext::renderer_runtime_id_for_test)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_idle_override_for_test(
        &self,
        document: super::DocumentHandle,
    ) -> Result<Option<crate::page::EmulatedIdleOverride>, String> {
        self.try_read(move |context| context.document_idle_override_for_test(document))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_contents_fetch_interception_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Result<(bool, Option<crate::page::SubresourceResourceType>), String> {
        self.try_read(move |context| context.web_contents_fetch_interception_for_test(handle))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn has_paused_navigation_auth_for_test(&self, handle: WebContentsHandle) -> bool {
        self.read_live(move |context| context.has_paused_navigation_auth_for_test(handle))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn record_navigation_history_for_test(
        &self,
        handle: WebContentsHandle,
        snapshot: (String, String),
    ) -> Result<(), String> {
        self.try_update(move |context| context.record_navigation_history_for_test(handle, snapshot))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_web_contents_opener_for_test(
        &self,
        handle: WebContentsHandle,
        opener: super::WebContentsId,
        can_access: bool,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.set_web_contents_opener_for_test(handle, opener, can_access)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn pending_document_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<(super::NavigationId, super::DocumentId)>, String> {
        self.try_read(move |context| context.pending_document(handle))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn install_document_lifecycle_for_test(
        &self,
        document: super::DocumentHandle,
        lifecycle: super::DocumentLifecycle,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.install_document_lifecycle_for_test(document, lifecycle)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn apply_document_lifecycle_for_test(
        &self,
        document: super::DocumentHandle,
        event: crate::page::RendererDocumentLifecycleEvent,
    ) -> Result<bool, String> {
        self.try_update(move |context| context.apply_document_lifecycle_for_test(document, event))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn replace_document_identity_for_test(
        &self,
        document: super::DocumentHandle,
        replacement: super::DocumentId,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.replace_document_identity_for_test(document, replacement)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn clear_web_contents_javascript_dialogs_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.clear_web_contents_javascript_dialogs_for_test(handle)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn mark_initial_empty_document_materialized_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.try_update(move |context| {
            context.mark_initial_empty_document_materialized_for_test(handle)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn mark_initial_empty_document_exited_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.try_update(move |context| context.mark_initial_empty_document_exited_for_test(handle))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn accepts_any_document_body_completion_for_test(
        &self,
        navigation: &super::NavigationId,
    ) -> bool {
        let navigation = *navigation;
        self.read_live(move |context| {
            context.accepts_any_document_body_completion_for_test(&navigation)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn service_worker_force_update_on_page_load_for_test(&self) -> bool {
        self.read_live(BrowserContext::service_worker_force_update_on_page_load_for_test)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn service_worker_pause_on_start_for_test(&self) -> bool {
        self.read_live(BrowserContext::service_worker_pause_on_start_for_test)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn resource_storage_handles_for_test(&self) -> super::BrowserContextResourceStorageHandles {
        self.read_live(BrowserContext::resource_storage_handles_for_test)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn http_cache_configuration_for_test(&self) -> (Option<PathBuf>, Option<u64>) {
        self.read_live(|context| {
            let (path, max_bytes) = context.http_cache_configuration_for_test();
            (path.map(std::path::Path::to_path_buf), max_bytes)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn cookie_store_for_test(&self) -> moli_cookie_jar::SharedBrowserCookieStore {
        self.read_live(|context| context.cookie_store_for_test().clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_storage_store_for_test(&self) -> crate::network::SharedWebStorageStore {
        self.read_live(|context| context.web_storage_store_for_test().clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn selected_session_storage_store_for_test(
        &self,
    ) -> Option<crate::network::SharedWebStorageStore> {
        self.read_live(|context| context.selected_session_storage_store_for_test().cloned())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn indexed_db_manager_for_test(&self) -> crate::storage::SharedIndexedDbManager {
        self.read_live(|context| context.indexed_db_manager_for_test().clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn storage_bucket_store_for_test(&self) -> crate::storage::SharedStorageBucketStore {
        self.read_live(|context| context.storage_bucket_store_for_test().clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn replace_storage_bucket_store_for_test(
        &self,
        storage: crate::storage::SharedStorageBucketStore,
    ) {
        self.update_live(move |context| context.replace_storage_bucket_store_for_test(storage));
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_http_proxy_override_for_test(&self, proxy: Option<String>) {
        self.update_live(move |context| context.set_http_proxy_override_for_test(proxy));
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_renderer_devtools_agent_token_for_test(
        &self,
        document: super::DocumentHandle,
    ) -> Option<crate::page::RendererDevToolsAgentToken> {
        self.read_live(move |context| {
            context
                .page_for_test(document)
                .map(crate::page::Page::renderer_devtools_agent_token)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_response_status_for_test(
        &self,
        document: super::DocumentHandle,
    ) -> Result<u16, String> {
        self.try_read(move |context| context.document_response_status_for_test(document))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_script_execution_for_test(
        &self,
        document: super::DocumentHandle,
    ) -> Result<crate::page::ScriptExecutionReport, String> {
        self.try_read(move |context| context.document_script_execution_for_test(document))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn document_renderer_inspection_endpoint_for_test(
        &self,
        document: super::DocumentHandle,
    ) -> Result<moli_renderer_v8::RendererInspectionEndpoint, String> {
        self.try_read(move |context| {
            context.document_renderer_inspection_endpoint_for_test(document)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_contents_navigation_browser_context_runtime_id_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Option<crate::RendererBrowserContextRuntimeId> {
        self.read_live(move |context| {
            context.web_contents_navigation_browser_context_runtime_id_for_test(handle)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_capability_does_not_own_the_physical_context() {
        let service = BrowserService::start().expect("Browser owner should start");
        let handle = service
            .handle()
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .expect("context creation should succeed");
        assert!(handle.is_live());
        assert!(handle.remove().expect("context removal should succeed"));
        assert!(!handle.is_live());
        service.shutdown();
    }
}
