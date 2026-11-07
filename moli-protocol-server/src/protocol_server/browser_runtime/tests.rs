use super::*;
use crate::{InstanceCommand, ServerConfig};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

mod targets;

#[tokio::test]
async fn readiness_publishes_one_revocable_handle_per_instance() {
    let (ready, mut handles) = mpsc::unbounded_channel();
    for _ in 0..2 {
        let ready = ready.clone();
        let server = ProtocolServer::new(ServerConfig {
            host: "127.0.0.1".into(),
            port: 0,
            ..ServerConfig::default()
        })
        .with_runtime_ready(move |browser| {
            ready.send(browser).expect("runtime receiver");
        });
        let control = BrowserInstanceControl::new();
        let serving = server.clone();
        let stop = control.clone();
        let task = tokio::spawn(async move { serving.serve_with_control(stop).await });
        let browser = timeout(Duration::from_secs(5), handles.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!browser.targets().await.unwrap().is_empty());
        assert!(handles.try_recv().is_err(), "one notification per instance");
        control.request(InstanceCommand::Shutdown);
        timeout(Duration::from_secs(7), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(browser.targets().await.unwrap_err().code, 404);
    }
}

#[tokio::test]
async fn listener_bind_failure_never_publishes_a_runtime() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let notifications = Arc::new(AtomicUsize::new(0));
    let seen = notifications.clone();
    let server = ProtocolServer::new(ServerConfig {
        host: "127.0.0.1".into(),
        port: listener.local_addr().unwrap().port(),
        ..ServerConfig::default()
    })
    .with_runtime_ready(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    let error = server.serve().await.unwrap_err();
    assert!(error.to_string().contains("failed to bind protocol server"));
    assert_eq!(notifications.load(Ordering::SeqCst), 0);
}
