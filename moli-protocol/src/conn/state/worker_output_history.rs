use std::collections::VecDeque;

/// A bounded replay window with cumulative emission positions. Eviction must
/// not change positions already captured by a session or a prepared output.
#[derive(Debug)]
pub(super) struct WorkerOutputHistory<T> {
    entries: VecDeque<(T, usize)>,
    end: usize,
    bytes: usize,
}

impl<T> Default for WorkerOutputHistory<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            end: 0,
            bytes: 0,
        }
    }
}

impl<T> WorkerOutputHistory<T> {
    pub(super) fn push(&mut self, value: T, payload_bytes: usize) {
        // Match V8's console history window. Keep a single oversized newest
        // message so retention cannot swallow live output. Renderer transport
        // admission separately bounds the size of every incoming record.
        const MAX_ENTRIES: usize = 1000;
        const MAX_BYTES: usize = 10 * 1024 * 1024;
        let charge = payload_bytes.saturating_add(std::mem::size_of::<(T, usize)>());
        while self.entries.len() >= MAX_ENTRIES || self.bytes.saturating_add(charge) > MAX_BYTES {
            let Some((_, bytes)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= bytes;
        }
        self.end = self
            .end
            .checked_add(1)
            .expect("Worker output position exhausted");
        self.bytes = self.bytes.saturating_add(charge);
        self.entries.push_back((value, charge));
    }

    pub(super) fn end(&self) -> usize {
        self.end
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn since(&self, position: usize) -> Vec<T>
    where
        T: Clone,
    {
        let start = position
            .saturating_sub(self.end - self.entries.len())
            .min(self.entries.len());
        self.entries
            .range(start..)
            .map(|(value, _)| value.clone())
            .collect()
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|(value, _)| value)
    }

    #[cfg(test)]
    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.entries.iter().map(|(value, _)| value)
    }
}
