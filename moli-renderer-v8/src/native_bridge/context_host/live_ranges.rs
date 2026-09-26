use super::*;

impl JsContextHost {
    pub(crate) fn live_ranges_is_empty(&mut self) -> bool {
        self.range_record_registry.active_is_empty()
    }

    pub(crate) fn register_live_range_record(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        range: v8::Local<'_, v8::Object>,
        handle: RangeRecordHandle,
    ) {
        self.range_record_registry
            .register_live_record(scope, handle, range);
    }
}
