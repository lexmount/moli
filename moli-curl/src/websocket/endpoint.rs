//! RFC 6455 endpoint admission belongs to the native owner, before TCP connect.
//! Curl still chooses and races addresses; a failed candidate releases its own
//! lease. Successful candidates remain held until the opening handshake ends.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, anyhow};
use curl::easy::Easy2;
use parking_lot::Mutex;

use super::{CurlWebSocketRequest, request::Handshake};
use crate::CurlTransferId;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Endpoint {
    Address(SocketAddr),
    // With remote proxy DNS the origin address is unknown. Serializing by the
    // proxy address would unnecessarily block unrelated remote hosts.
    Host(String, u16),
}

impl Endpoint {
    fn address(address: IpAddr, port: u16, scope: u32) -> Self {
        let socket = match address {
            IpAddr::V4(address) => SocketAddr::from((address, port)),
            IpAddr::V6(address) => match address.to_ipv4_mapped() {
                Some(address) => SocketAddr::from((address, port)),
                None => SocketAddrV6::new(address, port, 0, scope).into(),
            },
        };
        Self::Address(socket)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Attempt {
    transfer: CurlTransferId,
    token: usize,
}

struct Entry {
    owner: CurlTransferId,
    held: HashSet<Attempt>,
    waiting: VecDeque<Attempt>,
}

#[derive(Default)]
pub(super) struct EndpointLocks {
    entries: HashMap<Endpoint, Entry>,
    attempts: HashMap<Attempt, Endpoint>,
    ready: HashSet<CurlTransferId>,
}

impl EndpointLocks {
    fn acquire(&mut self, endpoint: Endpoint, attempt: Attempt) -> bool {
        let entry = self
            .entries
            .entry(endpoint.clone())
            .or_insert_with(|| Entry {
                owner: attempt.transfer,
                held: HashSet::new(),
                waiting: VecDeque::new(),
            });
        let existing = self.attempts.insert(attempt, endpoint).is_some();
        if entry.owner == attempt.transfer {
            entry.held.insert(attempt);
            true
        } else {
            if !existing {
                entry.waiting.push_back(attempt);
            }
            false
        }
    }

    fn release(&mut self, attempt: Attempt) {
        let Some(endpoint) = self.attempts.remove(&attempt) else {
            return;
        };
        let Some(entry) = self.entries.get_mut(&endpoint) else {
            return;
        };
        entry.held.remove(&attempt);
        entry.waiting.retain(|waiting| *waiting != attempt);
        if !entry.held.is_empty() {
            return;
        }
        if let Some(next) = entry.waiting.pop_front() {
            entry.owner = next.transfer;
            entry.held.insert(next);
            // Concurrent TCP candidates of one logical WebSocket share its
            // ownership, including different proxy addresses for the same host.
            entry.waiting.retain(|waiting| {
                if waiting.transfer == next.transfer {
                    entry.held.insert(*waiting);
                    false
                } else {
                    true
                }
            });
            self.ready.insert(next.transfer);
        } else {
            self.entries.remove(&endpoint);
        }
    }

    fn complete(&mut self, transfer: CurlTransferId) {
        let attempts: Vec<_> = self
            .attempts
            .keys()
            .filter(|attempt| attempt.transfer == transfer)
            .copied()
            .collect();
        for attempt in attempts {
            self.release(attempt);
        }
        self.ready.remove(&transfer);
    }

    pub(super) fn take_ready(&mut self) -> HashSet<CurlTransferId> {
        std::mem::take(&mut self.ready)
    }
}

/// The Easy borrows native callbacks from this stable allocation. It owns the
/// Arc through its Handler, which outlives curl_easy_cleanup. Curl connection
/// filters keep only their own attempt token, never a pointer to this context.
pub(super) struct EndpointGate {
    native: curl_sys::curl_moli_websocket_gate,
    locks: Arc<Mutex<EndpointLocks>>,
    transfer: CurlTransferId,
    proxy_target: Option<Endpoint>,
    completed: AtomicBool,
    #[cfg(test)]
    waiting: Arc<tokio::sync::Notify>,
}

impl EndpointGate {
    pub(super) fn install(
        easy: &mut Easy2<Handshake>,
        request: &CurlWebSocketRequest,
        transfer: CurlTransferId,
        locks: Arc<Mutex<EndpointLocks>>,
        #[cfg(test)] waiting: Arc<tokio::sync::Notify>,
    ) -> Result<()> {
        let proxy_target = if request
            .proxy
            .as_deref()
            .is_some_and(|proxy| !proxy.is_empty())
        {
            let url = url::Url::parse(&request.url)?;
            let port = url
                .port_or_known_default()
                .ok_or_else(|| anyhow!("missing WebSocket port"))?;
            Some(
                match url
                    .host()
                    .ok_or_else(|| anyhow!("missing WebSocket host"))?
                {
                    url::Host::Domain(host) => Endpoint::Host(host.to_owned(), port),
                    url::Host::Ipv4(address) => Endpoint::address(address.into(), port, 0),
                    url::Host::Ipv6(address) => Endpoint::address(address.into(), port, 0),
                },
            )
        } else {
            None
        };
        let gate = Arc::new_cyclic(|weak| Self {
            native: curl_sys::curl_moli_websocket_gate {
                userdata: weak.as_ptr() as usize,
                acquire: Some(acquire),
                release: Some(release),
            },
            locks,
            transfer,
            proxy_target,
            completed: AtomicBool::new(false),
            #[cfg(test)]
            waiting,
        });
        let native = &gate.native as *const _;
        easy.get_mut().endpoint_gate = Some(gate);
        // SAFETY: The owning Arc is stable and lives through native cleanup.
        // Only the runtime owner configures or calls this Easy's callbacks.
        let result = unsafe { curl_sys::curl_easy_moli_websocket_set_gate(easy.raw(), native) };
        if result != curl_sys::CURLE_OK {
            return Err(curl::Error::new(result).into());
        }
        Ok(())
    }

    pub(super) fn complete(&self) {
        if !self.completed.swap(true, Ordering::Relaxed) {
            self.locks.lock().complete(self.transfer);
        }
    }
}

impl Drop for EndpointGate {
    fn drop(&mut self) {
        self.complete();
    }
}

extern "C" fn acquire(
    userdata: usize,
    token: usize,
    address: *const u8,
    length: u32,
    port: u16,
    scope: u32,
) -> i32 {
    // SAFETY: The native gate is installed with a live Arc pointer, invoked
    // exclusively on its owner, outside any Rust borrow of the Handler.
    let gate = unsafe { &*(userdata as *const EndpointGate) };
    if gate.completed.load(Ordering::Relaxed) {
        return 1;
    }
    let endpoint = match gate.proxy_target.clone() {
        Some(endpoint) => endpoint,
        None => {
            let address = match length {
                // SAFETY: cf-socket passes exactly four or sixteen initialized
                // IP address octets, valid for this synchronous callback.
                4 => IpAddr::V4(Ipv4Addr::from(unsafe { *address.cast::<[u8; 4]>() })),
                16 => IpAddr::V6(Ipv6Addr::from(unsafe { *address.cast::<[u8; 16]>() })),
                _ => return -1,
            };
            Endpoint::address(address, port, scope)
        }
    };
    let admitted = gate.locks.lock().acquire(
        endpoint,
        Attempt {
            transfer: gate.transfer,
            token,
        },
    );
    #[cfg(test)]
    if !admitted {
        gate.waiting.notify_one();
    }
    i32::from(admitted)
}

extern "C" fn release(userdata: usize, token: usize) {
    // SAFETY: Same owner and lifetime contract as acquire.
    let gate = unsafe { &*(userdata as *const EndpointGate) };
    if !gate.completed.load(Ordering::Relaxed) {
        gate.locks.lock().release(Attempt {
            transfer: gate.transfer,
            token,
        });
    }
}
