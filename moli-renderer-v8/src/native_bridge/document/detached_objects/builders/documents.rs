use super::*;
use crate::dom::native::DomHost;
use crate::util::context_host_ptr_from_global_bridge;
use moli_webapi_declare::WebApiObject;
use url::Url;

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct DetachedDocumentObjectDeclaration<'scope, 'tag> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,

    #[webapi(to_string_tag)]
    to_string_tag: Option<&'tag str>,

    #[webapi(data_property, readonly)]
    location: v8::Local<'scope, v8::Value>,
}

fn detached_document_bridge_prototype_name(kind: &str) -> &'static str {
    if kind == "html" {
        "__detachedHTMLDocumentPrototype"
    } else if kind == "plain" {
        "__detachedDocumentPrototype"
    } else {
        "__detachedXMLDocumentPrototype"
    }
}

fn detached_document_constructor_name(kind: &str) -> &'static str {
    if kind == "html" {
        "HTMLDocument"
    } else if kind == "plain" {
        "Document"
    } else {
        "XMLDocument"
    }
}

fn new_detached_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    to_string_tag: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = global_constructor_prototype(scope, detached_document_constructor_name(kind))
        .or_else(|| {
        bridge_prototype_object(scope, detached_document_bridge_prototype_name(kind))
    })?;
    DetachedDocumentObjectDeclaration::new(prototype, to_string_tag, v8::null(scope).into())
        .bind(scope)
        .ok()
}

fn create_native_detached_document_handle_with_url(
    scope: &mut v8::PinScope<'_, '_>,
    kind: &str,
    url: Url,
    scripting_enabled: bool,
) -> Option<DomHandle> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let runtime = unsafe { &mut *runtime_ptr };
    let handle = if kind == "html" {
        runtime.create_detached_html_document_with_url_and_scripting(url, scripting_enabled)
    } else {
        runtime.create_detached_xml_document_with_url(url)
    };
    let _ = runtime
        .dom_host_mut()
        .set_document_scripting_enabled_for_handle(handle, scripting_enabled);
    Some(handle)
}

fn detached_document_url(parsed: &DomHost) -> Url {
    parsed
        .dom()
        .final_url()
        .cloned()
        .unwrap_or_else(|| Url::parse("about:blank").expect("static about:blank parses"))
}

fn set_detached_document_url_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
    url: &Url,
) -> Option<()> {
    let url = v8_string(scope, url.as_str())?;
    let _ = state.set(scope, v8str(scope, "url").into(), url.into());
    let _ = state.set(scope, v8str(scope, "documentURI").into(), url.into());
    let _ = state.set(scope, v8str(scope, "baseURI").into(), url.into());
    Some(())
}

fn new_detached_document_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    url: Url,
    scripting_enabled: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let to_string_tag = if kind == "html" {
        Some("HTMLDocument")
    } else if kind == "plain" {
        Some("Document")
    } else {
        Some("XMLDocument")
    };
    let document = new_detached_document_object(scope, kind, to_string_tag)?;
    let state = new_detached_state_object(scope, "document", 9, "#document")?;
    let _ = state.set(
        scope,
        v8str(scope, "documentKind").into(),
        v8_string(scope, kind)?.into(),
    );
    let _ = state.set(
        scope,
        v8str(scope, "readyState").into(),
        v8_string(scope, "complete")?.into(),
    );
    set_detached_document_url_state(scope, state, &url)?;
    define_detached_state(scope, document, state);
    if let Some(handle) =
        create_native_detached_document_handle_with_url(scope, kind, url, scripting_enabled)
    {
        define_detached_native_handle(scope, document, handle);
    }
    install_detached_document_instance_properties(scope, document, kind);
    let _ = ensure_detached_document_implementation(scope, document);
    Some(document)
}

fn import_detached_document_children_from_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    parsed: &DomHost,
) -> Option<()> {
    import_detached_document_children_from_host_with_reaction_policy(scope, document, parsed, false)
}

fn import_detached_document_children_from_host_with_reaction_policy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    parsed: &DomHost,
    append_to_current_reaction_queue: bool,
) -> Option<()> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let document_handle = detached_native_handle(scope, document)?;
    let children = parsed
        .child_handles(parsed.document_handle())
        .collect::<Vec<_>>();
    let lazy_native_import = parsed.dom().len() > 1_000;
    for child in children {
        let imported = unsafe { &mut *runtime_ptr }
            .dom_host_mut()
            .import_foreign_node_with_shadow_roots(document_handle, parsed, child, true)?;
        crate::native_bridge::element::queue_parser_details_toggle_events_in_subtree(
            scope,
            runtime_ptr,
            imported,
        );
        let inserted = if append_to_current_reaction_queue {
            unsafe { &mut *runtime_ptr }
                .insert_detached_native_child_appending_to_current_reaction_queue(
                    scope,
                    runtime_ptr,
                    document_handle,
                    imported,
                    None,
                )
        } else {
            unsafe { &mut *runtime_ptr }.insert_detached_native_child(
                scope,
                runtime_ptr,
                document_handle,
                imported,
                None,
            )
        };
        if !inserted {
            return None;
        }
        if !lazy_native_import {
            let imported_object = detached_native_object_for_handle(scope, runtime_ptr, imported)?;
            adopt_attached_imported_node_as_detached(scope, document, imported_object, document)?;
        }
    }
    unsafe { &mut *runtime_ptr }
        .sync_owner_style_sheet_texts_for_document_tree_scopes(document_handle);
    Some(())
}

pub(crate) fn build_detached_document_object_from_dom_host<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    parsed: DomHost,
) -> Option<v8::Local<'s, v8::Object>> {
    build_detached_document_object_from_dom_host_with_content_type(scope, kind, parsed, None, None)
}

