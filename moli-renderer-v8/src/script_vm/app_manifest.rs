use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, Request, RequestCredentialsMode, RequestMode,
    RequestRedirectMode, RequestResourceType,
};

use super::ScriptVm;
use crate::{
    app_manifest::{
        RendererAppManifestLinkIdentity, RendererAppManifestLoadPreparation,
        RendererAppManifestLoadPublication, RendererAppManifestNetworkObservation,
        RendererAppManifestQueryResult, RendererPreparedAppManifestLoad,
        complete_default_app_manifest,
    },
    document_runtime::DocumentSubresourceCspKind,
    dom::native::{DomHost, DomMutationEffects},
    native_bridge::JsContextHost,
    network::{
        RendererPreparedNetworkResourceLoad,
        loads::{ResourceLoadDisposition, ResourceLoadKind},
    },
};

pub(super) struct ScriptVmAppManifestCache {
    link_identity: RendererAppManifestLinkIdentity,
    result: RendererAppManifestQueryResult,
}

impl ScriptVm {
    pub(crate) fn prepare_app_manifest_load(&mut self) -> RendererAppManifestLoadPreparation {
        let document_url = self.document_runtime.document_url().clone();
        let Some((link_identity, manifest_url, use_credentials)) = self.app_manifest_link() else {
            self.app_manifest_cache = None;
            return complete_default_app_manifest(&document_url, None);
        };
        if let Some(cache) = self.app_manifest_cache.as_ref()
            && cache.link_identity == link_identity
        {
            return RendererAppManifestLoadPreparation::Complete(Box::new(cache.result.clone()));
        }
        self.app_manifest_cache = None;

        if moli_url::is_opaque_origin(&document_url)
            || !matches!(manifest_url.scheme(), "http" | "https" | "data" | "blob")
        {
            return complete_default_app_manifest(&document_url, Some(&manifest_url));
        }

        let (_report_only_violation, enforced_violations) = self
            .document_runtime
            .document_subresource_csp_check(&manifest_url, DocumentSubresourceCspKind::Manifest)
            .into_violations();
        if !enforced_violations.is_empty() {
            return complete_default_app_manifest(&document_url, Some(&manifest_url));
        }

        let host = self._context_host.borrow();
        let Some(resource_loader) = host.current_main_document_resource_loader() else {
            return complete_default_app_manifest(&document_url, Some(&manifest_url));
        };
        let credentials_mode = if use_credentials {
            RequestCredentialsMode::Include
        } else {
            RequestCredentialsMode::Omit
        };
        let request_headers = host.extra_http_headers().clone();
        let cancel_handle = FetchCancelHandle::new();
        let Some(load) = resource_loader.register_load(
            ResourceLoadKind::Manifest,
            ResourceLoadDisposition::Ordinary,
            Some(cancel_handle.clone()),
        ) else {
            return complete_default_app_manifest(&document_url, Some(&manifest_url));
        };
        let request = Request::new("GET", manifest_url.as_str(), None, request_headers.clone())
            .expect("a resolved app manifest URL should remain valid")
            .with_initiator_url(&document_url)
            .with_request_origin(resource_loader.fetch_context().request_origin())
            .with_request_mode(RequestMode::Cors)
            .with_credentials_mode(credentials_mode)
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_resource_type(RequestResourceType::Manifest)
            .with_browser_request_metadata(BrowserRequestMetadata::Manifest)
            .with_page_network_policy();
        RendererAppManifestLoadPreparation::Ready(Box::new(RendererPreparedAppManifestLoad::new(
            document_url,
            resource_loader.fetch_context().request_origin(),
            manifest_url,
            link_identity,
            RendererPreparedNetworkResourceLoad::new(
                resource_loader.frozen_request_client(),
                request,
            ),
            RendererAppManifestNetworkObservation::new(
                self.root_frame_id.clone(),
                request_headers,
                credentials_mode,
                load,
                cancel_handle,
            ),
        )))
    }

    fn app_manifest_link(&self) -> Option<(RendererAppManifestLinkIdentity, url::Url, bool)> {
        let dom_host = self.document_runtime.dom_host();
        let document = dom_host.document_handle();
        let base_url = dom_host
            .document_base_url()
            .or_else(|| dom_host.final_url().cloned())
            .unwrap_or_else(|| self.document_runtime.document_url().clone());
        let (link_handle, link) = dom_host
            .html_elements_by_local_name_in_document_tree_order(document, "link")
            .into_iter()
            .find_map(|handle| {
                let element = dom_host.node(handle)?.as_element()?;
                element
                    .attribute("rel")?
                    .split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("manifest"))
                    .then_some((handle, element))
            })?;
        let rel = link.attribute("rel")?.to_owned();
        let href = link.attribute("href")?.to_owned();
        let manifest_url = base_url.join(&href).ok()?;
        let host = self._context_host.borrow();
        let link_change_epoch = host.app_manifest_link_change_epoch();
        let document_resource_loader = host
            .current_main_document_resource_loader()
            .map(|loader| loader.identity());
        let link_identity = RendererAppManifestLinkIdentity::new(
            link_handle.encoded(),
            rel,
            href,
            manifest_url.clone(),
            link_change_epoch,
            document_resource_loader,
        );
        let use_credentials = link
            .attribute("crossorigin")
            .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"));
        Some((link_identity, manifest_url, use_credentials))
    }

    pub(crate) fn publish_app_manifest_load(
        &mut self,
        publication: RendererAppManifestLoadPublication,
    ) {
        let (record, successful_result) = publication.into_parts();
        self._context_host
            .borrow_mut()
            .record_subresource_network(record);
        let Some((link_identity, result)) = successful_result else {
            return;
        };
        if self
            .app_manifest_link()
            .is_some_and(|(current_identity, _, _)| current_identity == link_identity)
        {
            self.app_manifest_cache = Some(ScriptVmAppManifestCache {
                link_identity,
                result,
            });
        }
    }
}

