use std::rc::Rc;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
    style_engine::{
        FullStyleWorldSnapshot, IncrementalStyleWorldUpdate, PreparedStyleWorldUpdate,
        StyleSourceId, StyleTreeScopeVersions, StyleViewport, StyleWorldEnvironment,
        StyleWorldUpdatePlan, StyloStyleEnvironment, StyloStylesheetSource,
        link_rel_qualifies_as_stylesheet, stylesheet_owner_type_is_supported,
    },
    stylesheet_blocking::link_rel_includes_token,
};

use super::super::super::super::JsContextHost;
use super::observation::StyleComputationContext;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct StyleObservationKey {
    source_document: Option<DomHandle>,
    tree_scope_versions: StyleTreeScopeVersions,
}

impl StyleObservationKey {
    pub(super) fn for_document(
        runtime: &JsContextHost,
        source_document: Option<DomHandle>,
    ) -> Self {
        Self {
            source_document,
            tree_scope_versions: StyleTreeScopeVersions::current(
                runtime.dom_host(),
                source_document,
            ),
        }
    }

    pub(super) fn source_document(&self) -> Option<DomHandle> {
        self.source_document
    }

    pub(super) fn tree_scope_versions(&self) -> StyleTreeScopeVersions {
        self.tree_scope_versions
    }
}

pub(super) fn stylesheet_source_document_for_handle(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    runtime.dom_host().owner_document_handle(handle)
}

pub(in crate::native_bridge::element) fn style_base_url(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> url::Url {
    let document_handle = if runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
    {
        Some(handle)
    } else {
        runtime
            .dom_host()
            .node(handle)
            .and_then(Node::owner_document)
    };
    document_handle
        .map(|document_handle| {
            if document_handle == runtime.dom_host().document_handle() {
                runtime
                    .dom_host()
                    .document_base_url()
                    .unwrap_or_else(|| runtime.document_url().clone())
            } else {
                runtime
                    .dom_host()
                    .node(document_handle)
                    .and_then(Node::as_document)
                    .map(|document| document.base_url().clone())
                    .unwrap_or_else(|| runtime.document_url().clone())
            }
        })
        .unwrap_or_else(|| runtime.document_url().clone())
}

pub(super) fn prepare_style_world_update(
    runtime: &JsContextHost,
    key: &StyleObservationKey,
    context: StyleComputationContext,
    media: StyloStyleEnvironment,
    plan: &StyleWorldUpdatePlan,
) -> Rc<PreparedStyleWorldUpdate> {
    let quirks_mode = quirks_mode(runtime, key.source_document());
    let environment = StyleWorldEnvironment::new(
        context.viewport(),
        media,
        quirks_mode,
        key.tree_scope_versions(),
    );
    match plan {
        StyleWorldUpdatePlan::Full => {
            let inputs = complete_style_world_snapshot(runtime, key, context, media);
            #[cfg(test)]
            runtime.note_style_world_full_snapshot_for_test();
            Rc::new(PreparedStyleWorldUpdate::full(environment, inputs))
        }
        StyleWorldUpdatePlan::Incremental(plan) => {
            #[cfg(test)]
            runtime.note_style_world_update_materialization_for_test();
            let document_stylesheet_sources = plan
                .refreshes_document_stylesheets()
                .then(|| document_stylesheet_sources(runtime, key.source_document(), context));
            let shadow_stylesheet_sources = plan
                .shadow_stylesheet_roots()
                .iter()
                .copied()
                .filter(|root| {
                    runtime.dom_host().owner_document_handle(*root) == key.source_document()
                        && runtime
                            .dom_host()
                            .shadow_root_host(*root)
                            .is_some_and(|host| runtime.dom_host().is_connected(host))
                })
                .map(|root| (root, shadow_stylesheet_sources(runtime, root, context)))
                .collect();
            let script_custom_property_registrations =
                plan.refreshes_custom_property_registrations().then(|| {
                    key.source_document()
                        .map(|document| {
                            runtime.script_css_custom_property_registration_records(document)
                        })
                        .unwrap_or_default()
                });
            Rc::new(PreparedStyleWorldUpdate::incremental(
                environment,
                IncrementalStyleWorldUpdate::new(
                    document_stylesheet_sources,
                    shadow_stylesheet_sources,
                    plan.connected_shadow_roots().map(|roots| roots.to_vec()),
                    script_custom_property_registrations,
                ),
            ))
        }
    }
}

pub(super) fn stylesheet_query_fallback(
    runtime: &JsContextHost,
    key: &StyleObservationKey,
    context: StyleComputationContext,
    environment: StyloStyleEnvironment,
) -> Rc<FullStyleWorldSnapshot> {
    complete_style_world_snapshot(runtime, key, context, environment)
}

pub(super) fn stylo_style_environment(
    runtime: &JsContextHost,
    document: Option<DomHandle>,
) -> StyloStyleEnvironment {
    #[cfg(test)]
    runtime.note_style_observation_environment_resolution_for_test();
    StyloStyleEnvironment::from_emulated_media(runtime.emulated_media()).with_page_color_schemes(
        document
            .map(|document| {
                crate::document_color_scheme::document_page_color_schemes(
                    runtime.dom_host(),
                    document,
                )
            })
            .unwrap_or_default(),
    )
}

fn complete_style_world_snapshot(
    runtime: &JsContextHost,
    key: &StyleObservationKey,
    context: StyleComputationContext,
    environment: StyloStyleEnvironment,
) -> Rc<FullStyleWorldSnapshot> {
    #[cfg(test)]
    runtime.note_style_world_update_materialization_for_test();
    let source_document = key.source_document();
    let mut inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: document_stylesheet_sources(runtime, source_document, context),
        shadow_stylesheet_sources: Vec::new(),
        script_custom_property_registrations: source_document
            .map(|document| runtime.script_css_custom_property_registration_records(document))
            .unwrap_or_default(),
        environment,
        quirks_mode: quirks_mode(runtime, source_document),
    };
    for root in source_document
        .map(|document| connected_shadow_roots(runtime.dom_host(), document))
        .unwrap_or_default()
    {
        inputs
            .shadow_stylesheet_sources
            .push((root, shadow_stylesheet_sources(runtime, root, context)));
    }
    Rc::new(inputs)
}

