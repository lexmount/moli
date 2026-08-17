use std::collections::HashMap;

use crate::{ParkableImage, WeakParkableImage};

#[derive(Clone)]
pub(crate) struct ResidentImage {
    pub(crate) id: u64,
    pub(crate) image: ParkableImage,
    pub(crate) capacity_blocked: bool,
}

#[derive(Default)]
pub(crate) struct ParkableImageRegistry {
    resident: HashMap<u64, ResidentEntry>,
    parked: HashMap<u64, WeakParkableImage>,
}

struct ResidentEntry {
    image: WeakParkableImage,
    capacity_blocked: bool,
}

impl ParkableImageRegistry {
    pub(crate) fn register_resident(&mut self, image: &ParkableImage) {
        let id = image.id();
        debug_assert!(!self.parked.contains_key(&id));
        let previous = self.resident.insert(
            id,
            ResidentEntry {
                image: image.downgrade(),
                capacity_blocked: false,
            },
        );
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
        let previous = self.resident.insert(
            id,
            ResidentEntry {
                image: image.downgrade(),
                capacity_blocked: false,
            },
        );
        debug_assert!(previous.is_none());
    }

    pub(crate) fn remove(&mut self, id: u64) -> bool {
        self.resident.remove(&id).is_some() || self.parked.remove(&id).is_some()
    }

    pub(crate) fn block_for_capacity(&mut self, id: u64) {
        if let Some(image) = self.resident.get_mut(&id) {
            image.capacity_blocked = true;
        }
    }

    pub(crate) fn unblock_capacity_waiters(&mut self) {
        for image in self.resident.values_mut() {
            image.capacity_blocked = false;
        }
    }

    pub(crate) fn resident_images(&mut self) -> Vec<ResidentImage> {
        let before = self.resident.len();
        let mut images = Vec::with_capacity(before);
        self.resident.retain(|&id, registered| {
            if let Some(image) = registered.image.upgrade() {
                images.push(ResidentImage {
                    id,
                    image,
                    capacity_blocked: registered.capacity_blocked,
                });
                true
            } else {
                false
            }
        });
        if self.resident.len() != before {
            // A dead image may have released a disk extent before its weak
            // registration was pruned.
            self.unblock_capacity_waiters();
        }
        images
    }

    pub(crate) fn all_images(&mut self) -> Vec<ParkableImage> {
        let mut images = self
            .resident_images()
            .into_iter()
            .map(|registered| registered.image)
            .collect::<Vec<_>>();
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
