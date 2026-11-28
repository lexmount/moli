use std::ops::{Index, IndexMut};

use slotmap::DenseSlotMap;

use super::{LayoutBox, LayoutBoxId};

/// Stable handles identify boxes across passes. Dense indices belong to the
/// current storage arrangement and index both working and published geometry.
#[derive(Debug)]
pub(crate) struct LayoutBoxes<N> {
    entries: DenseSlotMap<LayoutBoxId, LayoutBox<N>>,
    storage_revision: u64,
}

impl<N> Default for LayoutBoxes<N> {
    fn default() -> Self {
        Self {
            entries: DenseSlotMap::with_key(),
            storage_revision: 0,
        }
    }
}

impl<N> LayoutBoxes<N> {
    pub(crate) fn insert(&mut self, value: LayoutBox<N>) -> LayoutBoxId {
        self.storage_revision += 1;
        self.entries.insert(value)
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, value: LayoutBox<N>) {
        self.insert(value);
    }

    pub(crate) fn pop(&mut self) -> Option<LayoutBox<N>> {
        let id = *self.entries.keys_as_slice().last()?;
        self.storage_revision += 1;
        self.entries.remove(id)
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn capacity(&self) -> usize {
        self.entries.capacity()
    }

    #[inline]
    pub(crate) fn index_of(&self, id: impl std::borrow::Borrow<LayoutBoxId>) -> usize {
        let value = &self.entries[*id.borrow()];
        self.entries
            .values_as_slice()
            .element_offset(value)
            .expect("a live layout box belongs to the dense storage")
    }

    #[inline]
    pub(crate) fn id_at(&self, index: usize) -> LayoutBoxId {
        self.entries.keys_as_slice()[index]
    }

    #[inline]
    pub(crate) fn keys(
        &self,
    ) -> impl DoubleEndedIterator<Item = LayoutBoxId> + ExactSizeIterator + '_ {
        self.entries.keys_as_slice().iter().copied()
    }

    #[inline]
    pub(crate) fn get(&self, id: LayoutBoxId) -> Option<&LayoutBox<N>> {
        self.entries.get(id)
    }

    #[inline]
    pub(crate) fn get_mut(&mut self, id: LayoutBoxId) -> Option<&mut LayoutBox<N>> {
        self.entries.get_mut(id)
    }

    #[inline]
    pub(crate) fn values(&self) -> std::slice::Iter<'_, LayoutBox<N>> {
        self.entries.values_as_slice().iter()
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut LayoutBox<N>> {
        self.entries.values_mut()
    }

    pub(crate) fn keyed(&self) -> impl Iterator<Item = (LayoutBoxId, &LayoutBox<N>)> {
        self.entries.iter()
    }

    pub(crate) fn keyed_mut(&mut self) -> impl Iterator<Item = (LayoutBoxId, &mut LayoutBox<N>)> {
        self.entries.iter_mut()
    }

    pub(crate) fn storage_revision(&self) -> u64 {
        self.storage_revision
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(LayoutBoxId) -> bool) {
        let previous_len = self.entries.len();
        self.entries.retain(|id, _| keep(id));
        if self.entries.len() != previous_len {
            self.storage_revision += 1;
        }
    }
}

impl<N> Index<LayoutBoxId> for LayoutBoxes<N> {
    type Output = LayoutBox<N>;

    #[inline]
    fn index(&self, id: LayoutBoxId) -> &Self::Output {
        &self.entries[id]
    }
}

impl<N> IndexMut<LayoutBoxId> for LayoutBoxes<N> {
    #[inline]
    fn index_mut(&mut self, id: LayoutBoxId) -> &mut Self::Output {
        &mut self.entries[id]
    }
}

impl<N> Index<usize> for LayoutBoxes<N> {
    type Output = LayoutBox<N>;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.entries.values_as_slice()[index]
    }
}

impl<N> IndexMut<usize> for LayoutBoxes<N> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.entries.values_as_mut_slice()[index]
    }
}

#[cfg(test)]
impl<N> Extend<LayoutBox<N>> for LayoutBoxes<N> {
    fn extend<T: IntoIterator<Item = LayoutBox<N>>>(&mut self, values: T) {
        for value in values {
            self.insert(value);
        }
    }
}

#[cfg(test)]
mod tests;
