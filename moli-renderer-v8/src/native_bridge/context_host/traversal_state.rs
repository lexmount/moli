use super::*;

impl PageScriptHost {
    pub(crate) fn node_iterators_is_empty(&self) -> bool {
        self.native_bridge().node_iterators_is_empty()
    }
}
