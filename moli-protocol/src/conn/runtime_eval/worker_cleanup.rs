use anyhow::Result;

use super::*;
use anyhow::Context as _;

pub(crate) struct SessionWorkerCleanup {
    session: String,
    commands: Vec<(u64, Result<WorkerCleanupCall>)>,
}

enum WorkerCleanupCall {
    Shared(PendingSharedWorkerRuntimeProtocolMessageDispatch),
    Service(PendingServiceWorkerRuntimeProtocolMessageDispatch),
}

enum WorkerCleanupReply {
    Shared(CompletedSharedWorkerRuntimeProtocolMessageDispatch),
    Service(CompletedServiceWorkerRuntimeProtocolMessageDispatch),
}

pub(crate) struct CompletedSessionWorkerCleanup {
    session: String,
    replies: Vec<Result<CompletedWorkerCleanupCall>>,
}

struct CompletedWorkerCleanupCall {
    command_id: u64,
    reply: WorkerCleanupReply,
    response: Option<Result<moli_core::RendererRuntimeInspectorAsyncCompletion>>,
}

impl CdpConnection {
    pub(crate) fn prepare_session_worker_cleanup(
        &mut self,
        session_id: &str,
    ) -> SessionWorkerCleanup {
        let service_worker = matches!(
            self.session_route(Some(session_id)),
            Some(CdpSessionRoute::ServiceWorkerTarget { .. })
        );
        let plan = if service_worker {
            self.service_worker_target_for_session_mut(Some(session_id))
                .map(|target| target.take_runtime_remote_object_cleanup_plan(session_id))
        } else {
            self.shared_worker_target_for_session_mut(Some(session_id))
                .map(|target| target.take_runtime_remote_object_cleanup_plan(session_id))
        };
        let (groups, ids) = plan.unwrap_or_default();
        let payloads = groups
            .into_iter()
            .map(|group| ("Runtime.releaseObjectGroup", json!({"objectGroup": group})))
            .chain(
                ids.into_iter()
                    .map(|id| ("Runtime.releaseObject", json!({"objectId": id}))),
            );
        let mut command_id = SHARED_WORKER_RUNTIME_REMOTE_OBJECT_CLEANUP_COMMAND_ID_BASE;
        let mut commands = Vec::new();
        for (method, params) in payloads {
            let descriptor = RendererCommandDescriptor::for_document_helper(
                json!({"id": command_id, "method": method, "params": params}).to_string(),
            );
            let pending = descriptor.and_then(|descriptor| if service_worker {
                self.start_service_worker_runtime_protocol_message_for_session_with_deferred_response(Some(session_id), descriptor, command_id).map(WorkerCleanupCall::Service)
            } else {
                self.start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(Some(session_id), descriptor, command_id).map(WorkerCleanupCall::Shared)
            });
            commands.push((command_id, pending));
            command_id = command_id.saturating_add(1);
        }
        SessionWorkerCleanup {
            session: session_id.to_owned(),
            commands,
        }
    }
}

impl SessionWorkerCleanup {
    pub(crate) async fn wait(self) -> CompletedSessionWorkerCleanup {
        let mut replies = Vec::with_capacity(self.commands.len());
        for (command_id, pending) in self.commands {
            let reply = async {
                let (reply, receiver) = match pending? {
                    WorkerCleanupCall::Shared(pending) => {
                        let mut completed = pending.wait().await?;
                        let receiver = completed.take_deferred_response_receiver();
                        (WorkerCleanupReply::Shared(completed), receiver)
                    }
                    WorkerCleanupCall::Service(pending) => {
                        let mut completed = pending.wait().await?;
                        let receiver = completed.take_deferred_response_receiver();
                        (WorkerCleanupReply::Service(completed), receiver)
                    }
                };
                let response = match receiver {
                    Some(receiver) => Some(
                        receiver
                            .await
                            .context(RuntimeInspectorError::ResponseCanceled),
                    ),
                    None => None,
                };
                Ok(CompletedWorkerCleanupCall {
                    command_id,
                    reply,
                    response,
                })
            }
            .await;
            replies.push(reply);
        }
        CompletedSessionWorkerCleanup {
            session: self.session,
            replies,
        }
    }
}

impl CompletedSessionWorkerCleanup {
    pub(crate) fn finish(self, conn: &mut CdpConnection) {
        for reply in self.replies {
            let result = reply.and_then(|completed| {
                match completed.reply {
                    WorkerCleanupReply::Shared(reply) => {
                        conn.complete_shared_worker_runtime_protocol_message_for_session(reply)?;
                    }
                    WorkerCleanupReply::Service(reply) => {
                        conn.complete_service_worker_runtime_protocol_message_for_session(reply)?;
                    }
                }
                if let Some(response) = completed.response {
                    // The protocol-output adapter consumes the original call
                    // correlation even when its response channel has closed.
                    let _ = conn.resolve_runtime_inspector_response_ready(
                        RuntimeInspectorResponseReady::new(
                            completed.command_id,
                            Some(&self.session),
                            response,
                        ),
                    );
                }
                Ok(())
            });
            if let Err(error) = result {
                tracing::warn!(%error, "failed to release worker Runtime objects during target detach");
            }
        }
    }
}