impl JsContextHost {
    pub(crate) fn note_app_manifest_link_mutation(
        &mut self,
        dom_host: &DomHost,
        effects: &DomMutationEffects,
    ) {
        if app_manifest_link_changed(dom_host, effects) {
            self.advance_app_manifest_link_change_epoch();
        }
    }
}

fn app_manifest_link_changed(dom_host: &DomHost, effects: &DomMutationEffects) -> bool {
    // Blink invalidates through LinkManifest::Process/OwnerRemoved. In particular,
    // crossorigin does not process the link and therefore preserves the cache.
    effects
        .style()
        .attribute_mutations()
        .iter()
        .any(|mutation| manifest_link_attribute_changed(dom_host, mutation))
        || effects
            .tree()
            .connected_roots()
            .iter()
            .chain(effects.tree().disconnected_roots())
            .copied()
            .any(|root| subtree_contains_manifest_link(dom_host, root))
}

fn manifest_link_attribute_changed(
    dom_host: &DomHost,
    mutation: &crate::dom::native::DomAttributeMutation,
) -> bool {
    let Some(element) = dom_host
        .node(mutation.target())
        .and_then(crate::dom::native::Node::as_element)
        .filter(|element| element.local_name().eq_ignore_ascii_case("link"))
    else {
        return false;
    };
    if !dom_host.is_connected(mutation.target())
        || mutation.namespace().is_some()
        || mutation.old_value() == mutation.new_value()
    {
        return false;
    }
    match mutation.local_name() {
        "rel" => mutation
            .old_value()
            .into_iter()
            .chain(mutation.new_value())
            .any(rel_contains_manifest),
        "href" | "type" | "as" | "sizes" | "media" => {
            element.attribute("rel").is_some_and(rel_contains_manifest)
        }
        _ => false,
    }
}

fn subtree_contains_manifest_link(
    dom_host: &DomHost,
    root: crate::document_runtime::DomHandle,
) -> bool {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        if dom_host
            .node(handle)
            .and_then(crate::dom::native::Node::as_element)
            .filter(|element| element.local_name().eq_ignore_ascii_case("link"))
            .and_then(|element| element.attribute("rel"))
            .is_some_and(rel_contains_manifest)
        {
            return true;
        }
        stack.extend(dom_host.child_handles(handle));
    }
    false
}

fn rel_contains_manifest(value: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("manifest"))
}

#[cfg(test)]
mod tests {
    use super::ScriptVm;
    use crate::{
        dom::native::{DomHost, NativeDom},
        script_vm::{ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness},
    };

    fn new_test_vm() -> StandaloneScriptVmHarness {
        let _js_runtime = crate::JsRuntime::initialize();
        let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
            DomHost::from_dom(NativeDom::new(
                url::Url::parse("https://manifest-cache.test/page").expect("test URL should parse"),
            )),
            page_task_queue.owner_attached_runtime_page_task_sender_for_test(),
            page_task_queue.parser_boundary_sender(),
        )
        .expect("script VM bootstrap should succeed")
        .finish()
        .expect("script VM finish should succeed")
    }

    fn install_manifest_link(vm: &mut ScriptVm) {
        vm.eval(
            r#"
(() => {
  const parent = document.head || document.documentElement || document;
  const link = document.createElement('link');
  link.id = 'app-manifest';
  link.rel = 'manifest';
  link.href = '/app.webmanifest';
  parent.appendChild(link);
})()
"#,
        )
        .expect("manifest link setup should evaluate");
    }

    #[test]
    fn transient_href_change_invalidates_manifest_link_identity() {
        let mut vm = new_test_vm();
        install_manifest_link(&mut vm);
        let (before_identity, before_url, _) = vm
            .app_manifest_link()
            .expect("manifest link should be discoverable");

        vm.eval(
            r#"
(() => {
  const link = document.getElementById('app-manifest');
  const href = link.getAttribute('href');
  link.setAttribute('href', '/other.webmanifest');
  link.setAttribute('href', href);
})()
"#,
        )
        .expect("transient manifest href mutation should evaluate");

        let (after_identity, after_url, _) = vm
            .app_manifest_link()
            .expect("restored manifest link should remain discoverable");
        assert_eq!(after_url, before_url);
        assert_ne!(after_identity, before_identity);
    }

    #[test]
    fn crossorigin_change_preserves_manifest_link_identity() {
        let mut vm = new_test_vm();
        install_manifest_link(&mut vm);
        let (before_identity, _, before_credentials) = vm
            .app_manifest_link()
            .expect("manifest link should be discoverable");

        vm.eval("document.getElementById('app-manifest').crossOrigin = 'use-credentials'")
            .expect("manifest crossorigin mutation should evaluate");

        let (after_identity, _, after_credentials) = vm
            .app_manifest_link()
            .expect("manifest link should remain discoverable");
        assert_eq!(after_identity, before_identity);
        assert!(!before_credentials);
        assert!(after_credentials);
    }
}
