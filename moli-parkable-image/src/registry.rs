use std::collections::HashMap;

use crate::image::{ParkableImage, WeakParkableImage};

#[derive(Default)]
pub(crate) struct ParkableImageRegistry {
    resident: HashMap<u64, WeakParkableImage>,
    parked: HashMap<u64, WeakParkableImage>,
}

impl ParkableImageRegistry {
    pub(crate) fn register_resident(&mut self, image: &ParkableImage) {
        let id = image.id();
        debug_assert!(!self.parked.contains_key(&id));
        let previous = self.resident.insert(id, image.downgrade());
        debug_assert!(previous.is_none());
    }

    pub(crate) fn move_to_parked(&mut self, image: &ParkableImage) {
        let id = image.id();
        let resident = self.resident.remove(&id);
        debug_assert!(resident.is_some());
        let previous = self.parked.insert(id, image.downgrade());
        debug_assert!(previous.is_none());
    }

    pub(crate) fn move_to_resident(&mut self, image: &ParkableImage) {
        let id = image.id();
        let parked = self.parked.remove(&id);
        debug_assert!(parked.is_some());
        let previous = self.resident.insert(id, image.downgrade());
        debug_assert!(previous.is_none());
    }

    pub(crate) fn remove(&mut self, id: u64) -> bool {
        self.resident.remove(&id).is_some() || self.parked.remove(&id).is_some()
    }

    pub(crate) fn resident_images(&mut self) -> Vec<ParkableImage> {
        let mut images = Vec::with_capacity(self.resident.len());
        self.resident.retain(|_, registered| {
            if let Some(image) = registered.upgrade() {
                images.push(image);
                true
            } else {
                false
            }
        });
        images
    }

    pub(crate) fn all_images(&mut self) -> Vec<ParkableImage> {
        let mut images = self.resident_images();
        images.reserve(self.parked.len());
        self.parked.retain(|_, registered| {
            if let Some(image) = registered.upgrade() {
                images.push(image);
                true
            } else {
                false
            }
        });
        images
    }
}
