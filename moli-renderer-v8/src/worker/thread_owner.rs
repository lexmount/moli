use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

use parking_lot::Mutex;

use super::handle::WorkerThread;

/// The physical Context retains OS-thread ownership even after a Worker handle
/// is retired. Worker VMs only inherit a weak, closeable spawn capability.
#[derive(Debug, Default)]
pub(crate) struct WorkerThreadOwner {
    state: Arc<Mutex<WorkerThreads>>,
}

#[derive(Debug, Default)]
pub(crate) struct WorkerThreads {
    closed: bool,
    threads: Vec<Arc<WorkerThread>>,
}

#[derive(Clone, Debug)]
pub(crate) enum WorkerThreadRegistrar {
    Owned(Weak<Mutex<WorkerThreads>>),
    #[cfg(test)]
    Standalone,
}

impl Default for WorkerThreadRegistrar {
    fn default() -> Self {
        #[cfg(test)]
        return Self::Standalone;
        #[cfg(not(test))]
        Self::Owned(Weak::new())
    }
}

impl WorkerThreadOwner {
    pub(crate) fn registrar(&self) -> WorkerThreadRegistrar {
        WorkerThreadRegistrar::Owned(Arc::downgrade(&self.state))
    }

    pub(crate) fn terminate_all(&self) {
        let threads = {
            let mut state = self.state.lock();
            state.closed = true;
            state.threads.clone()
        };
        // Neither V8 interruption nor join runs under the registry lock.
        for thread in threads {
            thread.request_termination();
        }
    }

    pub(crate) fn shutdown_and_join(&mut self) {
        self.terminate_all();
        let threads = std::mem::take(&mut self.state.lock().threads);
        for thread in threads {
            thread.join();
        }
    }
}

