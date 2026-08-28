use super::JsContextHost;
use crate::{
    context_bootstrap::{
        SharedWebStorageStore, deep_clone_shared_web_storage_store,
        web_storage_area_key_for_storage_key,
    },
    document_runtime::{DocumentPolicyContainer, DomHandle},
    native_bridge::element::SpecialBrowsingContextTarget,
    network::context::DocumentResourceLoader,
    runtime::{
        PageVmEnvConfig, RendererPendingAuxiliaryPage, RendererPendingPopupActivation,
        RendererRelatedInitialEmptyPageRealmInit, RendererStagedAuxiliaryWindowProxy,
    },
    util::{get_private_value, set_private_value},
};
use moli_crypto::sha256_hex;
use moli_storage_key::{MoliStorageKey, StoragePartitionRelation, site_for_url};
use moli_url::origin_ascii_serialization;
use percent_encoding::percent_decode_str;
use url::Url;
const RENDERER_OWNED_AUXILIARY_POPUP_ID_SLOT: &str = "__lmRendererOwnedAuxiliaryPopupId";
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuxiliaryPageStorageScope {
    origin: String,
    area_key: String,
    storage_key: MoliStorageKey,
}

impl AuxiliaryPageStorageScope {
    fn new(origin: String, area_key: String, storage_key: MoliStorageKey) -> Self {
        Self {
            origin,
            area_key,
            storage_key,
        }
    }

    fn from_web_storage_scope(scope: super::child_frames::WebStorageScope) -> Self {
        let (origin, area_key, storage_key) = scope.into_parts();
        Self {
            origin,
            area_key,
            storage_key,
        }
    }

    pub(super) fn origin(&self) -> &str {
        &self.origin
    }

    pub(super) fn area_key(&self) -> &str {
        &self.area_key
    }

    pub(super) fn storage_key(&self) -> &MoliStorageKey {
        &self.storage_key
    }
}

pub(crate) struct OpenedAuxiliaryBrowsingContext<'scope> {
    pub(crate) window: v8::Local<'scope, v8::Object>,
    pub(crate) popup_id: u64,
    pub(crate) pending_auxiliary_page: RendererPendingAuxiliaryPage,
    pub(crate) captured_session_storage_store: SharedWebStorageStore,
    pub(crate) captured_initial_empty_document_storage_key: MoliStorageKey,
}

