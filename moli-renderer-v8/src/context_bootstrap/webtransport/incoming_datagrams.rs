//! Native ingress is bounded even while no JavaScript reader has demand.
//! Only a registered pull publishes bytes through the networking task source.

use bytes::Bytes;
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub(super) struct Incoming {
    state: Mutex<State>,
    changed: Notify,
}

struct State {
    datagrams: VecDeque<(Instant, Bytes)>,
    maximum: usize,
    age: Option<Duration>,
    discarded: bool,
    closed: Option<String>,
}

impl State {
    fn expire(&mut self, now: Instant) {
        if let Some(age) = self.age {
            while self
                .datagrams
                .front()
                .is_some_and(|(received, _)| now.saturating_duration_since(*received) > age)
            {
                self.datagrams.pop_front();
            }
        }
    }

    fn trim(&mut self) {
        while self.datagrams.len() > self.maximum {
            self.datagrams.pop_front();
        }
    }
}

impl Incoming {
    pub(super) fn new(maximum: usize, age: Option<Duration>) -> Self {
        Self {
            state: Mutex::new(State {
                datagrams: VecDeque::new(),
                maximum: maximum.max(1),
                age,
                discarded: false,
                closed: None,
            }),
            changed: Notify::new(),
        }
    }

    pub(super) fn push(&self, datagram: Bytes) {
        self.push_at(datagram, Instant::now());
    }

    fn push_at(&self, datagram: Bytes, now: Instant) {
        let mut state = self.state.lock();
        if state.discarded || state.closed.is_some() {
            return;
        }
        state.expire(now);
        state.datagrams.push_back((now, datagram));
        state.trim();
        drop(state);
        self.changed.notify_one();
    }

    pub(super) fn configure(&self, maximum: usize, age: Option<Duration>) {
        let mut state = self.state.lock();
        state.maximum = maximum.max(1);
        state.age = age;
        state.expire(Instant::now());
        state.trim();
    }

    pub(super) fn discard(&self) {
        let mut state = self.state.lock();
        state.discarded = true;
        state.datagrams.clear();
    }

    pub(super) fn finish(&self, error: String) {
        let mut state = self.state.lock();
        if state.closed.is_some() {
            return;
        }
        state.closed = Some(error);
        state.datagrams.clear();
        drop(state);
        self.changed.notify_one();
    }

    pub(super) async fn read(&self) -> Result<Bytes, String> {
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock();
                state.expire(Instant::now());
                if let Some((_, datagram)) = state.datagrams.pop_front() {
                    return Ok(datagram);
                }
                if let Some(error) = &state.closed {
                    return Err(error.clone());
                }
            }
            changed.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unread_ingress_discards_oldest_payloads_at_the_configured_count() {
        let incoming = Incoming::new(5, None);
        for value in 0..50 {
            incoming.push(Bytes::from(vec![value]));
        }
        for value in 45..50 {
            assert_eq!(incoming.read().await.unwrap(), Bytes::from(vec![value]));
        }
        incoming.finish("closed".into());
        assert_eq!(incoming.read().await.unwrap_err(), "closed");
    }

    #[tokio::test]
    async fn incoming_age_and_later_limit_changes_apply_to_retained_payloads() {
        let incoming = Incoming::new(5, Some(Duration::from_secs(60)));
        incoming.push_at(
            Bytes::from_static(b"expired"),
            Instant::now() - Duration::from_secs(120),
        );
        incoming.push(Bytes::from_static(b"current"));
        assert_eq!(
            incoming.read().await.unwrap(),
            Bytes::from_static(b"current")
        );
        incoming.configure(5, None);
        for value in 0..5 {
            incoming.push(Bytes::from(vec![value]));
        }
        incoming.configure(2, None);
        assert_eq!(incoming.read().await.unwrap(), Bytes::from(vec![3]));
        assert_eq!(incoming.read().await.unwrap(), Bytes::from(vec![4]));
    }

    #[tokio::test]
    async fn finishing_ingress_wakes_a_pending_read_and_rejects_late_bytes() {
        let incoming = std::sync::Arc::new(Incoming::new(1, None));
        let reader = incoming.clone();
        let waiting = tokio::spawn(async move { reader.read().await });
        tokio::task::yield_now().await;
        incoming.finish("owner retired".into());
        incoming.push(Bytes::from_static(b"late"));
        assert_eq!(waiting.await.unwrap().unwrap_err(), "owner retired");
        assert_eq!(incoming.read().await.unwrap_err(), "owner retired");
    }
}
