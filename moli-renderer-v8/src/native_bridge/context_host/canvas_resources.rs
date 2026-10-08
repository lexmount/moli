use std::{collections::HashMap, sync::Arc};

use crate::document_runtime::DomHandle;
use crate::native_bridge::context_host::visual_resource_generation::VisualResourceGeneration;

const MAX_RETAINED_CANVAS_PAINT_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct CanvasResourceStore {
    pixels_by_element: HashMap<DomHandle, Arc<moli_image::RgbaImage>>,
    retained_bytes: usize,
    visual_generation: VisualResourceGeneration,
}

impl CanvasResourceStore {
    pub(super) fn transfer_from(
        &mut self,
        source: &mut Self,
        handles: &HashMap<DomHandle, DomHandle>,
    ) {
        let mut changed = false;
        for (&old, &new) in handles {
            if let Some(pixels) = source.pixels_by_element.remove(&old) {
                source.retained_bytes -= pixels.byte_len();
                self.retained_bytes = self
                    .retained_bytes
                    .checked_add(pixels.byte_len())
                    .expect("retained canvas byte accounting fits usize");
                assert!(self.pixels_by_element.insert(new, pixels).is_none());
                changed = true;
            }
        }
        if changed {
            source.visual_generation.bump();
            self.visual_generation.bump();
        }
    }

    pub(super) fn new(visual_generation: VisualResourceGeneration) -> Self {
        Self {
            pixels_by_element: HashMap::new(),
            retained_bytes: 0,
            visual_generation,
        }
    }

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
        self.visual_generation.bump();
        true
    }

    fn remove(&mut self, element: DomHandle) -> bool {
        let Some(pixels) = self.pixels_by_element.remove(&element) else {
            return false;
        };
        self.retained_bytes = self.retained_bytes.saturating_sub(pixels.byte_len());
        self.visual_generation.bump();
        true
    }

    fn get(&self, element: DomHandle) -> Option<Arc<moli_image::RgbaImage>> {
        self.pixels_by_element.get(&element).cloned()
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
    fn ownership_transfer_preserves_canvas_pixels_and_both_hosts_accounting() {
        let mut source = CanvasResourceStore::default();
        let mut target = CanvasResourceStore::default();
        let source_node = DomHandle::new(7);
        let unmoved_source = DomHandle::new(8);
        let target_node = DomHandle::new(12);
        let unmoved_target = DomHandle::new(7);
        assert!(source.replace(source_node, 1, 1, vec![1, 2, 3, 4]));
        assert!(source.replace(unmoved_source, 1, 1, vec![5, 6, 7, 8]));
        assert!(target.replace(unmoved_target, 1, 1, vec![9, 10, 11, 12]));
        let retained = source.get(source_node).unwrap();
        target.transfer_from(&mut source, &HashMap::from([(source_node, target_node)]));
        assert!(source.get(source_node).is_none());
        assert!(Arc::ptr_eq(&retained, &target.get(target_node).unwrap()));
        assert_eq!(source.retained_bytes, 4);
        assert_eq!(target.retained_bytes, 8);
        assert_eq!(source.get(unmoved_source).unwrap().rgba, [5, 6, 7, 8]);
        assert_eq!(target.get(unmoved_target).unwrap().rgba, [9, 10, 11, 12]);
        source.transfer_from(
            &mut target,
            &HashMap::from([(target_node, DomHandle::new(20))]),
        );
        assert!(Arc::ptr_eq(
            &retained,
            &source.get(DomHandle::new(20)).unwrap()
        ));
        assert_eq!(source.retained_bytes, 8);
        assert_eq!(target.retained_bytes, 4);
    }

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