impl JsContextHost {
    /// Creates the synchronous WindowProxy and the authoritative initial
    /// Document directly inside a reserved related auxiliary Page.
    ///
    /// Production callers must use this entry point after existing-target
    /// selection and new-context admission. It has no lightweight record or
    /// parser/loader fallback: failure leaves the caller to publish a
    /// no-local-proxy activation for the browser-owned target transaction.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_renderer_owned_related_auxiliary_page<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        opener: v8::Local<'s, v8::Object>,
        opener_child_handle: Option<DomHandle>,
        target_name: &str,
        href: &str,
        creator_base_url: Url,
        creator_policy_container: DocumentPolicyContainer,
        creator_resource_authority: DocumentResourceLoader,
    ) -> Option<OpenedAuxiliaryBrowsingContext<'s>> {
        let pending_auxiliary_page = self.reserve_pending_auxiliary_page(true)?;
        let accepted_sandbox_policy = creator_policy_container.sandbox;
        let inherits_creator_security_token = !accepted_sandbox_policy.forces_opaque_origin;
        let requested_url = Url::parse(href).ok()?;
        self.create_renderer_owned_initial_empty_window(
            scope,
            host_ptr,
            opener,
            opener_child_handle,
            target_name,
            requested_url,
            pending_auxiliary_page,
            creator_base_url,
            creator_policy_container,
            creator_resource_authority,
            inherits_creator_security_token,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_renderer_owned_initial_empty_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        opener: v8::Local<'s, v8::Object>,
        opener_child_handle: Option<DomHandle>,
        target_name: &str,
        requested_url: Url,
        pending_auxiliary_page: RendererPendingAuxiliaryPage,
        creator_base_url: Url,
        creator_policy_container: DocumentPolicyContainer,
        creator_resource_authority: DocumentResourceLoader,
        inherits_creator_security_token: bool,
    ) -> Option<OpenedAuxiliaryBrowsingContext<'s>> {
        let initial_url = if moli_url::is_about_blank(&requested_url) {
            requested_url
        } else {
            // A non-empty destination is not the initial Document. It remains
            // on the immutable popup activation until target admission, then
            // the auxiliary Page owner starts its one replacement navigation.
            about_blank_url()
        };
        let initial_base_url = auxiliary_page_initial_base_url(&initial_url, creator_base_url);
        let storage_scope = self.auxiliary_page_storage_scope_for_initiated_navigation(
            opener_child_handle,
            &initial_url,
            creator_policy_container.sandbox.forces_opaque_origin,
        );
        let session_storage_store =
            self.clone_auxiliary_page_session_storage_store(opener_child_handle, &storage_scope);
        let (window, facade_context) = self
            .bridge
            .bindings
            .instantiate_same_origin_window_proxy_shell(scope, host_ptr);
        let inherited_security_token = inherits_creator_security_token.then(|| {
            let facade_context = v8::Local::new(scope, &facade_context);
            v8::Global::new(scope, facade_context.get_security_token(scope))
        });
        let staged_window_proxy = RendererStagedAuxiliaryWindowProxy::new(
            v8::Global::new(scope, window),
            facade_context,
            inherited_security_token,
        );
        let initial_document_scripting_enabled =
            !self.script_execution_disabled() && creator_policy_container.sandbox.allows_scripts;
        let mut dom_host = crate::dom::native::DomHost::from_dom(
            crate::parser::HtmlParser::with_scripting_enabled(initial_document_scripting_enabled)
                .parse(
                    initial_url.clone(),
                    "<!doctype html><html><head></head><body></body></html>".to_owned(),
                ),
        );
        let document_handle = dom_host.document_handle();
        let _ = dom_host.set_document_fallback_base_url_for_handle(
            document_handle,
            Some(initial_base_url.clone()),
        );
        let mut env = PageVmEnvConfig::related_initial_empty(
            crate::RendererWebStorageHandles::new(
                self.web_storage_store(),
                session_storage_store.clone(),
            ),
            storage_scope.storage_key().clone(),
            &creator_policy_container,
            self.indexed_db_manager(),
            self.storage_bucket_store(),
        );
        if let Some(policy) = self.current_top_level_cross_origin_opener_policy() {
            env.cross_origin_opener_policy =
                crate::cross_origin_isolation::CrossOriginOpenerPolicyCommit::Inherited(policy);
        }
        let popup_id = self.next_auxiliary_browsing_context_id;
        self.next_auxiliary_browsing_context_id = self
            .next_auxiliary_browsing_context_id
            .wrapping_add(1)
            .max(1);
        let init = RendererRelatedInitialEmptyPageRealmInit {
            dom_host,
            loader: creator_resource_authority
                .request_client()
                .fork_with_isolated_page_network_policy(),
            env,
            inherited_origin: storage_scope.origin().to_owned(),
            policy_container: creator_policy_container,
            auxiliary_popup_id: popup_id,
            staged_window_proxy,
            opener: Some(v8::Global::new(scope, opener)),
            window_name: trackable_auxiliary_window_name(target_name).unwrap_or_default(),
        };
        if let Err(error) =
            self.stage_related_initial_empty_page_in_scope(scope, pending_auxiliary_page, init)
        {
            tracing::debug!(
                error = %error,
                "failed to stage renderer-owned initial auxiliary realm; rejecting synchronous proxy path"
            );
            return None;
        }

        Some(OpenedAuxiliaryBrowsingContext {
            window,
            popup_id,
            pending_auxiliary_page,
            captured_session_storage_store: session_storage_store,
            captured_initial_empty_document_storage_key: storage_scope.storage_key().clone(),
        })
    }
}

impl JsContextHost {
    fn clone_auxiliary_page_session_storage_store(
        &mut self,
        opener_child_handle: Option<DomHandle>,
        target_scope: &AuxiliaryPageStorageScope,
    ) -> SharedWebStorageStore {
        let source_store = self.session_storage_store.clone();
        let target_store = deep_clone_shared_web_storage_store(&source_store);
        let Some(source_scope) = self.auxiliary_page_opener_storage_scope(opener_child_handle)
        else {
            return target_store;
        };
        if source_scope.origin() != target_scope.origin()
            || source_scope.area_key() == target_scope.area_key()
        {
            return target_store;
        }
        let entries = {
            let mut source = source_store.lock();
            source
                .sorted_keys_utf16(source_scope.area_key())
                .into_iter()
                .filter_map(|key| {
                    source
                        .get_item_utf16(source_scope.area_key(), &key)
                        .map(|value| (key, value))
                })
                .collect::<Vec<_>>()
        };
        {
            let mut target = target_store.lock();
            for (key, value) in entries {
                let _ = target.set_item_utf16(target_scope.area_key(), &key, &value);
            }
        }
        target_store
    }

    fn auxiliary_page_storage_scope_for_initiated_navigation(
        &mut self,
        opener_child_handle: Option<DomHandle>,
        target_url: &Url,
        sandbox_forces_opaque_origin: bool,
    ) -> AuxiliaryPageStorageScope {
        if sandbox_forces_opaque_origin {
            return self.auxiliary_page_opaque_storage_scope(target_url);
        }
        if moli_url::is_about_blank(target_url)
            && let Some(opener_scope) =
                self.auxiliary_page_opener_storage_scope(opener_child_handle)
        {
            if opener_scope.origin() == "null" {
                return opener_scope;
            }
            if opener_child_handle.is_none() {
                return opener_scope;
            }
            let storage_key = web_storage_key_for_child_about_blank_auxiliary_page(&opener_scope);
            let area_key = web_storage_area_key_for_storage_key(&storage_key);
            return AuxiliaryPageStorageScope::new(
                opener_scope.origin().to_owned(),
                area_key,
                storage_key,
            );
        }
        AuxiliaryPageStorageScope::from_web_storage_scope(
            self.web_storage_scope_for_url_as_first_party(target_url),
        )
    }

