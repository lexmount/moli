use std::sync::Arc;

use super::{NATIVE_DOM_NODE_CHUNK_CAPACITY, Node};

#[derive(Debug, Clone)]
pub(super) struct NativeNodeStorage {
    pub(super) chunks: Arc<Vec<Arc<Vec<Option<Node>>>>>,
    allocated: usize,
    owned: usize,
}

impl NativeNodeStorage {
    pub(super) fn from_node(node: Node) -> Self {
        Self {
            chunks: Arc::new(vec![Arc::new(vec![Some(node)])]),
            allocated: 1,
            owned: 1,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.allocated
    }

    pub(super) fn is_empty(&self) -> bool {
        self.owned == 0
    }

    pub(super) fn get(&self, index: usize) -> Option<&Node> {
        let chunk = self.chunks.get(index / NATIVE_DOM_NODE_CHUNK_CAPACITY)?;
        chunk.get(index % NATIVE_DOM_NODE_CHUNK_CAPACITY)?.as_ref()
    }

    pub(super) fn get_mut(&mut self, index: usize) -> Option<&mut Node> {
        let chunks = Arc::make_mut(&mut self.chunks);
        let chunk = chunks.get_mut(index / NATIVE_DOM_NODE_CHUNK_CAPACITY)?;
        Arc::make_mut(chunk)
            .get_mut(index % NATIVE_DOM_NODE_CHUNK_CAPACITY)?
            .as_mut()
    }

    /// Move the complete native payload out without reusing its old handle.
    /// Snapshots keep their own payload through the existing chunk COW boundary.
    pub(super) fn take(&mut self, index: usize) -> Option<Node> {
        self.get(index)?;
        let chunks = Arc::make_mut(&mut self.chunks);
        let chunk = Arc::make_mut(&mut chunks[index / NATIVE_DOM_NODE_CHUNK_CAPACITY]);
        let node = chunk[index % NATIVE_DOM_NODE_CHUNK_CAPACITY].take()?;
        self.owned -= 1;
        Some(node)
    }

    pub(super) fn push(&mut self, node: Node) {
        let chunks = Arc::make_mut(&mut self.chunks);
        match chunks.last_mut() {
            Some(chunk) if chunk.len() < NATIVE_DOM_NODE_CHUNK_CAPACITY => {
                Arc::make_mut(chunk).push(Some(node));
            }
            _ => chunks.push(Arc::new(vec![Some(node)])),
        }
        self.allocated += 1;
        self.owned += 1;
    }

    pub(super) fn iter(&self) -> NativeDomNodes<'_> {
        NativeDomNodes {
            storage: self,
            front: 0,
            back: self.allocated,
            remaining: self.owned,
        }
    }
}

#[derive(Clone)]
pub struct NativeDomNodes<'a> {
    storage: &'a NativeNodeStorage,
    front: usize,
    back: usize,
    remaining: usize,
}

impl NativeDomNodes<'_> {
    pub fn iter(&self) -> Self {
        self.clone()
    }
}

impl<'a> Iterator for NativeDomNodes<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        while self.front < self.back {
            let index = self.front;
            self.front += 1;
            if let Some(node) = self.storage.get(index) {
                self.remaining -= 1;
                return Some(node);
            }
        }
        debug_assert_eq!(self.remaining, 0);
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for NativeDomNodes<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        while self.front < self.back {
            self.back -= 1;
            if let Some(node) = self.storage.get(self.back) {
                self.remaining -= 1;
                return Some(node);
            }
        }
        debug_assert_eq!(self.remaining, 0);
        None
    }
}

impl ExactSizeIterator for NativeDomNodes<'_> {}
impl std::iter::FusedIterator for NativeDomNodes<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{NativeNodeId, NodeData, NodeFlags, Text};

    fn node(index: usize) -> Node {
        Node::new(
            NativeNodeId::new(index),
            None,
            None,
            NodeFlags::new(false),
            NodeData::Text(Text::new("native")),
        )
    }

    #[test]
    fn vacancies_preserve_exact_size_double_ended_iteration_and_handle_allocation() {
        assert_eq!(
            std::mem::size_of::<Option<Node>>(),
            std::mem::size_of::<Node>()
        );
        let mut storage = NativeNodeStorage::from_node(node(0));
        for index in 1..520 {
            storage.push(node(index));
        }
        for index in [0, 1, 3, 256, 257, 518, 519] {
            assert!(storage.take(index).is_some());
        }
        assert!(storage.take(256).is_none());
        assert!(storage.get_mut(256).is_none());
        assert_eq!(storage.len(), 520);
        let mut iter = storage.iter();
        assert_eq!(iter.len(), 513);
        assert_eq!(iter.next().unwrap().id().index(), 2);
        assert_eq!(iter.next_back().unwrap().id().index(), 517);
        assert_eq!(iter.size_hint(), (511, Some(511)));
        let copied = iter.iter();
        assert_eq!(copied.len(), 511);
        let rest = iter
            .by_ref()
            .map(|node| node.id().index())
            .collect::<Vec<_>>();
        assert_eq!(rest.len(), 511);
        assert!(!rest.contains(&256));
        assert!(iter.next().is_none());
        assert!(iter.next_back().is_none());
        storage.push(node(520));
        assert_eq!(storage.get(520).unwrap().id().index(), 520);
        assert_eq!(storage.iter().len(), 514);
    }

    #[test]
    fn transfer_retires_only_the_source_chunk_and_preserves_immutable_snapshots() {
        let mut storage = NativeNodeStorage::from_node(node(0));
        for index in 1..600 {
            storage.push(node(index));
        }
        let snapshot = storage.clone();
        let payload = storage.take(300).unwrap();
        assert_eq!(payload.id().index(), 300);
        assert_eq!(snapshot.get(300).unwrap().id().index(), 300);
        assert!(storage.get(300).is_none());
        assert_eq!(snapshot.iter().len(), 600);
        assert_eq!(storage.iter().len(), 599);
        for (index, (live, frozen)) in storage
            .chunks
            .iter()
            .zip(snapshot.chunks.iter())
            .enumerate()
        {
            assert_eq!(Arc::ptr_eq(live, frozen), index != 1);
        }
    }
}