pub(crate) fn build_detached_document_object_from_dom_host_with_content_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    parsed: DomHost,
    content_type: Option<&str>,
    character_set: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let url = detached_document_url(&parsed);
    let quirks_mode = parsed.dom().document()?.quirks_mode();
    let scripting_enabled = parsed.dom().document()?.scripting_enabled();
    let compat_mode = if quirks_mode == selectors::matching::QuirksMode::Quirks {
        "BackCompat"
    } else {
        "CSS1Compat"
    };
    let document = new_detached_document_shell(scope, kind, url, scripting_enabled)?;
    if let Some(state) = detached_state_object(scope, document) {
        let _ = state.set(
            scope,
            v8str(scope, "compatMode").into(),
            v8_string(scope, compat_mode)?.into(),
        );
        if let Some(content_type) = content_type {
            let _ = state.set(
                scope,
                v8str(scope, "contentType").into(),
                v8_string(scope, content_type)?.into(),
            );
        }
        if let Some(character_set) = character_set {
            let _ = state.set(
                scope,
                v8str(scope, "characterSet").into(),
                v8_string(scope, character_set)?.into(),
            );
        }
    }
    if let Some(document_handle) = detached_native_handle(scope, document)
        && let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        let _ = unsafe { &mut *runtime_ptr }
            .dom_host_mut()
            .set_document_quirks_mode_for_handle(document_handle, quirks_mode);
    }
    import_detached_document_children_from_host(scope, document, &parsed)?;
    Some(document)
}

fn insert_detached_document_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    child: v8::Local<'s, v8::Object>,
) -> bool {
    sync_detached_native_insert(scope, parent, child, None)
}

fn create_native_html_shell_element(
    runtime: &mut JsContextHost,
    document_handle: DomHandle,
    local_name: &str,
    registry_association: custom_elements::CustomElementRegistryAssociation,
) -> Option<DomHandle> {
    let handle = runtime.create_element_ns(Some(XHTML_NS), local_name)?;
    runtime.initialize_new_native_node_owner_document(document_handle, handle)?;
    runtime.set_custom_element_registry_association(handle, registry_association);
    Some(handle)
}

fn populate_native_html_document_shell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    title: Option<&str>,
) -> Option<()> {
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let document_handle = detached_native_handle(scope, document)?;
    let registry_association =
        unsafe { &*runtime_ptr }.effective_custom_element_registry_association(document_handle);
    let runtime = unsafe { &mut *runtime_ptr };

    let doctype = runtime.create_document_type("html", "", "");
    runtime.initialize_new_native_node_owner_document(document_handle, doctype)?;
    let html =
        create_native_html_shell_element(runtime, document_handle, "html", registry_association)?;
    let head =
        create_native_html_shell_element(runtime, document_handle, "head", registry_association)?;
    let body =
        create_native_html_shell_element(runtime, document_handle, "body", registry_association)?;
    let title_nodes = if let Some(title) = title {
        let title_element = create_native_html_shell_element(
            runtime,
            document_handle,
            "title",
            registry_association,
        )?;
        let title_text = runtime.create_text_node_for_document(document_handle, title);
        Some((title_element, title_text))
    } else {
        None
    };

    runtime
        .dom_host_mut()
        .append_child_without_mutation_effects(document_handle, doctype)
        .then_some(())?;
    runtime
        .dom_host_mut()
        .append_child_without_mutation_effects(document_handle, html)
        .then_some(())?;
    runtime
        .dom_host_mut()
        .append_child_without_mutation_effects(html, head)
        .then_some(())?;
    if let Some((title_element, title_text)) = title_nodes {
        runtime
            .dom_host_mut()
            .append_child_without_mutation_effects(head, title_element)
            .then_some(())?;
        runtime
            .dom_host_mut()
            .append_child_without_mutation_effects(title_element, title_text)
            .then_some(())?;
    }
    runtime
        .dom_host_mut()
        .append_child_without_mutation_effects(html, body)
        .then_some(())?;
    Some(())
}

pub(in crate::native_bridge::document) fn build_detached_html_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    title: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let document = new_detached_document_shell(
        scope,
        "html",
        Url::parse("about:blank").expect("static about:blank parses"),
        false,
    )?;
    populate_native_html_document_shell(scope, document, title)?;
    Some(document)
}

pub(in crate::native_bridge::document) fn build_detached_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: &str,
    namespace_uri: Option<String>,
    qualified_name: Option<String>,
    doctype: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    if kind == "html" {
        return build_detached_html_document_object(scope, None);
    }
    let document = new_detached_document_shell(
        scope,
        kind,
        Url::parse("about:blank").expect("static about:blank parses"),
        false,
    )?;
    if kind == "xml"
        && let Some(namespace_uri) = namespace_uri.as_deref()
        && let Some(state) = detached_state_object(scope, document)
    {
        let _ = state.set(
            scope,
            v8str(scope, "creationNamespace").into(),
            v8_string(scope, namespace_uri)?.into(),
        );
    }

    if let Some(doctype) = doctype {
        let doctype = if detached_is_node(scope, doctype) {
            doctype
        } else {
            adopt_live_node_as_detached(scope, document, doctype)?
        };
        detached_set_owner_document(scope, doctype, document);
        insert_detached_document_child(scope, document, doctype).then_some(())?;
    }
    if let Some(qualified_name) = qualified_name {
        let root = build_detached_element_object(
            scope,
            document,
            &qualified_name,
            namespace_uri,
            "xml",
            true,
            None,
            None,
        )?;
        insert_detached_document_child(scope, document, root).then_some(())?;
    }
    Some(document)
}
