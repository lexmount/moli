//! Native connection admission order, independent of V8 and task dispatch.

use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ConnectionRequestId(u64);

pub(super) struct ConnectionQueue<T> {
    next_id: u64,
    queues: BTreeMap<String, VecDeque<(ConnectionRequestId, T)>>,
    keys: BTreeMap<ConnectionRequestId, String>,
}

impl<T> Default for ConnectionQueue<T> {
    fn default() -> Self {
        Self {
            next_id: 0,
            queues: BTreeMap::new(),
            keys: BTreeMap::new(),
        }
    }
}

impl<T> ConnectionQueue<T> {
    pub(super) fn push(&mut self, key: String, value: T) -> (ConnectionRequestId, bool) {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("IDB connection request id overflow");
        let id = ConnectionRequestId(self.next_id);
        self.keys.insert(id, key.clone());
        let queue = self.queues.entry(key).or_default();
        let is_head = queue.is_empty();
        queue.push_back((id, value));
        (id, is_head)
    }

    pub(super) fn head(&self, id: ConnectionRequestId) -> Option<&T> {
        let key = self.keys.get(&id)?;
        let (head, value) = self.queues.get(key)?.front()?;
        (*head == id).then_some(value)
    }

    pub(super) fn head_mut(&mut self, id: ConnectionRequestId) -> Option<&mut T> {
        let key = self.keys.get(&id)?;
        let (head, value) = self.queues.get_mut(key)?.front_mut()?;
        (*head == id).then_some(value)
    }

    pub(super) fn head_for_key(&self, key: &str) -> Option<(ConnectionRequestId, &T)> {
        self.queues
            .get(key)?
            .front()
            .map(|(id, value)| (*id, value))
    }

    pub(super) fn heads(&self) -> impl Iterator<Item = (ConnectionRequestId, &T)> {
        self.queues
            .values()
            .filter_map(|queue| queue.front())
            .map(|(id, value)| (*id, value))
    }

    /// A callback may finish only its exact head, never a newer request whose
    /// database name happens to be the same. Starting an upgrade does not pop
    /// this entry: the open result still belongs to it.
    pub(super) fn finish(
        &mut self,
        id: ConnectionRequestId,
    ) -> Option<(T, Option<ConnectionRequestId>)> {
        let key = self.keys.get(&id)?.clone();
        let queue = self.queues.get_mut(&key)?;
        if queue.front()?.0 != id {
            return None;
        }
        let (_, value) = queue
            .pop_front()
            .expect("checked nonempty connection queue");
        self.keys.remove(&id);
        let next = queue.front().map(|(id, _)| *id);
        if queue.is_empty() {
            self.queues.remove(&key);
        }
        Some((value, next))
    }

    /// Retiring a realm may remove both a running head and queued followers.
    /// Return only newly exposed heads so unrelated databases are not woken.
    pub(super) fn retire(
        &mut self,
        mut matches: impl FnMut(&T) -> bool,
    ) -> Vec<ConnectionRequestId> {
        let mut ready = Vec::new();
        self.queues.retain(|_, queue| {
            let previous = queue.front().map(|(id, _)| *id);
            queue.retain(|(id, value)| {
                if matches(value) {
                    self.keys.remove(id);
                    false
                } else {
                    true
                }
            });
            if let Some((id, _)) = queue.front()
                && Some(*id) != previous
            {
                ready.push(*id);
            }
            !queue.is_empty()
        });
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_head_can_finish_and_empty_queues_are_removed() {
        let mut queue = ConnectionQueue::default();
        let (first, ready) = queue.push("db".into(), 'a');
        assert!(ready);
        let (second, ready) = queue.push("db".into(), 'b');
        assert!(!ready);
        assert!(queue.head(second).is_none());
        assert!(queue.finish(second).is_none());
        assert_eq!(queue.finish(first), Some(('a', Some(second))));
        assert!(queue.finish(first).is_none());
        assert_eq!(queue.head(second), Some(&'b'));
        assert_eq!(queue.finish(second), Some(('b', None)));
        assert!(queue.queues.is_empty());
        assert!(queue.keys.is_empty());
    }

    #[test]
    fn databases_have_independent_heads_and_retirement_preserves_survivor_order() {
        let mut queue = ConnectionQueue::default();
        let (first, _) = queue.push("db".into(), 1);
        let (_, _) = queue.push("db".into(), 1);
        let (survivor, _) = queue.push("db".into(), 2);
        let (last, _) = queue.push("db".into(), 3);
        let (independent, ready) = queue.push("other-origin-or-bucket".into(), 2);
        assert!(ready);
        assert_eq!(queue.retire(|owner| *owner == 1), [survivor]);
        assert!(queue.finish(first).is_none());
        assert_eq!(queue.head(independent), Some(&2));
        assert_eq!(queue.finish(survivor), Some((2, Some(last))));
        assert_eq!(queue.finish(last), Some((3, None)));
        assert_eq!(queue.finish(independent), Some((2, None)));
        assert!(queue.keys.is_empty());
    }
}
