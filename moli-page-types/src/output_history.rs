use std::collections::VecDeque;

/// A bounded replay window. Positions keep increasing when old output is
/// evicted, so existing consumers cannot mistake new output for an old entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputHistory<T> {
    entries: Vec<T>,
    charges: VecDeque<(usize, usize)>,
    end: usize,
    bytes: usize,
}

impl<T> Default for OutputHistory<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            charges: VecDeque::new(),
            end: 0,
            bytes: 0,
        }
    }
}

impl<T> OutputHistory<T> {
    /// Returns evicted entries so reports can retire their derived views too.
    /// Keep one oversized newest entry: retention must not suppress live output.
    /// Transport admission separately bounds each incoming record.
    pub fn push(&mut self, value: T, payload_bytes: usize) -> Vec<T> {
        const MAX_ENTRIES: usize = 1000;
        const MAX_BYTES: usize = 10 * 1024 * 1024;
        let charge = payload_bytes.saturating_add(std::mem::size_of::<(T, usize, usize)>());
        let mut removed = 0;
        while self.charges.len() >= MAX_ENTRIES || self.bytes.saturating_add(charge) > MAX_BYTES {
            let Some((_, bytes)) = self.charges.pop_front() else {
                break;
            };
            self.bytes -= bytes;
            removed += 1;
        }
        let evicted = self.entries.drain(..removed).collect();
        self.charges.push_back((self.end, charge));
        self.end = self.end.checked_add(1).expect("output position exhausted");
        self.bytes = self.bytes.saturating_add(charge);
        self.entries.push(value);
        evicted
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }

    pub fn as_slice(&self) -> &[T] {
        &self.entries
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.entries.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut()
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.entries.retain(|entry| {
            let charge = self.charges.pop_front().expect("one charge per entry");
            if keep(entry) {
                self.charges.push_back(charge);
                true
            } else {
                self.bytes -= charge.1;
                false
            }
        });
    }

    pub fn into_items(self) -> impl Iterator<Item = T> {
        self.entries.into_iter()
    }

    pub fn iter_since(&self, position: usize) -> impl Iterator<Item = &T> {
        self.entries
            .iter()
            .zip(&self.charges)
            .filter(move |(_, (index, _))| *index >= position)
            .map(|(entry, _)| entry)
    }

    pub fn since(&self, position: usize) -> Vec<T>
    where
        T: Clone,
    {
        self.iter_since(position).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::OutputHistory;

    #[test]
    fn eviction_and_retirement_preserve_consumer_positions() {
        let mut history = OutputHistory::default();
        for index in 0..1010 {
            history.push(index, 0);
        }
        assert_eq!(history.len(), 1000);
        assert_eq!(history.since(0), (10..1010).collect::<Vec<_>>());
        history.retain(|index| index % 2 == 0);
        let cursor = history.end();
        assert_eq!(history.since(1005), vec![1006, 1008]);
        history.push(1010, 0);
        assert_eq!(history.since(cursor), vec![1010]);
        assert!(history.since(history.end()).is_empty());
        history.retain(|_| false);
        assert_eq!(history.retained_bytes(), 0);
        assert_eq!(history.end(), 1011);
    }

    #[test]
    fn byte_limit_keeps_a_tail_and_releases_an_oversized_entry() {
        let mut history = OutputHistory::default();
        for index in 0..10 {
            history.push(index, 1024 * 1024);
        }
        assert_eq!(history.since(0), (1..10).collect::<Vec<_>>());
        assert!(history.retained_bytes() <= 10 * 1024 * 1024);
        history.push(10, 11 * 1024 * 1024);
        assert_eq!(history.as_slice(), &[10]);
        history.push(11, 0);
        assert_eq!(history.as_slice(), &[11]);
        assert!(history.retained_bytes() < 1024);
    }
}
