use crate::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    style_engine::{computed_longhand_count, computed_longhand_name_at},
};

use super::declaration::{ComputedStyleRead, StyleComputationContext, computed_style_applies};

pub(super) fn computed_property_count(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> usize {
    if !computed_style_applies(runtime, handle) {
        return 0;
    }
    computed_longhand_count()
        + sorted_custom_property_names(&ComputedStyleRead::new_with_context(
            runtime, handle, context,
        ))
        .len()
}

pub(super) fn computed_property_name_at(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    index: usize,
) -> Option<String> {
    if !computed_style_applies(runtime, handle) {
        return None;
    }

    let longhand_count = computed_longhand_count();
    if index < longhand_count {
        return computed_longhand_name_at(index).map(str::to_owned);
    }

    let custom_names = sorted_custom_property_names(&ComputedStyleRead::new_with_context(
        runtime, handle, context,
    ));
    custom_names.get(index - longhand_count).cloned()
}

pub(super) fn computed_property_names(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> Vec<String> {
    if !computed_style_applies(runtime, handle) {
        return Vec::new();
    }
    let read = ComputedStyleRead::new_with_context(runtime, handle, context);
    computed_property_names_for_read(&read)
}

pub(super) fn computed_property_names_for_read(read: &ComputedStyleRead<'_>) -> Vec<String> {
    let mut names = Vec::with_capacity(computed_longhand_count());
    names.extend(
        (0..computed_longhand_count())
            .filter_map(computed_longhand_name_at)
            .map(str::to_owned),
    );
    names.extend(sorted_custom_property_names(read));
    names
}

fn sorted_custom_property_names(read: &ComputedStyleRead<'_>) -> Vec<String> {
    let mut names = read.custom_property_names();
    names.sort_unstable();
    names.dedup();
    names
}
