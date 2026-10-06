//! URL parsing captures the blob entry, including a failed lookup. Revoking
//! the URL must not invalidate an existing Request or a successfully opened
//! XHR. Private carriers retain the Blob bytes or native MediaSource identity
//! until their last JS owner is collected; isolate teardown also releases them.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use crate::util::{get_private_value, set_private_value};
use moli_file_api::ObjectUrlData;

/// Fetch authority comes from the pinned execution context, never from the
/// serialized blob URL or the realm in which an argument getter runs.
pub(crate) struct BlobUrlFetchEnvironment {
    access_key: crate::blob::ObjectUrlAccessKey,
    top_level_creation_url: Option<url::Url>,
}

impl BlobUrlFetchEnvironment {
    pub(crate) fn for_window(
        host: &mut crate::native_bridge::JsContextHost,
        binding: &crate::native_bridge::WindowExecutionContextBinding,
    ) -> Option<Self> {
        let identity = binding.resolve_identity(host)?;
        let storage_context =
            host.storage_context_for_window_execution_context_identity(identity)?;
        let dispatch_scope = identity.dispatch_scope();
        let top_level_creation_url = match dispatch_scope {
            crate::native_bridge::OwnerDispatchScope::Child(_) => None,
            crate::native_bridge::OwnerDispatchScope::Top
            | crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => {
                let loader = host.document_resource_loader_for_dispatch_scope(dispatch_scope)?;
                Some(loader.creation_url().clone())
            }
        };
        Some(Self {
            access_key: crate::blob::ObjectUrlAccessKey::new(
                host.browser_context_runtime().storage_partition_identity(),
                storage_context.storage_key().clone(),
            ),
            top_level_creation_url,
        })
    }

    pub(crate) fn for_worker(scope: &mut v8::PinScope<'_, '_>) -> Option<Self> {
        Some(Self {
            access_key: crate::blob::ObjectUrlAccessKey::for_worker(scope)?,
            top_level_creation_url: None,
        })
    }
}

pub(in crate::network_host) const BLOB_URL_ENTRY_SLOT: &str = "__lmBlobUrlEntry";
const ID_SLOT: &str = "__lmBlobUrlEntryId";

#[derive(Clone)]
pub(crate) struct CapturedBlobUrl {
    url: url::Url,
    entry: Option<Arc<crate::blob::RendererObjectUrlEntry>>,
    fetch_authorized: bool,
}

impl CapturedBlobUrl {
    pub(crate) fn capture(url: &url::Url) -> Option<Self> {
        if url.scheme() != "blob" {
            return None;
        }
        let url = url.clone();
        let entry = crate::blob::object_url_entry(url.as_str()).map(Arc::new);
        Some(Self {
            url,
            entry,
            fetch_authorized: false,
        })
    }

    /// Scope a copy for this fetch without changing the parsed Request/XHR
    /// entry. A denied lookup stays captured and must not fall back to a fresh
    /// unrestricted lookup in the store.
    pub(crate) fn for_environment(mut self, environment: Option<&BlobUrlFetchEnvironment>) -> Self {
        self.fetch_authorized = self.entry.as_ref().is_some_and(|entry| {
            environment.is_some_and(|environment| {
                entry.access_key.as_ref().is_some_and(|key| {
                    key == &environment.access_key
                    // Fetch explicitly exempts a top-level document fetching
                    // its own creation URL from storage-key partition checks.
                    || (key.same_partition(&environment.access_key)
                        && environment.top_level_creation_url.as_ref() == Some(&self.url))
                })
            })
        });
        if !self.fetch_authorized {
            self.entry = None;
        }
        self
    }

    pub(crate) fn is_authorized_fetch(&self, url: &url::Url) -> bool {
        self.fetch_authorized && self.matches(url)
    }