impl Drop for WorkerThreadOwner {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

impl WorkerThreadRegistrar {
    pub(crate) fn spawn(&self, thread: &Arc<WorkerThread>, spawn: impl FnOnce() -> JoinHandle<()>) {
        match self {
            Self::Owned(owner) => {
                if let Some(owner) = owner.upgrade() {
                    let mut state = owner.lock();
                    if !state.closed {
                        state.threads.retain(|thread| !thread.reap_finished());
                        // Admission, OS spawn and registration are atomic with
                        // shutdown, including nested Worker creation.
                        thread.set_join_handle(spawn());
                        state.threads.push(Arc::clone(thread));
                        return;
                    }
                }
                thread.request_termination();
            }
            #[cfg(test)]
            Self::Standalone => thread.set_join_handle(spawn()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicBool, mpsc};
    use std::time::Duration;

    use super::*;
    use crate::worker::{WorkerDevToolsHandle, WorkerHandle, WorkerMessage};

    fn thread_control() -> (
        Arc<WorkerThread>,
        WorkerHandle,
        tokio::sync::mpsc::UnboundedReceiver<WorkerMessage>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (_, parent_rx) = tokio::sync::mpsc::unbounded_channel();
        let isolate = Arc::new(Mutex::new(None));
        let devtools = WorkerDevToolsHandle::new(tx.clone(), Arc::clone(&isolate));
        let thread = WorkerThread::new(isolate, Arc::new(AtomicBool::new(false)), devtools);
        let handle = WorkerHandle::from_thread(tx, parent_rx, Arc::clone(&thread));
        (thread, handle, rx)
    }

    fn threads(registrar: &WorkerThreadRegistrar) -> Vec<Arc<WorkerThread>> {
        let WorkerThreadRegistrar::Owned(owner) = registrar else {
            panic!("expected a physical Context owner");
        };
        owner.upgrade().unwrap().lock().threads.clone()
    }

    #[test]
    fn dropped_handle_is_nonblocking_and_owner_joins_retired_thread() {
        let mut owner = WorkerThreadOwner::default();
        let (thread, handle, mut rx) = thread_control();
        let (release_tx, release_rx) = mpsc::channel();
        let (exited_tx, exited_rx) = mpsc::channel();
        owner.registrar().spawn(&thread, || {
            std::thread::spawn(move || {
                assert!(matches!(rx.blocking_recv(), Some(WorkerMessage::Terminate)));
                release_rx.recv().unwrap();
                exited_tx.send(()).unwrap();
            })
        });
        drop(handle);
        assert!(!thread.is_joined());
        release_tx.send(()).unwrap();
        owner.shutdown_and_join();
        assert!(thread.is_joined());
        exited_rx
            .try_recv()
            .expect("owner must wait for thread exit");
        assert!(threads(&owner.registrar()).is_empty());
    }

    #[test]
    fn terminal_owner_rejects_nested_and_escaped_spawn_capabilities() {
        let mut owner = WorkerThreadOwner::default();
        let registrar = owner.registrar();
        let nested_registrar = registrar.clone();
        let (thread, handle, mut rx) = thread_control();
        let (done_tx, done_rx) = mpsc::channel();
        registrar.spawn(&thread, || {
            std::thread::spawn(move || {
                assert!(matches!(rx.blocking_recv(), Some(WorkerMessage::Terminate)));
                let (child, _handle, mut child_rx) = thread_control();
                nested_registrar.spawn(&child, || panic!("nested admission reopened"));
                assert!(matches!(child_rx.try_recv(), Ok(WorkerMessage::Terminate)));
                done_tx.send(()).unwrap();
            })
        });
        owner.shutdown_and_join();
        assert!(thread.is_joined());
        done_rx
            .try_recv()
            .expect("nested rejection must complete without panicking");
        drop(handle);
        drop(owner);
        let (late, _handle, mut rx) = thread_control();
        registrar.spawn(&late, || panic!("escaped admission reopened"));
        assert!(matches!(rx.try_recv(), Ok(WorkerMessage::Terminate)));
    }

    #[test]
    fn admission_reaps_joined_thread_history_without_retaining_handles() {
        let mut owner = WorkerThreadOwner::default();
        for _ in 0..32 {
            let (thread, handle, mut rx) = thread_control();
            owner.registrar().spawn(&thread, || {
                std::thread::spawn(move || {
                    assert!(matches!(rx.blocking_recv(), Some(WorkerMessage::Terminate)));
                })
            });
            handle.terminate_and_join();
            assert!(thread.is_joined());
            assert_eq!(threads(&owner.registrar()).len(), 1);
        }
        owner.shutdown_and_join();
        assert!(threads(&owner.registrar()).is_empty());
    }

    #[tokio::test]
    async fn context_owner_joins_all_worker_kinds_and_nested_threads_with_live_handles() {
        use crate::network::ResourceRequestClient;
        use crate::runtime::{
            RendererBrowserContextRuntime, ServiceWorkerRegistrationId, ServiceWorkerVersionId,
        };
        use crate::worker::{
            WorkerGlobalKind, WorkerSpawnOptions, WorkerToParentMessage, spawn_worker_with_options,
        };

        crate::ensure_v8_for_test();
        let mut owner = RendererBrowserContextRuntime::new();
        let peer = RendererBrowserContextRuntime::new();
        let context = owner.worker_context_runtime();
        let client =
            ResourceRequestClient::from_browser_resource_runtime(owner.browser_resource_runtime());
        let url = url::Url::parse("https://worker-owner.test/worker.js").unwrap();
        let kinds = [
            WorkerGlobalKind::Dedicated {
                name: String::new(),
            },
            WorkerGlobalKind::Shared {
                name: String::new(),
                storage_key: moli_storage_key::MoliStorageKey::first_party_from_url(&url, None),
            },
            WorkerGlobalKind::Service {
                registration_id: ServiceWorkerRegistrationId::from_u64_for_test(1),
                version_id: ServiceWorkerVersionId::from_u64_for_test(1),
                scope_url: url.clone(),
            },
        ];
        let mut handles = Vec::new();
        for kind in kinds {
            let source = if matches!(kind, WorkerGlobalKind::Dedicated { .. }) {
                r#"const child = new Worker('data:text/javascript,' + encodeURIComponent(
                    'postMessage("ready"); while (true) {}'));
                child.onmessage = () => console.log('ready');"#
            } else {
                "console.log('ready'); while (true) {}"
            };
            let mut handle = spawn_worker_with_options(
                WorkerSpawnOptions::new_with_request_client(
                    source.into(),
                    url.to_string(),
                    client.clone(),
                )
                .with_worker_context_runtime(context.clone())
                .with_global_kind(kind),
            );
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    match handle.recv().await.expect("worker must report readiness") {
                        WorkerToParentMessage::Console(message)
                            if message.message == "log: ready" =>
                        {
                            break;
                        }
                        WorkerToParentMessage::Error { message, .. } => {
                            panic!("worker failed: {message}")
                        }
                        _ => {}
                    }
                }
            })
            .await
            .expect("worker must enter its running state");
            handles.push(handle);
        }
        let peer_handle = spawn_worker_with_options(
            WorkerSpawnOptions::new_with_request_client(
                "onmessage = () => postMessage('peer');".into(),
                url.to_string(),
                ResourceRequestClient::from_browser_resource_runtime(
                    peer.browser_resource_runtime(),
                ),
            )
            .with_worker_context_runtime(peer.worker_context_runtime()),
        );
        let owned_threads = threads(&context.worker_threads);
        assert_eq!(
            owned_threads.len(),
            4,
            "nested Worker must inherit the same owner"
        );
        // SharedWorker retirement must not remove its OS join obligation.
        drop(handles.remove(1));
        owner.shutdown_and_join();
        assert!(owned_threads.iter().all(|thread| thread.is_joined()));
        assert!(threads(&context.worker_threads).is_empty());
        assert!(
            threads(&peer.worker_context_runtime().worker_threads)
                .iter()
                .all(|thread| !thread.is_joined())
        );
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        assert!(peer_handle.devtools_handle().dispatch_runtime_protocol_message(
            None,
            r#"{"id":71,"method":"Runtime.evaluate","params":{"expression":"41 + 1","returnByValue":true}}"#.into(),
            None,
            reply_tx,
        ));
        let replies = tokio::time::timeout(Duration::from_secs(3), reply_rx)
            .await
            .expect("peer must still execute after another Context shuts down")
            .expect("peer reply channel")
            .expect("peer evaluation");
        assert!(
            replies
                .into_iter()
                .map(crate::runtime::RendererRuntimeInspectorMessage::into_v8_inspector_message)
                .any(|reply| reply["id"] == 71 && reply["result"]["result"]["value"] == 42)
        );

        let mut late = spawn_worker_with_options(
            WorkerSpawnOptions::new_with_request_client(
                "throw new Error('must not run');".into(),
                url.to_string(),
                client,
            )
            .with_worker_context_runtime(context),
        );
        assert!(
            late.recv().await.is_none(),
            "stale spawn must close without starting an OS thread"
        );
        peer_handle.terminate_and_join();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn context_shutdown_unblocks_and_joins_synchronous_worker_network_boundaries() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        use crate::network::ResourceRequestClient;
        use crate::runtime::RendererBrowserContextRuntime;
        use crate::worker::{WorkerSpawnOptions, spawn_worker_with_options};

        crate::ensure_v8_for_test();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (retire_tx, retire_rx) = mpsc::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let owner_thread = std::thread::spawn(move || {
            let mut owner = RendererBrowserContextRuntime::new();
            let context = owner.worker_context_runtime();
            let client = ResourceRequestClient::from_browser_resource_runtime(
                owner.browser_resource_runtime(),
            );
            let scripts = [
                format!(
                    "const xhr = new XMLHttpRequest(); xhr.open('GET', '{base}/xhr', false); xhr.send();"
                ),
                format!("importScripts('{base}/import.js');"),
            ];
            let handles = scripts
                .into_iter()
                .map(|script| {
                    spawn_worker_with_options(
                        WorkerSpawnOptions::new_with_request_client(
                            script,
                            format!("{base}/worker.js"),
                            client.clone(),
                        )
                        .with_worker_context_runtime(context.clone()),
                    )
                })
                .collect::<Vec<_>>();
            let owned_threads = threads(&context.worker_threads);
            assert_eq!(owned_threads.len(), 2);
            retire_rx.recv().unwrap();
            owner.shutdown_and_join();
            let _ = done_tx.send(owned_threads.iter().all(|thread| thread.is_joined()));
            drop(handles);
        });

        let mut pending = Vec::new();
        let mut paths = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .expect("both Workers must enter the blocking network boundary")
                .unwrap();
            let mut request = Vec::new();
            tokio::time::timeout(Duration::from_secs(3), async {
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                }
            })
            .await
            .expect("request headers");
            paths.insert(
                String::from_utf8(request)
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned(),
            );
            pending.push(socket);
        }
        assert_eq!(paths, ["/import.js".to_owned(), "/xhr".to_owned()].into());
        retire_tx.send(()).unwrap();
        let completed = tokio::time::timeout(Duration::from_secs(3), done_rx).await;
        if !matches!(completed, Ok(Ok(true))) {
            // Release the held responses on failure so the regression reports
            // its assertion instead of leaving a deadlocked test process.
            for socket in &mut pending {
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
        }
        tokio::task::spawn_blocking(move || owner_thread.join())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(completed, Ok(Ok(true))),
            "shutdown must cancel transport before joining blocked Workers: {completed:?}"
        );
        for mut socket in pending {
            let mut byte = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), socket.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        }
    }
}
