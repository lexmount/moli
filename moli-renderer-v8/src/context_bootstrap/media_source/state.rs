use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{util::get_private_value, web_api_interfaces};
use moli_webapi_declare::WebApiObject;

const MEDIA_SOURCE_ID_SLOT: &str = "__moliMediaSourceId";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Native identity retained by the wrapper, object URLs and parsed URL entries.
/// It remains independent of the V8 wrapper's lifetime and contains no Blob data.
#[derive(Debug)]
pub(crate) struct MediaSourceObject {
    id: u64,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaSource)]
struct MediaSourceInstanceDeclaration<'scope> {
    #[webapi(slot = MEDIA_SOURCE_ID_SLOT)]
    id: v8::Local<'scope, v8::BigInt>,
}

type Store = Rc<RefCell<MediaSourceObjects>>;

#[derive(Default)]
struct MediaSourceObjects {
    entries: HashMap<u64, (v8::Weak<v8::Object>, Arc<MediaSourceObject>)>,
}

pub(super) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) {
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .expect("MediaSource identity exhausted");
    let source = Arc::new(MediaSourceObject { id });
    MediaSourceInstanceDeclaration::new(v8::BigInt::new_from_u64(scope, source.id))
        .initialize(scope, object)
        .expect("MediaSource instance should initialize");
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store.borrow_mut().entries.insert(id, (weak, source));
}

pub(crate) fn media_source_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Arc<MediaSourceObject>> {
    if !web_api_interfaces::MediaSource::is_instance(scope, object) {
        return None;
    }
    let id = get_private_value(scope, object, MEDIA_SOURCE_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())?
        .u64_value()
        .0;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)
        .map(|(_, source)| source.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_file_api::{BlobStore, ObjectUrlData, ObjectUrlTarget};

    #[test]
    fn media_source_urls_retain_identity_after_wrapper_gc_and_isolate_drop() {
        moli_v8_test_util::ensure_v8();
        let urls = BlobStore::<u64, u64, (), (), Arc<MediaSourceObject>>::default();
        let (url, weak) = {
            let mut isolate = v8::Isolate::new(Default::default());
            let (url, weak) = {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let object = v8::Object::new(scope);
                initialize(scope, object);
                let source = media_source_object(scope, object).unwrap();
                assert!(Arc::ptr_eq(
                    &source,
                    &media_source_object(scope, object).unwrap()
                ));
                let other = v8::Object::new(scope);
                initialize(scope, other);
                assert!(!Arc::ptr_eq(
                    &source,
                    &media_source_object(scope, other).unwrap()
                ));
                let weak = Arc::downgrade(&source);
                let url = urls
                    .create_object_url_with_target(
                        Some(1),
                        Some(2),
                        ObjectUrlTarget::MediaSource(source),
                        "https://example.test",
                        None,
                        None,
                    )
                    .unwrap();
                (url, weak)
            };
            isolate.low_memory_notification();
            assert!(
                isolate
                    .get_slot::<Store>()
                    .unwrap()
                    .borrow()
                    .entries
                    .is_empty()
            );
            assert!(
                weak.upgrade().is_some(),
                "URL retains the associated native object"
            );
            (url, weak)
        };
        let Some(ObjectUrlData::MediaSource(captured)) = urls.object_url_data(&url) else {
            panic!("URL must retain its MediaSource kind");
        };
        assert!(Arc::ptr_eq(&captured, &weak.upgrade().unwrap()));
        assert_eq!(urls.cleanup_object_url_lifetime(1, 2), 1);
        assert!(urls.object_url_data(&url).is_none());
        assert!(
            weak.upgrade().is_some(),
            "parsed URL retains the native object"
        );
        drop(captured);
        assert!(weak.upgrade().is_none());
    }
}
