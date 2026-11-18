use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use moli_fetch::{RequestMode, ResponseHead};
use moli_fetch_types::{FetchResponseMetadata, FetchResponseType};
use moli_url::WebOrigin;
use url::Url;

use crate::{
    network_host::fetch_surface::{response_slot_value, set_response_slot_value},
    types::AsyncSubresourceFetchResponseFilter,
    util::{get_private_value, set_private_value, v8_string},
};

pub(in crate::network_host) const RESPONSE_METADATA_SLOT: &str = "__moliResponseMetadata";
const METADATA_ID_SLOT: &str = "__moliResponseMetadataId";

pub(super) struct ResponseObjectMetadata {
    pub(super) response: Arc<FetchResponseMetadata>,
    pub(super) preload_redirect_url: Option<Url>,
}

impl From<Arc<FetchResponseMetadata>> for ResponseObjectMetadata {
    fn from(response: Arc<FetchResponseMetadata>) -> Self {
        Self {
            response,
            preload_redirect_url: None,
        }
    }
}

type MetadataStore = Rc<RefCell<ResponseMetadataEntries>>;

#[derive(Default)]
struct ResponseMetadataEntries {
    next_id: u64,
    entries: HashMap<u64, (v8::Weak<v8::Object>, Rc<ResponseObjectMetadata>)>,
}

pub(crate) fn set_response_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    metadata: Arc<FetchResponseMetadata>,
) {
    install_response_metadata(scope, response, metadata.into());
}

pub(super) fn install_response_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    metadata: ResponseObjectMetadata,
) {
    let store = if let Some(store) = scope.get_slot::<MetadataStore>() {
        Rc::clone(store)
    } else {
        let store = MetadataStore::default();
        scope.set_slot(Rc::clone(&store));
        store
    };
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("Response metadata identity exhausted");
        store.next_id
    };

    // Clones share the private anchor. Its weak handle keeps the native
    // snapshot alive while any Response refers to it, even across realms.
    let anchor = v8::Object::new(scope);
    let id_value = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, anchor, METADATA_ID_SLOT, id_value.into());
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        anchor,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store
        .borrow_mut()
        .entries
        .insert(id, (weak, Rc::new(metadata)));
    set_response_slot_value(scope, response, RESPONSE_METADATA_SLOT, anchor.into());
}

fn response_object_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> Option<Rc<ResponseObjectMetadata>> {
    let anchor = response_slot_value(scope, response, RESPONSE_METADATA_SLOT)?;
    let anchor = v8::Local::<v8::Object>::try_from(anchor).ok()?;
    let id = get_private_value(scope, anchor, METADATA_ID_SLOT)?;
    let id = v8::Local::<v8::BigInt>::try_from(id).ok()?.u64_value().0;
    let store = scope.get_slot::<MetadataStore>()?.borrow();
    store
        .entries
        .get(&id)
        .map(|(_, metadata)| Rc::clone(metadata))
}

pub(super) fn response_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> Option<FetchResponseMetadata> {
    Some((*response_object_metadata(scope, response)?.response).clone())
}

pub(in crate::network_host) fn response_metadata_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let snapshot = response_object_metadata(scope, response)?;
    let metadata = &snapshot.response;
    match attribute {
        "type" => v8_string(scope, metadata.response_type.as_str()).map(Into::into),
        "redirected" => Some(
            v8::Boolean::new(
                scope,
                metadata.response_type.is_readable() && metadata.redirected,
            )
            .into(),
        ),
        "url" => {
            let url = metadata
                .final_url()
                .filter(|_| metadata.response_type != FetchResponseType::Opaque)
                .map(|url| {
                    snapshot
                        .preload_redirect_url
                        .as_ref()
                        .unwrap_or(url)
                        .as_str()
                })
                .unwrap_or_default();
            let url = url.split_once('#').map_or(url, |(url, _)| url);
            v8_string(scope, url).map(Into::into)
        }
        _ => None,
    }
}

/// The response's filtering and timing permission travel together. A Service
/// Worker delivery also suppresses transfer accounting for this request.
pub(crate) struct FetchResponseDelivery {
    pub(crate) metadata: Arc<FetchResponseMetadata>,
    pub(crate) from_service_worker: bool,
}

impl FetchResponseDelivery {
    pub(crate) fn for_request(
        origin: &WebOrigin,
        head: &ResponseHead,
        mode: RequestMode,
        filter: Option<&AsyncSubresourceFetchResponseFilter>,
        web_security_disabled: bool,
    ) -> Self {
        let service_worker_metadata = filter.and_then(|filter| filter.service_worker_metadata());
        let metadata = if let Some(metadata) = service_worker_metadata {
            let mut metadata = metadata.clone();
            if metadata.response_type == FetchResponseType::Default {
                metadata.response_type = FetchResponseType::Basic;
            }
            if metadata.url_list.is_empty() {
                metadata.url_list = head.url_list().to_vec();
            }
            metadata.redirected |= head.redirected;
            metadata
        } else {
            let mut metadata =
                head.fetch_metadata_with_web_security(origin, mode, web_security_disabled);
            if let Some(filter) = filter {
                metadata.response_type = filter.response_type();
            }
            metadata
        };
        Self {
            metadata: Arc::new(metadata),
            from_service_worker: service_worker_metadata.is_some(),
        }
    }

    pub(crate) fn block_opaque_response(&mut self) {
        let metadata = Arc::make_mut(&mut self.metadata);
        metadata.response_type = FetchResponseType::Opaque;
        metadata.timing_allow_passed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_metadata_lives_until_the_last_anchor_reference_is_collected() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (metadata, clone) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let response = v8::Object::new(scope);
            let metadata = Arc::new(FetchResponseMetadata {
                response_type: FetchResponseType::Opaque,
                url_list: vec![Url::parse("https://cross.test/private#fragment").unwrap()],
                timing_allow_passed: true,
                redirected: true,
            });
            let weak_metadata = Arc::downgrade(&metadata);
            set_response_metadata(scope, response, metadata);
            let clone = v8::Object::new(scope);
            let anchor = response_slot_value(scope, response, RESPONSE_METADATA_SLOT).unwrap();
            set_response_slot_value(scope, clone, RESPONSE_METADATA_SLOT, anchor);
            assert_eq!(
                response_metadata(scope, clone).unwrap(),
                *weak_metadata.upgrade().unwrap()
            );
            (weak_metadata, v8::Global::new(scope, clone))
        };

        isolate.low_memory_notification();
        assert!(metadata.upgrade().is_some());
        drop(clone);
        isolate.low_memory_notification();
        assert!(metadata.upgrade().is_none());
    }
}
