use crate::context::{AudioNodeId, DESTINATION_NODE_ID};
use crate::render::graph::Node;

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default)]
pub(super) struct AudioNodeIdHasher(u64);

impl Hasher for AudioNodeIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _bytes: &[u8]) {
        panic!("AudioNodeIdHasher only supports u64 keys");
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

pub(super) type AudioNodeIdSet = HashSet<AudioNodeId, BuildHasherDefault<AudioNodeIdHasher>>;

#[derive(Debug)]
pub(crate) struct NodeCollection {
    nodes: Vec<Option<RefCell<Node>>>,
}

impl NodeCollection {
    pub fn new() -> Self {
        let mut instance = Self {
            nodes: Vec::with_capacity(64),
        };
        instance.ensure_capacity(64);
        instance
    }

    // NodeCollection is considered empty until the destination is set up
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        let destination_id = DESTINATION_NODE_ID.0 as usize;
        self.nodes[destination_id].is_none()
    }

    #[inline(always)]
    fn ensure_capacity(&mut self, new_len: usize) {
        self.nodes
            .resize_with(new_len.max(self.nodes.len()), || None);
    }

    #[inline(always)]
    pub fn insert(&mut self, index: AudioNodeId, value: RefCell<Node>) {
        let index = index.0 as usize;
        self.ensure_capacity(index + 1);
        self.nodes[index] = Some(value);
    }

    #[inline(always)]
    pub fn remove(&mut self, index: AudioNodeId) -> RefCell<Node> {
        self.nodes[index.0 as usize]
            .take()
            .expect("Unable to remove non-existing Node in NodeCollection")
    }

    #[inline(always)]
    pub fn keys(&self) -> impl Iterator<Item = AudioNodeId> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_ref().and(Some(AudioNodeId(i as u64))))
    }

    #[inline(always)]
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut RefCell<Node>> {
        self.nodes.iter_mut().filter_map(Option::as_mut)
    }

    #[inline(always)]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (AudioNodeId, &mut RefCell<Node>)> {
        self.nodes
            .iter_mut()
            .enumerate()
            .filter_map(|(i, v)| v.as_mut().map(|m| (AudioNodeId(i as u64), m)))
    }

    #[inline(always)]
    pub fn contains(&self, index: AudioNodeId) -> bool {
        self.nodes[index.0 as usize].is_some()
    }

    #[inline(always)]
    pub fn get_mut(&mut self, index: AudioNodeId) -> Option<&mut RefCell<Node>> {
        self.nodes[index.0 as usize].as_mut()
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_unchecked(&self, index: AudioNodeId) -> &RefCell<Node> {
        self.nodes[index.0 as usize].as_ref().unwrap()
    }

    #[track_caller]
    #[inline(always)]
    pub fn get_unchecked_mut(&mut self, index: AudioNodeId) -> &mut Node {
        self.nodes[index.0 as usize].as_mut().unwrap().get_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // regression test for:
    // https://github.com/orottier/web-audio-api-rs/issues/389
    #[test]
    fn test_empty() {
        let nodes = NodeCollection::new();
        assert!(nodes.is_empty());
    }
}
