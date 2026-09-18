//! Submission capability for a specific owner, without thread ownership.

use super::{
    CurlWebSocketConnection, CurlWebSocketRequest, SESSION_CAPACITY, connection::SessionIo,
};
use crate::{
    CurlTransferId, HostResolveOverrides, NetworkAddressPolicy, SelectedProxy,
    runtime::identity::next_transfer_id,
};
use anyhow::{Context, Result, bail};
use curl::multi::MultiWaker;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Semaphore;
use url::{Host, Url};

pub(crate) struct Submission {
    pub(super) id: CurlTransferId,
    pub(super) request: CurlWebSocketRequest,
    pub(super) io: SessionIo,
}

/// Admission capability for one native owner. Clones do not keep that owner
/// alive; shutting down its runtime closes connections and rejects new ones.
#[derive(Clone, Debug)]
pub struct CurlWebSocketConnector {
    inner: Arc<ConnectorInner>,
    network_address_policy: NetworkAddressPolicy,
}

#[derive(Debug)]
struct ConnectorInner {
    submissions: crossbeam_channel::Sender<Submission>,
    slots: Arc<Semaphore>,
    waker: MultiWaker,
    shutdown: Arc<AtomicBool>,
}

impl CurlWebSocketConnector {
    #[cfg(test)]
    pub(super) fn available_session_slots(&self) -> usize {
        self.inner.slots.available_permits()
    }

    pub(crate) fn channel() -> (
        crossbeam_channel::Sender<Submission>,
        crossbeam_channel::Receiver<Submission>,
    ) {
        crossbeam_channel::bounded(SESSION_CAPACITY)
    }

    pub(crate) fn new(
        submissions: crossbeam_channel::Sender<Submission>,
        waker: MultiWaker,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            inner: Arc::new(ConnectorInner {
                submissions,
                slots: Arc::new(Semaphore::new(SESSION_CAPACITY)),
                waker,
                shutdown,
            }),
            network_address_policy: NetworkAddressPolicy::default(),
        }
    }

    pub fn with_network_address_policy(
        mut self,
        network_address_policy: NetworkAddressPolicy,
    ) -> Self {
        self.network_address_policy = network_address_policy;
        self
    }

    pub fn connect(&self, mut request: CurlWebSocketRequest) -> Result<CurlWebSocketConnection> {
        if self.inner.shutdown.load(Ordering::Acquire) {
            bail!("curl WebSocket runtime is closed");
        }
        self.apply_network_address_policy(&mut request)?;
        let slot = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .context("too many curl WebSocket sessions")?;
        let id = next_transfer_id()?;
        let (connection, io) = CurlWebSocketConnection::channel(id, self.inner.waker.clone(), slot);
        self.inner
            .submissions
            .try_send(Submission { id, request, io })
            .map_err(|_| anyhow::anyhow!("curl WebSocket runtime cannot accept a session"))?;
        let _ = self.inner.waker.wakeup();
        Ok(connection)
    }

    fn apply_network_address_policy(&self, request: &mut CurlWebSocketRequest) -> Result<()> {
        if !self.network_address_policy.is_enforced() {
            return Ok(());
        }

        let url = Url::parse(&request.url)
            .with_context(|| format!("failed to parse WebSocket URL `{}`", request.url))?;
        // A validated remote-DNS proxy owns request-target admission. An empty
        // proxy string disables proxying in curl. Chromium-style socks/socks5
        // and socks4 URLs are normalized to curl's remote-DNS variants here.
        if let Some(proxy) = request.proxy.as_deref().filter(|proxy| !proxy.is_empty()) {
            let selected = SelectedProxy::parse(proxy).with_context(|| {
                format!(
                    "network address policy requires a supported remote-DNS WebSocket proxy, got `{proxy}`"
                )
            })?;
            request.proxy = Some(selected.curl_url().to_owned());
            return Ok(());
        }
        let host = url
            .host()
            .ok_or_else(|| anyhow::anyhow!("WebSocket URL `{url}` is missing a host"))?;

        match host {
            Host::Domain(host) => {
                let port = url
                    .port_or_known_default()
                    .ok_or_else(|| anyhow::anyhow!("WebSocket URL `{url}` has no port"))?;
                let host_resolve = HostResolveOverrides::parse(&request.resolve_entries)?;
                if let Some(addresses) = host_resolve.addresses_for(host, port) {
                    return self
                        .network_address_policy
                        .check_addresses(addresses, url.as_str());
                }
                let target = request.dns_resolution.target().ok_or_else(|| {
                    anyhow::anyhow!(
                        "network address policy requires shared DNS resolution for WebSocket hostname `{host}` in `{url}`"
                    )
                })?;
                if !target.host().eq_ignore_ascii_case(host) || target.port() != port {
                    bail!(
                        "WebSocket DNS target `{}:{}` does not match policy target `{host}:{port}`",
                        target.host(),
                        target.port()
                    );
                }
                request.dns_resolution.set_network_address_policy(
                    self.network_address_policy.clone(),
                    url.to_string(),
                );
            }
            Host::Ipv4(address) => self
                .network_address_policy
                .check_address(address.into(), url.as_str())?,
            Host::Ipv6(address) => self
                .network_address_policy
                .check_address(address.into(), url.as_str())?,
        }
        Ok(())
    }
}
