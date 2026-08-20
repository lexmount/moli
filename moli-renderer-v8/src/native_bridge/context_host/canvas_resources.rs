use std::{collections::HashMap, sync::Arc};

use crate::document_runtime::DomHandle;

const MAX_RETAINED_CANVAS_PAINT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Default)]
pub(super) struct CanvasResourceStore {
    pixels_by_element: HashMap<DomHandle, Arc<moli_image::RgbaImage>>,
    retained_bytes: usize,
    paint_generation: u64,
}

impl CanvasResourceStore {
    fn replace(&mut self, element: DomHandle, width: u32, height: u32, rgba: Vec<u8>) -> bool {
        let Ok(pixels) = moli_image::RgbaImage::try_new(width, height, rgba) else {
            return false;
        };
        let previous_bytes = self
            .pixels_by_element
            .get(&element)
            .map_or(0, |pixels| pixels.byte_len());
        let Some(next_retained_bytes) = self
            .retained_bytes
            .checked_sub(previous_bytes)
            .and_then(|bytes| bytes.checked_add(pixels.byte_len()))
        else {
            return false;
        };
        if next_retained_bytes > MAX_RETAINED_CANVAS_PAINT_BYTES {
            return false;
        }
        self.pixels_by_element.insert(element, Arc::new(pixels));
        self.retained_bytes = next_retained_bytes;
        self.paint_generation = self.paint_generation.saturating_add(1);
        true
    }

    fn remove(&mut self, element: DomHandle) -> bool {
        let Some(pixels) = self.pixels_by_element.remove(&element) else {
            return false;
        };
        self.retained_bytes = self.retained_bytes.saturating_sub(pixels.byte_len());
        self.paint_generation = self.paint_generation.saturating_add(1);
        true
    }

    fn get(&self, element: DomHandle) -> Option<Arc<moli_image::RgbaImage>> {
        self.pixels_by_element.get(&element).cloned()
    }

    fn elements(&self) -> impl Iterator<Item = DomHandle> + '_ {
        self.pixels_by_element.keys().copied()
    }

    const fn paint_generation(&self) -> u64 {
        self.paint_generation
    }
}

impl super::JsContextHost {
    pub(crate) fn replace_canvas_pixels(
        &mut self,
        element: DomHandle,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> bool {
        if self.canvas_resources.replace(element, width, height, rgba) {
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

    pub(in crate::native_bridge::context_host) fn canvas_paint_generation(&self) -> u64 {
        self.canvas_resources.paint_generation()
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
        assert!(store.replace(element, 2, 1, vec![255, 0, 0, 255, 255, 0, 0, 255]));
        assert_eq!(store.retained_bytes, 8);
        let old_snapshot = store.get(element).expect("first canvas frame");

        assert!(store.replace(element, 1, 1, vec![0, 0, 255, 255]));
        assert_eq!(store.retained_bytes, 4);
        assert_eq!(old_snapshot.rgba, [255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(store.get(element).unwrap().rgba, [0, 0, 255, 255]);

        assert!(store.remove(element));
        assert_eq!(store.retained_bytes, 0);
        assert!(!store.remove(element));
    }
}