    fn auxiliary_page_opaque_storage_scope(&mut self, url: &Url) -> AuxiliaryPageStorageScope {
        let top_level_site = site_for_url(self.document_url());
        let relation = StoragePartitionRelation::from_sites(&site_for_url(url), &top_level_site);
        let storage_key = MoliStorageKey::new(
            "null".to_owned(),
            top_level_site,
            Some(self.browser_context_runtime.next_opaque_origin_nonce()),
            relation,
        );
        AuxiliaryPageStorageScope::new(
            "null".to_owned(),
            web_storage_area_key_for_storage_key(&storage_key),
            storage_key,
        )
    }

    fn auxiliary_page_opener_storage_scope(
        &mut self,
        opener_child_handle: Option<DomHandle>,
    ) -> Option<AuxiliaryPageStorageScope> {
        if let Some(handle) = opener_child_handle {
            let top_origin = origin_ascii_serialization(self.document_url());
            return self
                .child_browsing_context_web_storage_scope(handle, &top_origin)
                .map(AuxiliaryPageStorageScope::from_web_storage_scope);
        }
        Some(AuxiliaryPageStorageScope::from_web_storage_scope(
            self.top_web_storage_scope(),
        ))
    }
}

impl JsContextHost {
    pub(crate) fn record_pending_popup_activation(
        &mut self,
        activation: RendererPendingPopupActivation,
        window_open_event: Option<crate::RendererPendingWindowOpenEvent>,
    ) {
        if let (Some(event), Some(creation_user_gesture)) = (
            window_open_event.as_ref(),
            activation.creation_had_transient_user_activation(),
        ) {
            assert_eq!(
                event.user_gesture, creation_user_gesture,
                "Page.windowOpen must observe the frozen pre-consumption activation transaction"
            );
        }
        let mut items = Vec::with_capacity(1 + usize::from(window_open_event.is_some()));
        if let Some(event) = window_open_event {
            items.push(crate::runtime::RendererOutputItem::Observation(
                crate::runtime::RendererProtocolObservation::WindowOpen(event),
            ));
        }
        items.push(crate::runtime::RendererOutputItem::OwnerAction(
            crate::runtime::RendererOwnerAction::Popup(activation.clone()),
        ));
        let published = self.append_live_turn_items(items);
        if published {
            return;
        }
        #[cfg(test)]
        self.pending_popup_activations.push(activation);
        #[cfg(not(test))]
        {
            let _ = activation;
            panic!("a production popup must have a concrete renderer output sink");
        }
    }

    #[cfg(test)]
    pub(crate) fn take_pending_popup_activations(&mut self) -> Vec<RendererPendingPopupActivation> {
        std::mem::take(&mut self.pending_popup_activations)
    }

    pub(crate) fn pending_popup_activation_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_popup_activations.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }
}

pub(crate) fn set_renderer_owned_auxiliary_popup_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    popup_id: u64,
) {
    let value = v8::BigInt::new_from_u64(scope, popup_id);
    set_private_value(
        scope,
        window,
        RENDERER_OWNED_AUXILIARY_POPUP_ID_SLOT,
        value.into(),
    );
}

pub(crate) fn renderer_owned_auxiliary_popup_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, window, RENDERER_OWNED_AUXILIARY_POPUP_ID_SLOT)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (popup_id, lossless) = value.u64_value();
    (lossless && popup_id != 0).then_some(popup_id)
}

fn trackable_auxiliary_window_name(target_name: &str) -> Option<String> {
    if target_name.is_empty() || SpecialBrowsingContextTarget::parse(target_name).is_some() {
        return None;
    }
    Some(target_name.to_owned())
}

fn web_storage_key_for_child_about_blank_auxiliary_page(
    opener_scope: &AuxiliaryPageStorageScope,
) -> MoliStorageKey {
    let popup_top_level_site = format!(
        "popup:{}",
        sha256_hex(
            opener_scope
                .storage_key()
                .serialized_storage_key()
                .as_bytes()
        )
    );
    MoliStorageKey::new(
        opener_scope.origin().to_owned(),
        popup_top_level_site,
        None,
        StoragePartitionRelation::ThirdParty,
    )
}

fn auxiliary_page_initial_base_url(target_url: &Url, creator_base_url: Url) -> Url {
    if moli_url::is_about_blank(target_url) {
        creator_base_url
    } else {
        target_url.clone()
    }
}

fn about_blank_url() -> Url {
    Url::parse("about:blank").expect("about:blank should parse")
}

pub(crate) fn javascript_url_csp_source(url: &Url) -> String {
    format!("javascript:{}", javascript_url_source(url))
}

pub(crate) fn javascript_url_source(url: &Url) -> String {
    let source = url
        .as_str()
        .strip_prefix("javascript:")
        .unwrap_or_else(|| url.path());
    percent_decode_str(source).decode_utf8_lossy().into_owned()
}
