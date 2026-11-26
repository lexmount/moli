use std::{
    ops::{Index, IndexMut},
    sync::Arc,
};

use slotmap::{SecondaryMap, SlotMap};

use super::{LayoutBox, LayoutBoxId};

type BoxMap<N> = SlotMap<LayoutBoxId, LayoutBox<N>>;

/// Pass-local order. Handles never encode these positions.
#[derive(Clone, Debug, Default)]
pub(crate) struct BoxOrder {
    ids: Vec<LayoutBoxId>,
    positions: SecondaryMap<LayoutBoxId, u32>,
}

impl BoxOrder {
    pub(crate) fn new(ids: Vec<LayoutBoxId>) -> Self {
        let mut positions = SecondaryMap::new();
        for (index, id) in ids.iter().copied().enumerate() {
            assert!(
                positions
                    .insert(id, u32::try_from(index).expect("layout order exceeds u32"))
                    .is_none(),
                "a layout box may appear only once in an order"
            );
        }
        Self { ids, positions }
    }

    #[inline]
    pub(crate) fn index_of(&self, id: impl std::borrow::Borrow<LayoutBoxId>) -> usize {
        self.position(*id.borrow())
            .expect("layout handle does not belong to this order")
    }

    #[inline]
    pub(crate) fn position(&self, id: LayoutBoxId) -> Option<usize> {
        self.positions.get(id).map(|index| *index as usize)
    }

    #[inline]
    pub(crate) fn contains(&self, id: LayoutBoxId) -> bool {
        self.positions.contains_key(id)
    }

    #[inline]
    pub(crate) fn id_at(&self, index: usize) -> LayoutBoxId {
        self.ids[index]
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }
}

/// Stable box identities with an explicit logical iteration order.
/// The backend's slot/dense storage order is never a tree or publication order.
#[derive(Debug)]
pub(crate) struct LayoutBoxes<N> {
    entries: BoxMap<N>,
    order: Arc<BoxOrder>,
}

impl<N> Default for LayoutBoxes<N> {
    fn default() -> Self {
        Self {
            entries: BoxMap::with_key(),
            order: Arc::default(),
        }
    }
}

impl<N> LayoutBoxes<N> {
    pub(crate) fn insert(&mut self, value: LayoutBox<N>) -> LayoutBoxId {
        let id = self.entries.insert(value);
        let order = Arc::make_mut(&mut self.order);
        let index = u32::try_from(order.ids.len()).expect("layout order exceeds u32");
        order.ids.push(id);
        assert!(order.positions.insert(id, index).is_none());
        id
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, value: LayoutBox<N>) {
        self.insert(value);
    }

    pub(crate) fn pop(&mut self) -> Option<LayoutBox<N>> {
        let order = Arc::make_mut(&mut self.order);
        let id = order.ids.pop()?;
        order.positions.remove(id);
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
        self.order.index_of(id)
    }
    #[inline]
    pub(crate) fn id_at(&self, index: usize) -> LayoutBoxId {
        self.order.id_at(index)
    }
    #[inline]
    pub(crate) fn order(&self) -> &BoxOrder {
        &self.order
    }
    pub(crate) fn order_snapshot(&self) -> Arc<BoxOrder> {
        Arc::clone(&self.order)
    }
    #[inline]
    pub(crate) fn keys(
        &self,
    ) -> impl DoubleEndedIterator<Item = LayoutBoxId> + ExactSizeIterator + '_ {
        self.order.ids.iter().copied()
    }

    #[inline]
    pub(crate) fn get<I: BoxLookup>(&self, index: I) -> Option<&LayoutBox<N>> {
        self.entries.get(index.key(&self.order)?)
    }

    #[inline]
    pub(crate) fn get_mut<I: BoxLookup>(&mut self, index: I) -> Option<&mut LayoutBox<N>> {
        self.entries.get_mut(index.key(&self.order)?)
    }

    #[inline]
    pub(crate) fn iter(&self) -> OrderedBoxes<'_, N> {
        OrderedBoxes {
            ids: self.order.ids.iter(),
            entries: &self.entries,
        }
    }

    // Bulk resets may visit storage in any order. Callers needing logical
    // order use id_at()/get_mut() explicitly.
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut LayoutBox<N>> {
        self.entries.values_mut()
    }

    pub(crate) fn keyed_mut(&mut self) -> impl Iterator<Item = (LayoutBoxId, &mut LayoutBox<N>)> {
        self.entries.iter_mut()
    }

    pub(crate) fn retain_in_order(&mut self, ids: Vec<LayoutBoxId>) -> bool {
        if ids == self.order.ids {
            return false;
        }
        let order = BoxOrder::new(ids);
        for id in &order.ids {
            assert!(self.entries.contains_key(*id));
        }
        self.entries.retain(|id, _| order.contains(id));
        self.order = Arc::new(order);
        true
    }
}

pub(crate) trait BoxLookup {
    fn key(self, order: &BoxOrder) -> Option<LayoutBoxId>;
}
impl BoxLookup for LayoutBoxId {
    #[inline]
    fn key(self, _order: &BoxOrder) -> Option<LayoutBoxId> {
        Some(self)
    }
}
impl BoxLookup for &LayoutBoxId {
    #[inline]
    fn key(self, _order: &BoxOrder) -> Option<LayoutBoxId> {
        Some(*self)
    }
}
impl BoxLookup for &&LayoutBoxId {
    #[inline]
    fn key(self, _order: &BoxOrder) -> Option<LayoutBoxId> {
        Some(**self)
    }
}
impl BoxLookup for &mut LayoutBoxId {
    #[inline]
    fn key(self, _order: &BoxOrder) -> Option<LayoutBoxId> {
        Some(*self)
    }
}
impl BoxLookup for usize {
    #[inline]
    fn key(self, order: &BoxOrder) -> Option<LayoutBoxId> {
        order.ids.get(self).copied()
    }
}
impl<N, I: BoxLookup> Index<I> for LayoutBoxes<N> {
    type Output = LayoutBox<N>;
    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        self.get(index).expect("layout box is not live")
    }
}
impl<N, I: BoxLookup> IndexMut<I> for LayoutBoxes<N> {
    #[inline]
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        self.get_mut(index).expect("layout box is not live")
    }
}

pub(crate) struct OrderedBoxes<'a, N> {
    ids: std::slice::Iter<'a, LayoutBoxId>,
    entries: &'a BoxMap<N>,
}
impl<'a, N> Iterator for OrderedBoxes<'a, N> {
    type Item = &'a LayoutBox<N>;
    fn next(&mut self) -> Option<Self::Item> {
        self.ids.next().map(|id| &self.entries[*id])
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.ids.size_hint()
    }
}
impl<N> DoubleEndedIterator for OrderedBoxes<'_, N> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.ids.next_back().map(|id| &self.entries[*id])
    }
}
impl<N> ExactSizeIterator for OrderedBoxes<'_, N> {}
impl<'a, N> IntoIterator for &'a LayoutBoxes<N> {
    type Item = &'a LayoutBox<N>;
    type IntoIter = OrderedBoxes<'a, N>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
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