fn quirks_mode(runtime: &JsContextHost, document: Option<DomHandle>) -> style::context::QuirksMode {
    document
        .and_then(|document| runtime.dom_host().node(document))
        .and_then(Node::as_document)
        .map(|document| document.quirks_mode())
        .unwrap_or(style::context::QuirksMode::NoQuirks)
}

#[cfg(test)]
pub(super) fn connected_shadow_roots_for_test(
    host: &DomHost,
    document: DomHandle,
) -> Vec<DomHandle> {
    connected_shadow_roots(host, document)
}

fn connected_shadow_roots(host: &DomHost, document: DomHandle) -> Vec<DomHandle> {
    let mut roots = host
        .snapshot_connected_shadow_roots()
        .into_iter()
        .filter(|root| host.owner_document_handle(*root) == Some(document))
        .collect::<Vec<_>>();
    roots.sort_by_key(|root| root.index());
    roots
}

fn document_stylesheet_sources(
    runtime: &JsContextHost,
    source_document: Option<DomHandle>,
    context: StyleComputationContext,
) -> Vec<StyloStylesheetSource> {
    let mut sources = Vec::new();
    let Some(document) = source_document else {
        return sources;
    };
    #[cfg(test)]
    runtime.note_style_world_document_scope_materialization_for_test();
    for style_handle in
        active_stylesheet_handles(runtime, document, context.read_document.is_some())
    {
        sources.push(stylesheet_source(runtime, style_handle));
    }
    sources.extend(
        runtime
            .adopted_style_sheet_sources_for_document(document)
            .iter()
            .map(|source| {
                let client_id = source
                    .adopted_client_id()
                    .expect("installed adopted stylesheet must have a client id");
                source
                    .clone()
                    .with_source_id(Some(StyleSourceId::document_adopted_style_sheet(
                        document, client_id,
                    )))
            }),
    );
    sources
}

fn shadow_stylesheet_sources(
    runtime: &JsContextHost,
    root: DomHandle,
    context: StyleComputationContext,
) -> Vec<StyloStylesheetSource> {
    #[cfg(test)]
    runtime.note_style_world_shadow_scope_materialization_for_test();
    let mut sources = active_stylesheet_handles(runtime, root, context.read_document.is_some())
        .into_iter()
        .map(|handle| stylesheet_source(runtime, handle))
        .collect::<Vec<_>>();
    sources.extend(
        runtime
            .shadow_root_adopted_style_sheet_sources(root)
            .into_iter()
            .map(|source| {
                let client_id = source
                    .adopted_client_id()
                    .expect("installed adopted stylesheet must have a client id");
                source.with_source_id(Some(StyleSourceId::shadow_root_adopted_style_sheet(
                    root, client_id,
                )))
            }),
    );
    sources
}

