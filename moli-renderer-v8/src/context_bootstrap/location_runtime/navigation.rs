use super::*;

pub(in crate::context_bootstrap) fn resolve_location_navigation_target(
    scope: &mut v8::PinScope<'_, '_>,
    current_href: &str,
    kind: LocationNavigationKind,
    raw_target: Option<String>,
) -> Option<url::Url> {
    match kind {
        LocationNavigationKind::Reload => url::Url::parse(current_href).ok(),
        LocationNavigationKind::Assign | LocationNavigationKind::Replace => {
            let raw_target = raw_target.unwrap_or_default();
            if raw_target.is_empty() {
                return url::Url::parse(current_href).ok();
            }

            if let Ok(absolute) = url::Url::parse(&raw_target) {
                return Some(absolute);
            }

            // Blink's Location::SetLocation completes a relative URL against
            // EnteredDOMWindow(isolate)->document(), not the target Location's
            // current Document. This matters when an opener navigates a popup
            // more than once with the same relative string.
            let entry_context = scope.get_entered_or_microtask_context();
            if let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(entry_context)
                .or_else(|| context_host_ptr_from_global_bridge(scope))
                && let Ok(resolved) =
                    super::super::window_runtime::entered_window_api_base_url(scope, unsafe {
                        &*host_ptr
                    })
                    .join(&raw_target)
            {
                return Some(resolved);
            }

            url::Url::parse(current_href)
                .ok()
                .and_then(|base| base.join(&raw_target).ok())
        }
    }
}

pub(in crate::context_bootstrap) fn is_same_document_fragment_navigation(
    current: Option<&url::Url>,
    target: &url::Url,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    let mut current_without_fragment = current.clone();
    current_without_fragment.set_fragment(None);
    let mut target_without_fragment = target.clone();
    target_without_fragment.set_fragment(None);
    current_without_fragment == target_without_fragment
}

pub(in crate::context_bootstrap) fn urls_refer_to_same_document(
    current_href: &str,
    target_href: &str,
) -> bool {
    let Ok(current) = url::Url::parse(current_href) else {
        return false;
    };
    let Ok(target) = url::Url::parse(target_href) else {
        return false;
    };
    is_same_document_fragment_navigation(Some(&current), &target)
}
