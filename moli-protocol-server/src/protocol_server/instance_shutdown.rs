use tokio::sync::watch;

/// One server instance's persistent stop signal and admitted WebSocket workers.
/// It is not an ownership or authentication mechanism for protocol clients.
#[derive(Clone, Debug)]
pub(super) struct InstanceShutdown {
    state: watch::Sender<ShutdownState>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ShutdownState {
    requested: bool,
    connections: usize,
}

impl Default for InstanceShutdown {
    fn default() -> Self {
        let (state, _) = watch::channel(ShutdownState::default());
        Self { state }
    }
}

impl InstanceShutdown {
    pub(super) fn request(&self) {
        self.state.send_modify(|state| state.requested = true);
    }

    pub(super) async fn requested(&self) {
        let mut receiver = self.state.subscribe();
        let _ = receiver.wait_for(|state| state.requested).await;
    }

    pub(super) fn connection(&self) -> Option<InstanceConnection> {
        let mut admitted = false;
        self.state.send_if_modified(|state| {
            if state.requested {
                return false;
            }
            let Some(count) = state.connections.checked_add(1) else {
                return false;
            };
            state.connections = count;
            admitted = true;
            true
        });
        admitted.then(|| InstanceConnection(self.clone()))
    }

    pub(super) async fn drained(&self) {
        let mut receiver = self.state.subscribe();
        let _ = receiver.wait_for(|state| state.connections == 0).await;
    }
}

pub(super) struct InstanceConnection(InstanceShutdown);

impl Drop for InstanceConnection {
    fn drop(&mut self) {
        self.0.state.send_modify(|state| state.connections -= 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn shutdown_is_persistent_and_repeated_requests_do_not_admit_connections() {
        let instance = InstanceShutdown::default();
        let connection = instance.connection().expect("initial connection admitted");
        instance.request();
        instance.request();
        timeout(Duration::from_millis(100), instance.requested())
            .await
            .expect("late subscriber observes stop");
        assert!(instance.connection().is_none());
        assert!(
            timeout(Duration::from_millis(10), instance.drained())
                .await
                .is_err()
        );
        drop(connection);
        timeout(Duration::from_millis(100), instance.drained())
            .await
            .expect("all workers released");
    }

    #[tokio::test]
    async fn shutdown_does_not_affect_another_instance() {
        let first = InstanceShutdown::default();
        let second = InstanceShutdown::default();
        first.request();
        assert!(second.connection().is_some());
        assert!(
            timeout(Duration::from_millis(10), second.requested())
                .await
                .is_err()
        );
    }
}
