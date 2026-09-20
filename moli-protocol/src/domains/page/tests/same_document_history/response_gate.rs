use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(super) struct ResponseGate {
    pub(super) requests: AtomicUsize,
    pub(super) responses: AtomicUsize,
    pub(super) release: tokio::sync::Semaphore,
}

impl ResponseGate {
    pub(super) async fn wait_for(counter: &AtomicUsize, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while counter.load(Ordering::SeqCst) < expected {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("navigation request must reach the response gate");
    }
}