pub(super) fn active_stylesheet_handles(
    runtime: &JsContextHost,
    root: DomHandle,
    include_detached: bool,
) -> Vec<DomHandle> {
    let mut handles = runtime
        .dom_host()
        .stylesheet_candidate_handles_for_tree_scope(root)
        .iter()
        .copied()
        .filter(|handle| {
            let Some(element) = runtime.dom_host().node(*handle).and_then(Node::as_element) else {
                return false;
            };
            let style = element.is_inline_style_element()
                && runtime
                    .dom_host()
                    .get_attribute(*handle, "disabled")
                    .is_none();
            let link =
                element.is_html_element("link") && link_stylesheet_is_enabled(runtime, *handle);
            (style || link)
                && (include_detached || stylesheet_is_active_in_scope(runtime, root, *handle))
                && stylesheet_owner_type_is_supported(element)
        })
        .collect::<Vec<_>>();
    let preferred_title = handles
        .iter()
        .filter_map(|handle| {
            runtime
                .dom_host()
                .get_attribute(*handle, "title")
                .filter(|title| !title.is_empty())
                .map(|title| (*handle, title))
        })
        .min_by_key(|(handle, _)| handle.index())
        .map(|(_, title)| title);
    if let Some(preferred_title) = preferred_title {
        handles.retain(|handle| {
            runtime
                .dom_host()
                .get_attribute(*handle, "title")
                .filter(|title| !title.is_empty())
                .is_none_or(|title| title == preferred_title)
        });
    }
    handles
}

pub(super) fn effective_raw_stylesheet_sources(
    runtime: &JsContextHost,
    root: DomHandle,
    include_detached: bool,
    viewport: StyleViewport,
) -> Vec<StyloStylesheetSource> {
    active_stylesheet_handles(runtime, root, include_detached)
        .into_iter()
        .map(|handle| stylesheet_source(runtime, handle))
        .filter(|source| source.media_matches(runtime.emulated_media(), viewport))
        .collect()
}

fn stylesheet_source(runtime: &JsContextHost, handle: DomHandle) -> StyloStylesheetSource {
    let media = runtime
        .dom_host()
        .get_attribute(handle, "media")
        .unwrap_or_default();
    let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
        return StyloStylesheetSource::new(String::new(), style_base_url(runtime, handle))
            .with_owner_media_text(&media);
    };
    if element.is_html_element("link") {
        return runtime
            .linked_stylesheet_source_for_owner(handle)
            .unwrap_or_else(|| {
                StyloStylesheetSource::new(String::new(), style_base_url(runtime, handle))
            })
            .with_source_id(StyleSourceId::linked_style_sheet(
                runtime.dom_host(),
                handle,
            ))
            .with_owner_media_text(&media);
    }
    if element.is_inline_style_element()
        && let Some(source) = runtime.owner_style_sheet_source(handle)
    {
        return source
            .with_source_id(StyleSourceId::owner_style_sheet(runtime.dom_host(), handle))
            .with_owner_media_text(&media);
    }
    StyloStylesheetSource::new(String::new(), style_base_url(runtime, handle))
        .with_owner_media_text(&media)
}

fn link_stylesheet_is_enabled(runtime: &JsContextHost, handle: DomHandle) -> bool {
    if runtime
        .dom_host()
        .get_attribute(handle, "disabled")
        .is_some()
    {
        return false;
    }
    let rel = runtime.dom_host().get_attribute(handle, "rel");
    let Some(rel) = rel.as_deref() else {
        return false;
    };
    let title = runtime.dom_host().get_attribute(handle, "title");
    if !link_rel_qualifies_as_stylesheet(Some(rel), title.as_deref()) {
        return false;
    }
    !link_rel_includes_token(rel, "alternate")
        || runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| element.link_explicitly_enabled())
}

fn stylesheet_is_active_in_scope(
    runtime: &JsContextHost,
    root: DomHandle,
    handle: DomHandle,
) -> bool {
    runtime.dom_host().is_connected(handle)
        || runtime
            .child_browsing_context_host_for_document_handle(root)
            .is_some_and(|frame_handle| runtime.dom_host().is_connected(frame_handle))
        || (runtime.dom_host().is_shadow_root(root)
            && runtime
                .dom_host()
                .shadow_root_host(root)
                .is_some_and(|host| runtime.dom_host().is_connected(host)))
}
