use super::entries::{headers_entries, normalized_headers_entries};
use crate::util::{get_private_value, set_private_value, v8_string};

const HEADERS_ITERATION_VIEW_SLOT: &str = "__lmHeadersIterationView";

// A private, dense array of alternating names and values. Only strings leave
// this cache; each public entries() result must be a fresh pair in its realm.
pub(in crate::network_host::headers) struct HeadersIterationView<'s>(v8::Local<'s, v8::Array>);

impl<'s> HeadersIterationView<'s> {
    pub(in crate::network_host::headers) fn len(&self) -> usize {
        self.0.length() as usize / 2
    }

    pub(in crate::network_host::headers) fn get(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        index: usize,
    ) -> Option<(v8::Local<'s, v8::String>, v8::Local<'s, v8::String>)> {
        if index >= self.len() {
            return None;
        }
        let name = self.0.get_index(scope, (index * 2) as u32)?;
        let value = self.0.get_index(scope, (index * 2 + 1) as u32)?;
        Some((name.try_into().ok()?, value.try_into().ok()?))
    }
}

pub(super) fn invalidate_headers_iteration_view(
    scope: &mut v8::PinScope<'_, '_>,
    headers: v8::Local<'_, v8::Object>,
) {
    set_private_value(
        scope,
        headers,
        HEADERS_ITERATION_VIEW_SLOT,
        v8::undefined(scope).into(),
    );
}

pub(in crate::network_host::headers) fn headers_iteration_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    headers: v8::Local<'s, v8::Object>,
) -> Option<HeadersIterationView<'s>> {
    if let Some(view) = get_private_value(scope, headers, HEADERS_ITERATION_VIEW_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return Some(HeadersIterationView(view));
    }

    // All cursors share one lazy view. Writes invalidate it centrally, so
    // reentrant iteration sees mutations without changing any cursor's index.
    let entries = normalized_headers_entries(&headers_entries(scope, headers));
    let mut values = Vec::with_capacity(entries.len() * 2);
    for (name, value) in entries {
        values.push(v8_string(scope, &name)?.into());
        values.push(v8_string(scope, &value)?.into());
    }
    let view = v8::Array::new_with_elements(scope, &values);
    view.set_prototype(scope, v8::null(scope).into())?;
    set_private_value(scope, headers, HEADERS_ITERATION_VIEW_SLOT, view.into());
    Some(HeadersIterationView(view))
}
