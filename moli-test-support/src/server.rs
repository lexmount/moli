use super::*;

pub struct FixtureServer {
    addr: std::net::SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl FixtureServer {
    pub async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind fixture server")?;
        let addr = listener
            .local_addr()
            .context("failed to read fixture server address")?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let app = server_routes::build_router().layer(Extension(FixtureRuntimeState::default()));

        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                panic!("fixture server failed: {error}");
            }
        });

        Ok(Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
            task: Some(task),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn release_runtime_owned_in_order_error_after_dcl(&self) {
        crate::routes_core::notify_runtime_owned_in_order_error_after_dcl_gate(
            &self.addr.to_string(),
        );
    }

    pub fn release_runtime_owned_in_order_load_after_dcl(&self) {
        crate::routes_core::notify_runtime_owned_in_order_load_after_dcl_gate(
            &self.addr.to_string(),
        );
    }

    pub async fn shutdown(mut self) {
        crate::routes_core::clear_runtime_inserted_stylesheet_href_mutation_gate(
            &self.addr.to_string(),
        );
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        crate::routes_core::clear_runtime_inserted_stylesheet_href_mutation_gate(
            &self.addr.to_string(),
        );
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
