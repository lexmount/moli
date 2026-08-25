use axum::extract::ws::{Message, WebSocket};
use futures_util::StreamExt;
use std::time::Duration;

use crate::{cdp_frontend::CdpFrontendEndpoint, cdp_writer::spawn_socket_sink};

pub(super) enum CdpFrontendSocketKind {
    Browser,
    Page { target_id: String },
    ForegroundTab { page_target_id: String },
}

pub(super) async fn run_cdp_frontend_socket(
    socket: WebSocket,
    endpoint: CdpFrontendEndpoint,
    kind: CdpFrontendSocketKind,
) -> bool {
    let (socket_tx, mut socket_rx) = socket.split();
    let (sink, mut writer_finished_rx) = spawn_socket_sink(socket_tx);
    let socket_sink = sink.clone();
    let frontend_id = match &kind {
        CdpFrontendSocketKind::Browser => endpoint.attach_browser(sink).await,
        CdpFrontendSocketKind::Page { target_id } => {
            endpoint.attach_page(target_id.clone(), sink).await
        }
        CdpFrontendSocketKind::ForegroundTab { page_target_id } => {
            endpoint
                .attach_foreground_tab(page_target_id.clone(), sink)
                .await
        }
    };
    let frontend_id = match frontend_id {
        Ok(frontend_id) => frontend_id,
        Err(error) => {
            tracing::warn!(?error, "failed to attach CDP WebSocket frontend");
            return false;
        }
    };
    let mut detach_guard = CdpFrontendDetachGuard {
        endpoint: endpoint.clone(),
        frontend_id: Some(frontend_id),
        kind: match kind {
            CdpFrontendSocketKind::Browser => CdpFrontendKind::Browser,
            CdpFrontendSocketKind::Page { .. } => CdpFrontendKind::Page,
            CdpFrontendSocketKind::ForegroundTab { .. } => CdpFrontendKind::ForegroundTab,
        },
    };

    let exit = loop {
        tokio::select! {
            _ = endpoint.wait_for_shutdown() => break CdpSocketExit::TransportClose,
            _ = &mut writer_finished_rx => break CdpSocketExit::WriterFinished,
            maybe_message = socket_rx.next() => {
                let Some(Ok(message)) = maybe_message else {
                    break CdpSocketExit::TransportClose;
                };
                match message {
                    Message::Text(text) => {
                        if !endpoint.command(frontend_id, text.to_string()).await {
                            break CdpSocketExit::TransportClose;
                        }
                    }
                    Message::Close(_) => break CdpSocketExit::PeerClose,
                    _ => {}
                }
            }
        }
    };
    detach_guard.detach();
    match exit {
        CdpSocketExit::WriterFinished => wait_for_peer_close(&mut socket_rx).await,
        CdpSocketExit::PeerClose => {
            socket_sink.peer_close_received();
            let _ = writer_finished_rx.await;
        }
        CdpSocketExit::TransportClose => {
            socket_sink.close();
            let _ = writer_finished_rx.await;
        }
    }
    true
}

enum CdpSocketExit {
    WriterFinished,
    PeerClose,
    TransportClose,
}

async fn wait_for_peer_close(socket_rx: &mut futures_util::stream::SplitStream<WebSocket>) {
    let wait = async {
        while let Some(message) = socket_rx.next().await {
            match message {
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(1), wait).await;
}

enum CdpFrontendKind {
    Browser,
    Page,
    ForegroundTab,
}

struct CdpFrontendDetachGuard {
    endpoint: CdpFrontendEndpoint,
    frontend_id: Option<u64>,
    kind: CdpFrontendKind,
}

impl CdpFrontendDetachGuard {
    fn detach(&mut self) {
        let Some(frontend_id) = self.frontend_id.take() else {
            return;
        };
        match self.kind {
            CdpFrontendKind::Browser => self.endpoint.detach_browser(frontend_id),
            CdpFrontendKind::Page => self.endpoint.detach_page(frontend_id),
            CdpFrontendKind::ForegroundTab => {
                self.endpoint.detach_foreground_tab(frontend_id);
            }
        }
    }
}

impl Drop for CdpFrontendDetachGuard {
    fn drop(&mut self) {
        self.detach();
    }
}
