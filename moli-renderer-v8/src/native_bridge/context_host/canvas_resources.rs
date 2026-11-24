use std::{cell::RefCell, collections::HashMap, sync::Arc};

use crate::context_bootstrap::CanvasPixels;

use crate::document_runtime::DomHandle;
use crate::native_bridge::context_host::visual_resource_generation::VisualResourceGeneration;

const MAX_RETAINED_CANVAS_PAINT_BYTES: usize = 256 * 1024 * 1024;

struct CanvasResource {
    storage: CanvasPixels,
    snapshot: RefCell<Option<Arc<moli_image::RgbaImage>>>,
}

pub(super) struct CanvasResourceStore {
    pixels_by_element: HashMap<DomHandle, CanvasResource>,
    retained_bytes: usize,
    visual_generation: VisualResourceGeneration,
}

impl CanvasResourceStore {
    pub(super) fn new(visual_generation: VisualResourceGeneration) -> Self {
        Self {
            pixels_by_element: HashMap::new(),
            retained_bytes: 0,
            visual_generation,
        }
    }

    fn replace(&mut self, element: DomHandle, storage: CanvasPixels) -> bool {
        if let Some(resource) = self.pixels_by_element.get_mut(&element)
            && resource.storage.same_storage(&storage)
        {
            resource.snapshot.get_mut().take();
            self.visual_generation.bump();
            return true;
        }
        let previous_bytes = self
            .pixels_by_element
            .get(&element)
            .map_or(0, |resource| resource.storage.len());
        let Some(next_retained_bytes) = self
            .retained_bytes
            .checked_sub(previous_bytes)
            .and_then(|bytes| bytes.checked_add(storage.len()))
        else {
            return false;
        };
        if next_retained_bytes > MAX_RETAINED_CANVAS_PAINT_BYTES {
            return false;
        }
        self.pixels_by_element.insert(
            element,
            CanvasResource {
                storage,
                snapshot: RefCell::new(None),
            },
        );
        self.retained_bytes = next_retained_bytes;
        self.visual_generation.bump();
        true
    }

    fn remove(&mut self, element: DomHandle) -> bool {
        let Some(pixels) = self.pixels_by_element.remove(&element) else {
            return false;
        };
        self.retained_bytes = self.retained_bytes.saturating_sub(pixels.storage.len());
        self.visual_generation.bump();
        true
    }

    fn get(&self, element: DomHandle) -> Option<Arc<moli_image::RgbaImage>> {
        let resource = self.pixels_by_element.get(&element)?;
        let mut snapshot = resource.snapshot.borrow_mut();
        if snapshot.is_none() {
            let storage = &resource.storage;
            let pixels = moli_image::RgbaImage::try_new(
                storage.width,
                storage.height,
                storage.pixels_copy(),
            )
            .ok()?;
            *snapshot = Some(Arc::new(pixels));
        }
        snapshot.clone()
    }

    fn elements(&self) -> impl Iterator<Item = DomHandle> + '_ {
        self.pixels_by_element.keys().copied()
    }
}

impl Default for CanvasResourceStore {
    fn default() -> Self {
        Self::new(VisualResourceGeneration::default())
    }
}

impl super::PageScriptHost {
    pub(crate) fn replace_canvas_pixels(
        &mut self,
        element: DomHandle,
        storage: CanvasPixels,
    ) -> bool {
        if self.canvas_resources.replace(element, storage) {
            return true;
        }
        // Never let a previous frame survive a failed resize or an
        // over-budget surface admission.
        self.canvas_resources.remove(element);
        false
    }

    pub(crate) fn remove_canvas_pixels(&mut self, element: DomHandle) -> bool {
        self.canvas_resources.remove(element)
    }

    pub(crate) fn canvas_pixels_for_layout(
        &self,
        element: DomHandle,
    ) -> Option<Arc<moli_image::RgbaImage>> {
        self.canvas_resources.get(element)
    }

    pub(in crate::native_bridge::context_host) fn retire_canvas_resources_for_document(
        &mut self,
        document: DomHandle,
    ) -> usize {
        // Resolve ownership at retirement time so an adopted canvas keeps its
        // bitmap with the new Document.
        let retired = self
            .canvas_resources
            .elements()
            .filter(|element| self.dom_host().owner_document_handle(*element) == Some(document))
            .collect::<Vec<_>>();
        for element in &retired {
            self.canvas_resources.remove(*element);
        }
        retired.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_canvas_resource_preserves_old_snapshot_arcs_and_exact_accounting() {
        let mut store = CanvasResourceStore::default();
        let element = DomHandle::new(7);
        assert!(store.replace(
            element,
            CanvasPixels::from_owned(2, 1, vec![255, 0, 0, 255, 255, 0, 0, 255])
        ));
        assert_eq!(store.retained_bytes, 8);
        let old_snapshot = store.get(element).expect("first canvas frame");

        assert!(store.replace(
            element,
            CanvasPixels::from_owned(1, 1, vec![0, 0, 255, 255])
        ));
        assert_eq!(store.retained_bytes, 4);
        assert_eq!(old_snapshot.rgba, [255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(store.get(element).unwrap().rgba, [0, 0, 255, 255]);

        assert!(store.remove(element));
        assert_eq!(store.retained_bytes, 0);
        assert!(!store.remove(element));
    }
}
