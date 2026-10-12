use super::*;
use crate::native_bridge::WindowDocumentTaskTarget;

/// Sample once at the end of rendering. Notification ownership follows the
/// observer's Window, independently of target-Document geometry batching and
/// the callback's relevant Realm.
pub(crate) fn sample_realtime_intersections(
    _scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) -> Result<(), moli_layout::LayoutError> {
    let mut access = ObserverHostAccess::new(host_ptr);
    let mut ids: Vec<_> = access.read(|host| {
        host.observers(&OBSERVER_STORE_ACCESS)
            .intersection_observers
            .iter()
            .filter(|(_, state)| state.callback.document_target(host).is_some())
            .map(|(id, _)| *id)
            .collect()
    });
    if ids.is_empty() {
        return Ok(());
    }
    ids.sort_unstable();
    if let Some(mut batch) = access.store(ObserverStore::take_intersection_check_batch) {
        batch.retain_observers(&ids);
        let completed = access.read(|host| {
            intersection::compute_intersection_check_batch(host, host.dom_host(), batch)
        })?;
        access.store(|store| store.apply_intersection_check_batch(completed));
    }
    let targets = access.read(|host| {
        let store = host.observers(&OBSERVER_STORE_ACCESS);
        let mut targets = Vec::new();
        for id in ids {
            let state = &store.intersection_observers[&id];
            if !state.queued_entries.is_empty()
                && let Some(target) = state.callback.document_target(host)
                && !targets.contains(&target)
            {
                targets.push(target);
            }
        }
        targets
    });
    for target in targets {
        access.mutate(|host| host.queue_intersection_observer_notification(target));
    }
    Ok(())
}

pub(crate) fn realtime_intersection_observer_ids(
    host: &JsContextHost,
    target: WindowDocumentTaskTarget,
) -> Vec<u32> {
    let mut ids: Vec<_> = host
        .observers(&OBSERVER_STORE_ACCESS)
        .intersection_observers
        .iter()
        .filter(|(_, state)| state.callback.document_target(host) == Some(target))
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

pub(crate) fn deliver_realtime_intersection_observer(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
    id: u32,
) -> bool {
    let mut access = ObserverHostAccess::new(host_ptr);
    if !access.read(|host| {
        host.observers(&OBSERVER_STORE_ACCESS)
            .intersection_observers
            .get(&id)
            .is_some_and(|state| state.callback.document_target(host) == Some(target))
    }) {
        return false;
    }
    // Take each observer's records immediately before its callback. An earlier
    // callback can takeRecords() on a later observer in the same notification.
    let delivery = access.store(|store| {
        let state = store.intersection_observers.get_mut(&id)?;
        if state.queued_entries.is_empty() {
            return None;
        }
        Some(super::super::IntersectionObserverDelivery {
            observer: v8::Local::new(scope, &state.observer),
            callback: state.callback.prepare(scope),
            entries: std::mem::take(&mut state.queued_entries),
            options: state.options.clone(),
        })
    });
    let Some(delivery) = delivery else {
        return false;
    };
    invoke_intersection_deliveries(scope, host_ptr, vec![delivery]);
    true
}
