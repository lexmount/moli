//! URL parsing captures the blob entry, including a failed lookup. Revoking
//! the URL must not invalidate an existing Request or a successfully opened
//! XHR. Private carriers retain shared bytes until their last JS owner is
//! collected; the isolate store also releases them on isolate teardown.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use crate::util::{get_private_value, set_private_value};

pub(in crate::network_host) const BLOB_URL_ENTRY_SLOT: &str = "__lmBlobUrlEntry";
const ID_SLOT: &str = "__lmBlobUrlEntryId";

#[derive(Clone)]
pub(crate) struct CapturedBlobUrl {
    url: url::Url,
    data: Option<(Arc<[u8]>, String)>,
}

impl CapturedBlobUrl {
    pub(crate) fn capture(url: &url::Url) -> Option<Self> {
        if url.scheme() != "blob" {
            return None;
        }
        let mut url = url.clone();
        url.set_fragment(None);
        let data = crate::blob::object_url_shared_bytes_and_type(url.as_str());
        Some(Self { url, data })
    }

    pub(super) fn response(&self, url: &url::Url) -> Option<super::Response> {
        let (body, mime_type) = self.data.as_ref()?;
        Some(super::browser_response::blob_response(
            url,
            body.to_vec(),
            mime_type.clone(),
        ))
    }

    pub(super) fn matches(&self, url: &url::Url) -> bool {
        self.url.as_str()
            == url
                .as_str()
                .split_once('#')
                .map_or(url.as_str(), |(url, _)| url)
    }
}

type Store = Rc<RefCell<Entries>>;

#[derive(Default)]
struct Entries {
    next_id: u64,
    values: HashMap<u64, (v8::Weak<v8::Object>, CapturedBlobUrl)>,
}

pub(crate) fn set_blob_url_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry: Option<CapturedBlobUrl>,
) {
    let Some(entry) = entry else {
        set_private_value(
            scope,
            owner,
            BLOB_URL_ENTRY_SLOT,
            v8::undefined(scope).into(),
        );
        return;
    };
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("blob entry identity exhausted");
        store.next_id
    };
    let carrier = v8::Object::new(scope);
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        carrier,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().values.remove(&id);
            }
        }),
    );
    store.borrow_mut().values.insert(id, (weak, entry));
    set_private_value(
        scope,
        carrier,
        ID_SLOT,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
    set_private_value(scope, owner, BLOB_URL_ENTRY_SLOT, carrier.into());
}

pub(crate) fn blob_url_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<CapturedBlobUrl> {
    let carrier = get_private_value(scope, owner, BLOB_URL_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let id = get_private_value(scope, carrier, ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())?
        .u64_value()
        .0;
    scope
        .get_slot::<Store>()?
        .borrow()
        .values
        .get(&id)
        .map(|(_, entry)| entry.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> CapturedBlobUrl {
        CapturedBlobUrl {
            url: url::Url::parse("blob:https://example.test/captured").unwrap(),
            data: Some((Arc::from([0, 128, 255]), "application/example".to_owned())),
        }
    }

    #[test]
    fn private_entries_follow_the_last_owner_across_realms_gc_and_isolate_drop() {
        moli_v8_test_util::ensure_v8();
        let weak = {
            let mut isolate = v8::Isolate::new(Default::default());
            let (weak, surviving_owner) = {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let first = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, first);
                let owner = v8::Object::new(scope);
                let entry = entry();
                let weak = Arc::downgrade(&entry.data.as_ref().unwrap().0);
                set_blob_url_entry(scope, owner, Some(entry));
                let second = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, second);
                let other = v8::Object::new(scope);
                let carrier = get_private_value(scope, owner, BLOB_URL_ENTRY_SLOT).unwrap();
                set_private_value(scope, other, BLOB_URL_ENTRY_SLOT, carrier);
                let captured = blob_url_entry(scope, other).unwrap();
                assert!(Arc::ptr_eq(
                    &captured.data.unwrap().0,
                    &weak.upgrade().unwrap()
                ));
                (weak, v8::Global::new(scope, other))
            };
            isolate.low_memory_notification();
            assert!(weak.upgrade().is_some());
            drop(surviving_owner);
            isolate.low_memory_notification();
            assert!(weak.upgrade().is_none());
            assert!(
                isolate
                    .get_slot::<Store>()
                    .unwrap()
                    .borrow()
                    .values
                    .is_empty()
            );

            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let owner = v8::Object::new(scope);
            let entry = entry();
            let weak = Arc::downgrade(&entry.data.as_ref().unwrap().0);
            set_blob_url_entry(scope, owner, Some(entry));
            weak
        };
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn captured_entries_match_fragments_but_do_not_override_other_urls_or_methods() {
        let captured = entry();
        let mut url = captured.url.clone();
        url.set_fragment(Some("fragment"));
        let response =
            super::super::local_url_response_with_blob_entry(&url, "GET", Some(&captured))
                .unwrap()
                .unwrap();
        assert_eq!(response.head().status, 200);
        let (_, body) = response.into_body();
        assert_eq!(
            body.try_into_materialized_bytes().unwrap(),
            vec![0, 128, 255]
        );
        assert!(
            super::super::local_url_response_with_blob_entry(&url, "POST", Some(&captured))
                .unwrap()
                .is_err()
        );
        let other = url::Url::parse("blob:https://example.test/other").unwrap();
        assert!(
            super::super::local_url_response_with_blob_entry(&other, "GET", Some(&captured))
                .unwrap()
                .is_err()
        );
        let missing = CapturedBlobUrl { url, data: None };
        assert!(missing.response(&missing.url).is_none());
    }
}
