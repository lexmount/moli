use std::{sync::atomic::Ordering, time::Duration};

use super::*;

impl ServiceWorkerRuntimeService {
    fn idle_delay(&self) -> Duration {
        Duration::from_millis(self.inner.idle_delay_ms.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(super) fn set_idle_delay_for_test(&self, delay: Duration) {
        let delay_ms = delay.as_millis().min(u128::from(u64::MAX)) as u64;
        self.inner.idle_delay_ms.store(delay_ms, Ordering::Relaxed);
    }

    pub(super) fn schedule_idle_timeout(&self, timeout: ServiceWorkerIdleTimeout) {
        let delay = self.idle_delay();
        let service = self.downgrade();
        if delay.is_zero() {
            enqueue_idle_timeout_after_delay(service, timeout);
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                tokio::time::sleep(delay).await;
                enqueue_idle_timeout_after_delay(service, timeout);
            });
            return;
        }
        let _ = std::thread::Builder::new()
            .name(format!(
                "service-worker-idle-{}",
                timeout.version_id.as_u64()
            ))
            .spawn(move || {
                std::thread::sleep(delay);
                enqueue_idle_timeout_after_delay(service, timeout);
            });
    }
}

fn enqueue_idle_timeout_after_delay(
    service: WeakServiceWorkerRuntimeService,
    timeout: ServiceWorkerIdleTimeout,
) {
    let Some(service) = service.upgrade() else {
        return;
    };
    service.enqueue_worker_idle_timeout(timeout);
}