    pub(super) fn response(
        &self,
        url: &url::Url,
        request_headers: &[(String, String)],
    ) -> Option<Result<super::Response, super::browser_response::LocalUrlError>> {
        let ObjectUrlData::Blob {
            bytes: body,
            mime_type,
        } = &self.entry.as_ref()?.data
        else {
            // Fetch's blob scheme only reads Blob objects, never MediaSource.
            return Some(Err(super::browser_response::LocalUrlError::MediaSource));
        };
        Some(super::browser_response::blob_response(
            url,
            body,
            mime_type,
            request_headers,
        ))
    }

    pub(super) fn matches(&self, url: &url::Url) -> bool {
        self.url
            .as_str()
            .split_once('#')
            .map_or(self.url.as_str(), |(url, _)| url)
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
    use std::sync::Arc;

    fn bytes(entry: &CapturedBlobUrl) -> &Arc<[u8]> {
        let Some(ObjectUrlData::Blob { bytes, .. }) = entry.entry.as_ref().map(|entry| &entry.data)
        else {
            panic!("expected captured Blob bytes");
        };
        bytes
    }

    fn entry() -> CapturedBlobUrl {
        CapturedBlobUrl {
            url: url::Url::parse("blob:https://example.test/captured").unwrap(),
            entry: Some(Arc::new(moli_file_api::ObjectUrlEntry {
                data: ObjectUrlData::Blob {
                    bytes: Arc::from([0, 128, 255]),
                    mime_type: "application/example".to_owned(),
                },
                access_key: None,
            })),
            fetch_authorized: false,
        }
    }

    #[test]
    fn fetch_access_uses_full_creator_identity_without_mutating_the_parsed_entry() {
        use crate::runtime::RendererBrowserContextRuntime;
        use moli_storage_key::{MoliStorageKey, OpaqueOriginNonce};
        let browser = RendererBrowserContextRuntime::new();
        let partition = browser.handle().storage_partition_identity();
        let url = url::Url::parse("data:text/plain,opaque").unwrap();
        let storage_key =
            MoliStorageKey::first_party_from_url(&url, Some(OpaqueOriginNonce::new(101)));
        let creator_key =
            crate::blob::ObjectUrlAccessKey::new(partition.clone(), storage_key.clone());
        let mut captured = entry();
        captured.url = url::Url::parse("blob:null/captured").unwrap();
        Arc::get_mut(captured.entry.as_mut().unwrap())
            .unwrap()
            .access_key = Some(creator_key.clone());
        let own = BlobUrlFetchEnvironment {
            access_key: creator_key,
            top_level_creation_url: None,
        };
        let other_browser = RendererBrowserContextRuntime::new();
        let foreign_keys = [
            crate::blob::ObjectUrlAccessKey::new(
                other_browser.handle().storage_partition_identity(),
                storage_key.clone(),
            ),
            crate::blob::ObjectUrlAccessKey::new(
                partition.clone(),
                MoliStorageKey::first_party_from_url(&url, Some(OpaqueOriginNonce::new(102))),
            ),
            crate::blob::ObjectUrlAccessKey::new(
                partition.clone(),
                MoliStorageKey::from_url_and_top_level_site(
                    &url,
                    "https://another-site.test".to_owned(),
                    storage_key.opaque_nonce(),
                ),
            ),
            crate::blob::ObjectUrlAccessKey::new(partition, storage_key.with_cross_site_ancestor()),
        ];
        for access_key in foreign_keys {
            let environment = BlobUrlFetchEnvironment {
                access_key,
                top_level_creation_url: None,
            };
            let denied = captured.clone().for_environment(Some(&environment));
            assert!(!denied.is_authorized_fetch(&captured.url));
            assert!(
                super::super::local_url_response_with_blob_entry(
                    &captured.url,
                    "GET",
                    &[],
                    Some(&denied)
                )
                .unwrap()
                .is_err()
            );
        }
        assert!(
            captured
                .clone()
                .for_environment(None)
                .response(&captured.url, &[])
                .is_none()
        );
        let authorized = captured.clone().for_environment(Some(&own));
        assert!(authorized.is_authorized_fetch(&captured.url));
        assert_eq!(
            authorized
                .response(&captured.url, &[])
                .unwrap()
                .unwrap()
                .body_bytes(),
            [0, 128, 255]
        );
        assert!(captured.entry.is_some());
        assert!(!captured.fetch_authorized);
        let mut fragment = captured.url.clone();
        fragment.set_fragment(Some("fragment"));
        assert!(authorized.is_authorized_fetch(&fragment));
        assert!(!authorized.is_authorized_fetch(&url));
    }

    #[test]
    fn top_level_self_fetch_exception_does_not_cross_browser_partitions_or_cover_other_urls() {
        use crate::runtime::RendererBrowserContextRuntime;
        use moli_storage_key::MoliStorageKey;
        let browser = RendererBrowserContextRuntime::new();
        let partition = browser.handle().storage_partition_identity();
        let url = url::Url::parse("https://example.test/").unwrap();
        let creator = MoliStorageKey::first_party_from_url(&url, None);
        let embedded = MoliStorageKey::from_url_and_top_level_site(
            &url,
            "https://foreign.test".to_owned(),
            None,
        );
        let mut captured = entry();
        Arc::get_mut(captured.entry.as_mut().unwrap())
            .unwrap()
            .access_key = Some(crate::blob::ObjectUrlAccessKey::new(
            partition.clone(),
            embedded,
        ));
        let mut environment = BlobUrlFetchEnvironment {
            access_key: crate::blob::ObjectUrlAccessKey::new(partition, creator.clone()),
            top_level_creation_url: Some(captured.url.clone()),
        };
        assert!(
            captured
                .clone()
                .for_environment(Some(&environment))
                .is_authorized_fetch(&captured.url)
        );
        let mut different_fragment = captured.clone();
        different_fragment.url.set_fragment(Some("other"));
        assert!(
            !different_fragment
                .clone()
                .for_environment(Some(&environment))
                .is_authorized_fetch(&different_fragment.url)
        );
        environment.top_level_creation_url = Some(url);
        assert!(
            !captured
                .clone()
                .for_environment(Some(&environment))
                .is_authorized_fetch(&captured.url)
        );
        environment.top_level_creation_url = Some(captured.url.clone());
        let other_browser = RendererBrowserContextRuntime::new();
        environment.access_key = crate::blob::ObjectUrlAccessKey::new(
            other_browser.handle().storage_partition_identity(),
            creator,
        );
        assert!(
            !captured
                .clone()
                .for_environment(Some(&environment))
                .is_authorized_fetch(&captured.url)
        );
        let unkeyed = entry();
        assert!(
            !unkeyed
                .for_environment(Some(&environment))
                .is_authorized_fetch(&captured.url)
        );
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
                let weak = Arc::downgrade(bytes(&entry));
                set_blob_url_entry(scope, owner, Some(entry));
                let second = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, second);
                let other = v8::Object::new(scope);
                let carrier = get_private_value(scope, owner, BLOB_URL_ENTRY_SLOT).unwrap();
                set_private_value(scope, other, BLOB_URL_ENTRY_SLOT, carrier);
                let captured = blob_url_entry(scope, other).unwrap();
                assert!(Arc::ptr_eq(bytes(&captured), &weak.upgrade().unwrap()));
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
            let weak = Arc::downgrade(bytes(&entry));
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
            super::super::local_url_response_with_blob_entry(&url, "GET", &[], Some(&captured))
                .unwrap()
                .unwrap();
        assert_eq!(response.head().status, 200);
        let (_, body) = response.into_body();
        assert_eq!(
            body.try_into_materialized_bytes().unwrap(),
            vec![0, 128, 255]
        );
        assert!(
            super::super::local_url_response_with_blob_entry(&url, "POST", &[], Some(&captured))
                .unwrap()
                .is_err()
        );
        let other = url::Url::parse("blob:https://example.test/other").unwrap();
        assert!(
            super::super::local_url_response_with_blob_entry(&other, "GET", &[], Some(&captured))
                .unwrap()
                .is_err()
        );
        let missing = CapturedBlobUrl {
            url,
            entry: None,
            fetch_authorized: false,
        };
        assert!(missing.response(&missing.url, &[]).is_none());
    }
}
