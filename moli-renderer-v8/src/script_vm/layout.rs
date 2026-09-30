use super::*;

impl ScriptVm {
    pub(crate) fn elements_with_bounded_final_opacity_for_document_snapshot(
        &self,
        elements: impl IntoIterator<Item = DomHandle>,
    ) -> std::collections::HashSet<DomHandle> {
        let host = self._context_host.borrow();
        let document = host.document_handle();
        host.elements_with_bounded_final_opacity(document, elements)
    }

    pub(crate) fn sync_live_document_style_sources(&mut self) {
        let document = self.document_runtime.document_handle();
        self._context_host
            .borrow_mut()
            .install_prepared_style_sheets_for_document(document);
    }

    pub(crate) fn computed_style_property_values_for_document_snapshot(
        &self,
        handles: impl IntoIterator<Item = DomHandle>,
        properties: &[String],
    ) -> Vec<Vec<String>> {
        crate::native_bridge::element::computed_style_property_values_for_document_snapshot(
            &self._context_host.borrow(),
            handles,
            properties,
        )
    }

    pub(crate) fn computed_style_own_visibility_values_for_document_snapshot(
        &self,
        handles: impl IntoIterator<Item = DomHandle>,
    ) -> Vec<bool> {
        crate::native_bridge::element::computed_style_own_visibility_values_for_document_snapshot(
            &self._context_host.borrow(),
            handles,
        )
    }

    pub(crate) fn publish_layout(&mut self) -> anyhow::Result<()> {
        let viewport = {
            let host = self._context_host.borrow();
            if !host.layout_policy().uses_real_layout() {
                return Ok(());
            }
            host.layout_viewport_for_document(host.document_handle())
        };
        self.with_fresh_layout_pass(
            moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Explicit),
            |_| Ok(()),
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn screenshot_layout_snapshot(
        &mut self,
        viewport: moli_layout::PaintViewport,
    ) -> anyhow::Result<Option<moli_layout::PaintSnapshot>> {
        self.paint_layout_snapshot(viewport, moli_layout::LayoutFlushReason::Screenshot)
    }

    #[cfg(test)]
    pub(crate) fn publish_layout_for_test(&mut self) -> anyhow::Result<()> {
        let viewport = {
            let host = self._context_host.borrow();
            host.layout_viewport_for_document(host.document_handle())
        };
        self.screenshot_layout_snapshot(viewport)?
            .ok_or_else(|| anyhow::anyhow!("fixture screenshot requires a document"))?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn refresh_layout_snapshot_for_test(
        &mut self,
        viewport: moli_layout::LayoutViewport,
    ) -> Result<bool, moli_layout::LayoutError> {
        self.with_fresh_layout_pass(
            moli_layout::LayoutPassRequest::new(viewport, moli_layout::LayoutFlushReason::Test),
            |_| Ok(()),
        )
        .map(|result| result.is_some())
    }

    #[cfg(test)]
    pub(crate) fn paint_layout_snapshot(
        &mut self,
        viewport: moli_layout::PaintViewport,
        reason: moli_layout::LayoutFlushReason,
    ) -> anyhow::Result<Option<moli_layout::PaintSnapshot>> {
        self.paint_layout_snapshot_with_capture(
            viewport,
            reason,
            moli_layout::PaintCaptureRequest::viewport(),
        )
    }

    pub(crate) fn paint_layout_snapshot_with_capture(
        &mut self,
        viewport: moli_layout::PaintViewport,
        reason: moli_layout::LayoutFlushReason,
        capture: moli_layout::PaintCaptureRequest,
    ) -> anyhow::Result<Option<moli_layout::PaintSnapshot>> {
        self.with_fresh_layout_pass(
            moli_layout::LayoutPassRequest::with_capture(viewport, reason, capture),
            moli_layout::LayoutPassResult::take_paint_snapshot,
        )
        .map_err(anyhow::Error::new)
    }

    pub(crate) fn visual_state_token(
        &self,
        document: crate::runtime::RendererDocumentLifecycleIdentity,
        viewport: moli_layout::PaintViewport,
        base_background_color: [u8; 4],
    ) -> crate::runtime::RendererVisualStateToken {
        self._context_host
            .borrow()
            .visual_state_token(document, viewport, base_background_color)
    }

    #[cfg(test)]
    pub(crate) fn visual_resource_generation_handle_for_test(
        &self,
    ) -> crate::native_bridge::visual_resource_generation::VisualResourceGeneration {
        self._context_host
            .borrow()
            .visual_resource_generation_handle_for_test()
    }

    pub(super) fn with_fresh_layout_pass<T>(
        &mut self,
        request: moli_layout::LayoutPassRequest,
        consume: impl FnOnce(
            &mut moli_layout::LayoutPassResult<DomHandle>,
        ) -> Result<T, moli_layout::LayoutError>,
    ) -> Result<Option<T>, moli_layout::LayoutError> {
        // Font-source reconciliation is a pre-pass lifecycle step. CSS image
        // URLs come back from the actual box-construction traversal below.
        // Once the guard is entered, layout performs no JS, event-loop,
        // observer, or resource completion work and owns no state beyond this
        // call.
        self.reconcile_document_web_fonts_for_layout();
        let requests_paint = request.requests_paint();
        // Printing consumes a temporary projection. Screenshots, screencast
        // frames and explicit refreshes publish geometry for subsequent reads.
        let publishes_layout = request.reason != moli_layout::LayoutFlushReason::Print;
        let (document, result) = {
            let context_host = self._context_host.borrow();
            let document = context_host.document_handle();
            let result = context_host
                .build_layout_pass_for_document(document, request)
                .and_then(|pass| {
                    let Some(mut pass) = pass else {
                        return Ok(None);
                    };
                    let css_images = if requests_paint {
                        pass.css_image_references().to_vec()
                    } else {
                        Vec::new()
                    };
                    let value = consume(&mut pass)?;
                    if publishes_layout {
                        context_host.publish_layout_pass_for_document(document, pass);
                    }
                    Ok(Some((value, css_images)))
                });
            (document, result)
        };
        let (result, css_images) = match result {
            Ok(Some((value, css_images))) => (Ok(Some(value)), css_images),
            Ok(None) => (Ok(None), Vec::new()),
            Err(error) => (Err(error), Vec::new()),
        };
        self.start_css_images_discovered_by_layout(css_images);
        if publishes_layout
            && matches!(&result, Ok(Some(_)))
            && let Err(error) = self.with_default_context_scope(|scope, runtime_ptr| {
                crate::observer_runtime::queue_intersection_checks(scope, runtime_ptr);
                crate::native_bridge::element::queue_revealed_lazy_image_loads(
                    scope,
                    runtime_ptr,
                    document,
                );
                Ok(())
            })
        {
            // Entering the already-owned default context is infallible for
            // this body-only operation. Keep a failed admission non-fatal to
            // the completed frame; the next refresh retries from its newer
            // sampled geometry.
            tracing::warn!(?error, "failed to queue post-layout observable work");
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn layout_pass_observability_for_test(
        &self,
    ) -> (
        bool,
        u64,
        std::time::Duration,
        Option<moli_layout::LayoutPassMetrics>,
    ) {
        self._context_host
            .borrow()
            .layout_pass_observability_for_test()
    }

    #[cfg(test)]
    pub(crate) fn css_image_resource_is_ready_for_test(&self, resolved_url: &str) -> bool {
        let host = self._context_host.borrow();
        host.ready_css_image_for_layout(host.document_handle(), resolved_url)
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn css_image_resource_observability_for_test(
        &self,
    ) -> (usize, usize, usize, usize, Vec<String>) {
        self._context_host
            .borrow()
            .css_image_resource_observability_for_test()
    }

    #[cfg(test)]
    pub(crate) fn css_image_completion_notify_for_test(
        &self,
    ) -> std::sync::Arc<tokio::sync::Notify> {
        self._context_host
            .borrow()
            .css_image_completion_notify_for_test()
    }

    #[cfg(test)]
    pub(crate) fn layout_snapshot_cache_observability_for_test(
        &self,
    ) -> (
        u64,
        u64,
        u64,
        Option<(DomHandle, moli_layout::LayoutTreeRetentionMetrics)>,
    ) {
        let observability = self
            ._context_host
            .borrow()
            .layout_snapshot_cache_observability_for_test();
        (
            observability.hits,
            observability.misses,
            observability.publishes,
            observability.cached,
        )
    }

    pub(super) fn reconcile_document_web_fonts_for_layout(&mut self) {
        let font_fetch_enabled = self
            .document_runtime
            .current_document_resource_loader()
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Font)
            });
        if !font_fetch_enabled {
            return;
        }
        let root = {
            let host = self._context_host.borrow();
            host.dom_host().document_element_handle()
        };
        let Some(root) = root else {
            return;
        };
        let skip_pristine_document = {
            let host = self._context_host.borrow();
            let document = host.document_handle();
            host.document_web_font_sidecar_is_pristine()
                && !host.document_has_style_state(document)
                && !host.document_has_active_author_stylesheet_sources(document)
        };
        if skip_pristine_document {
            return;
        }
        let Some(resources) = crate::layout_renderer::current_native_stylesheet_resources(
            &self._context_host.borrow(),
            root,
        ) else {
            return;
        };
        tracing::trace!(
            stylesheet_import_dependency_count = resources.imports().len(),
            stylesheet_web_font_dependency_count = resources.web_fonts().len(),
            "observed typed stylesheet resource manifest"
        );
        let generation = resources.generation();
        if self
            ._context_host
            .borrow()
            .document_web_font_resources_are_current(generation)
        {
            return;
        }
        let resources = resources.web_fonts().to_vec();
        self._context_host
            .borrow()
            .retain_document_web_font_slots(resources.iter());
        if resources.is_empty() {
            self._context_host
                .borrow()
                .publish_document_web_font_resource_generation(generation);
            return;
        }
        let resource_count = resources.len();
        let bound = {
            let mut host = self._context_host.borrow_mut();
            let bound = resources
                .into_iter()
                .filter_map(|resource| {
                    host.accept_current_main_stylesheet_subresource_load_delay()
                        .map(|binding| (binding, resource))
                })
                .collect::<Vec<_>>();
            if bound.len() == resource_count {
                host.publish_document_web_font_resource_generation(generation);
            }
            bound
        };
        self.start_stylesheet_subresource_fetches(bound);
    }

    pub(super) fn start_css_images_discovered_by_layout(
        &mut self,
        references: Vec<moli_layout::LayoutCssImageReference<DomHandle>>,
    ) {
        if references.is_empty() {
            return;
        }
        let image_fetch_enabled = self
            .document_runtime
            .current_document_resource_loader()
            .is_some_and(|loader| {
                loader
                    .request_client()
                    .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Image)
            });
        if !image_fetch_enabled {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        let bound = {
            let mut host = self._context_host.borrow_mut();
            references
                .into_iter()
                .filter_map(|reference| {
                    let document = host.layout_document_for_source(reference.source)?;
                    if !seen.insert((document, reference.resolved_url.clone())) {
                        return None;
                    }
                    if host.stylesheet_css_image_is_current(document, &reference.resolved_url) {
                        return None;
                    }
                    let request_url = url::Url::parse(&reference.resolved_url).ok()?;
                    let binding = host
                        .accept_current_stylesheet_subresource_load_delay_for_document(document)?;
                    Some((
                        binding,
                        crate::css_resource_urls::StylesheetLoadBlockingResource::image(
                            request_url,
                        ),
                    ))
                })
                .collect::<Vec<_>>()
        };
        self.start_stylesheet_subresource_fetches(bound);
    }

    /// Initializes the first layout on demand, then keeps its content extent
    /// until an explicit refresh. Viewport and scroll remain live browser state.
    pub(crate) fn document_metrics_for_current_document(
        &self,
    ) -> Result<moli_layout::LayoutDocumentMetrics, moli_layout::LayoutError> {
        let host = self._context_host.borrow();
        let document = host.document_handle();
        let metrics = crate::native_bridge::element::observable_document_metrics(&host, document)?;
        let viewport = host.layout_viewport_for_document(document);
        let viewport_scroll = host
            .dom_host()
            .dom()
            .document_element_handle_for_document(document)
            .and_then(|root| host.dom_host().node(root))
            .and_then(|node| node.as_element())
            .map(|element| {
                moli_layout::LayoutPoint::new(
                    element.scroll_left() as f32,
                    element.scroll_top() as f32,
                )
            })
            .unwrap_or(moli_layout::LayoutPoint::ZERO);
        Ok(moli_layout::LayoutDocumentMetrics {
            viewport,
            viewport_scroll,
            content_size: metrics.content_size,
        })
    }

    pub(crate) fn observable_geometry_query_for_document(
        &self,
        document: DomHandle,
        query: &moli_layout::LayoutQuery<DomHandle>,
    ) -> Result<moli_layout::LayoutQueryAnswer<DomHandle>, moli_layout::LayoutError> {
        let host = self._context_host.borrow();
        crate::native_bridge::element::observable_geometry_query(&host, document, query)
    }

    pub(crate) fn observable_geometry_batch_for_document(
        &mut self,
        document: DomHandle,
        batch: &moli_layout::LayoutQueryBatch<DomHandle>,
    ) -> Result<moli_layout::LayoutAnswers<DomHandle>, moli_layout::LayoutError> {
        let host = self._context_host.borrow();
        crate::native_bridge::element::observable_geometry_batch(&host, document, batch)
    }

    pub(crate) fn observable_deep_hit_test_for_current_document(
        &mut self,
        point: moli_layout::LayoutPoint,
        ignore_pointer_events_none: bool,
    ) -> Result<Option<DomHandle>, moli_layout::LayoutError> {
        let host = self._context_host.borrow();
        let document = host.document_handle();
        crate::native_bridge::element::observable_deep_hit_test(
            &host,
            document,
            point,
            ignore_pointer_events_none,
        )
    }

    pub(super) fn complete_document_web_font(
        &mut self,
        terminal: crate::css_resource_urls::CompletedStylesheetWebFont,
    ) {
        match self
            ._context_host
            .borrow()
            .complete_document_web_font(terminal)
        {
            web_fonts::DocumentWebFontCompletion::Registered(outcome) => tracing::debug!(
                ?outcome,
                "registered current document web font for the next fresh layout refresh"
            ),
            web_fonts::DocumentWebFontCompletion::Invalid(error) => tracing::warn!(
                %error,
                "discarded invalid current document web font response"
            ),
            web_fonts::DocumentWebFontCompletion::NetworkFailed => {
                tracing::debug!("current document web font request reached a failed terminal")
            }
            web_fonts::DocumentWebFontCompletion::Stale => {
                tracing::debug!("discarded superseded document web font response")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn document_web_font_counts_for_test(&mut self) -> (usize, usize, usize) {
        self._context_host
            .borrow()
            .document_web_font_counts_for_test()
    }

    #[cfg(test)]
    pub(crate) fn normalized_layout_box_tree_for_test(&self) -> anyhow::Result<Option<String>> {
        let context_host = self._context_host.borrow();
        let Some(root) = context_host.dom_host().document_element_handle() else {
            return Ok(None);
        };
        crate::layout_renderer::build_normalized_native_box_tree_for_test(&context_host, root)
            .map(|tree| Some(tree.to_string()))
            .map_err(anyhow::Error::new)
    }

    #[cfg(test)]
    pub(crate) fn build_layout_pass_for_subtree_for_test(
        &self,
        root: DomHandle,
        request: moli_layout::LayoutPassRequest,
    ) -> Result<moli_layout::LayoutPassResult<DomHandle>, moli_layout::LayoutError> {
        let context_host = self._context_host.borrow();
        let mut services = moli_layout::DocumentLayoutServices::new();
        let mut embedded_document_services = HashMap::new();
        crate::layout_renderer::build_native_layout_pass(
            &context_host,
            root,
            &mut services,
            &mut embedded_document_services,
            request,
        )
    }

    pub(crate) fn computed_style_properties_for_inspector_handle(
        &self,
        handle: DomHandle,
    ) -> Option<Vec<(String, String)>> {
        crate::native_bridge::element::computed_style_properties_for_inspector_handle(
            &self._context_host.borrow(),
            handle,
        )
    }

    pub(crate) fn marker_pseudo_element_is_generated_for_document_snapshot(
        &self,
        handle: DomHandle,
    ) -> bool {
        crate::native_bridge::element::marker_pseudo_element_is_generated_for_document_snapshot(
            &self._context_host.borrow(),
            handle,
        )
        .unwrap_or(false)
    }

    pub(crate) fn owner_style_sheet_text(&self, owner: DomHandle) -> Option<String> {
        self._context_host.borrow().owner_style_sheet_text(owner)
    }

    pub(crate) fn linked_stylesheet_source_for_owner(
        &self,
        owner: DomHandle,
    ) -> Option<crate::style_engine::StyloStylesheetSource> {
        self._context_host
            .borrow()
            .linked_stylesheet_source_for_owner(owner)
    }

    pub(crate) fn stylesheet_owner_is_csp_blocked(&self, owner: DomHandle) -> bool {
        self._context_host
            .borrow()
            .stylesheet_owner_is_csp_blocked(owner)
    }

    pub(crate) fn drain_parser_defined_autonomous_custom_elements(&mut self) -> Vec<String> {
        self._context_host
            .borrow_mut()
            .drain_parser_defined_autonomous_custom_elements()
    }

    #[cfg(test)]
    pub(crate) fn document_handle_for_test(&self) -> DomHandle {
        self.document_runtime.document_handle()
    }

    #[cfg(test)]
    pub(crate) fn pending_style_invalidation_work_item_count_for_current_document_for_test(
        &self,
    ) -> usize {
        let document = self.document_runtime.document_handle();
        self._context_host
            .borrow()
            .pending_style_invalidation_work_item_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn element_handle_by_id_for_test(&self, id: &str) -> Option<DomHandle> {
        self._context_host
            .borrow()
            .dom_host()
            .element_handle_by_id(id)
    }

    #[cfg(test)]
    pub(crate) fn inline_style_base_url_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self._context_host
            .borrow()
            .inline_style_base_url_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_entry_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self._context_host
            .borrow()
            .computed_style_cache_entry_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn active_document_style_world_count_for_test(&self) -> usize {
        self._context_host
            .borrow()
            .active_document_style_world_count_for_test()
    }

    #[cfg(test)]
    pub(crate) fn document_style_world_is_active_for_test(&self, document: DomHandle) -> bool {
        self._context_host
            .borrow()
            .document_style_world_is_active_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .computed_style_cache_generation_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_rebuild_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .retained_style_system_rebuild_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_update_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .retained_style_system_update_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_stylist_identity_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self._context_host
            .borrow()
            .retained_stylist_identity_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_shadow_scope_flush_count_for_document_for_test(
        &self,
        document: DomHandle,
        root: DomHandle,
    ) -> Option<u64> {
        self._context_host
            .borrow()
            .retained_shadow_scope_flush_count_for_document_for_test(document, root)
    }

    #[cfg(test)]
    pub(crate) fn shared_worker_client_count_for_test(&self) -> usize {
        self._context_host
            .borrow()
            .shared_worker_client_count_for_test()
    }

    pub(crate) fn service_worker_client_id(
        &self,
    ) -> crate::service_worker_runtime::ServiceWorkerClientId {
        self._context_host.borrow().service_worker_client_id()
    }

    #[cfg(test)]
    pub(crate) fn custom_element_registry_association_count_for_test(&self) -> usize {
        self._context_host
            .borrow()
            .custom_element_registry_association_count_for_test()
    }

    #[cfg(test)]
    pub(crate) fn storage_bucket_keys_for_test(&self, storage_key: &str) -> Vec<String> {
        self.storage_bucket_store.lock().keys(storage_key)
    }

    #[cfg(test)]
    pub(crate) fn context_host_weak_for_test(&self) -> std::rc::Weak<RefCell<JsContextHost>> {
        Rc::downgrade(&self._context_host)
    }

    pub(crate) fn scroll_live_node_handle_into_view_if_needed(
        &mut self,
        handle: DomHandle,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Result<RendererScrollIntoViewResult> {
        if !self._context_host.borrow().dom_host().is_connected(handle) {
            return Ok(RendererScrollIntoViewResult::NodeDetached);
        }
        {
            let host = self._context_host.borrow();
            if host.layout_policy().uses_real_layout() {
                host.ensure_initial_layout()?;
                let document = host
                    .layout_document_for_source(handle)
                    .ok_or(moli_layout::LayoutError::NoLayoutSnapshot)?;
                host.with_latest_layout_tree_for_document(document, |_| ())
                    .ok_or(moli_layout::LayoutError::NoLayoutSnapshot)?;
            }
        }
        self.with_default_context_scope(|scope, runtime_ptr| {
            Ok(
                match crate::native_bridge::element::scroll_node_into_view_if_needed(
                    scope,
                    runtime_ptr,
                    handle,
                    rect,
                )? {
                    Some(_) => RendererScrollIntoViewResult::ScrolledOrAlreadyVisible,
                    None => RendererScrollIntoViewResult::NodeDoesNotHaveLayoutObject,
                },
            )
        })
    }

    pub(crate) fn client_rect_for_live_node_handle(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<ClientRect>, moli_layout::LayoutError> {
        let Some(document) = self
            ._context_host
            .borrow()
            .dom_host()
            .owner_document_handle(handle)
        else {
            return Ok(None);
        };
        let answer = self.observable_geometry_query_for_document(
            document,
            &moli_layout::LayoutQuery::ClientRects { source: handle },
        )?;
        let moli_layout::LayoutQueryAnswer::ClientRects(mut quads) = answer else {
            return Err(moli_layout::LayoutError::source_contract(
                "renderer client rect",
                "provider returned a mismatched client-rects answer",
            ));
        };
        if quads.is_empty() {
            return Ok(None);
        }
        self.compose_layout_quads_to_top(document, &mut quads)?;
        let Some(rect) = quads
            .into_iter()
            .map(moli_layout::LayoutQuad::bounding_rect)
            .reduce(moli_layout::LayoutRect::union)
        else {
            return Ok(None);
        };
        Ok(Some(ClientRect {
            left: f64::from(rect.x),
            top: f64::from(rect.y),
            right: f64::from(rect.right()),
            bottom: f64::from(rect.bottom()),
            width: f64::from(rect.width),
            height: f64::from(rect.height),
        }))
    }

    pub(crate) fn compose_layout_quads_to_top(
        &mut self,
        mut document: DomHandle,
        quads: &mut [moli_layout::LayoutQuad],
    ) -> Result<(), moli_layout::LayoutError> {
        for _ in 0..16 {
            let frame_context = {
                let host = self._context_host.borrow();
                if document == host.document_handle() {
                    return Ok(());
                }
                let Some(frame) = host.child_browsing_context_host_for_document_handle(document)
                else {
                    return Err(moli_layout::LayoutError::source_contract(
                        "frame geometry composition",
                        "child document has no live frame owner",
                    ));
                };
                let Some(parent_document) = host.dom_host().owner_document_handle(frame) else {
                    return Err(moli_layout::LayoutError::source_contract(
                        "frame geometry composition",
                        "frame owner has no parent document",
                    ));
                };
                (
                    frame,
                    parent_document,
                    host.layout_viewport_for_document(document),
                )
            };
            let (frame, parent_document, child_viewport) = frame_context;
            let answer = self.observable_geometry_query_for_document(
                parent_document,
                &moli_layout::LayoutQuery::BoxModel { source: frame },
            )?;
            let moli_layout::LayoutQueryAnswer::BoxModel(Some(frame_model)) = answer else {
                return Err(moli_layout::LayoutError::source_contract(
                    "frame geometry composition",
                    "frame owner has no content-box geometry",
                ));
            };
            for quad in quads.iter_mut() {
                for point in &mut quad.points {
                    *point = map_child_viewport_point_to_parent_content(
                        *point,
                        child_viewport,
                        frame_model.content,
                    );
                }
            }
            document = parent_document;
        }
        Err(moli_layout::LayoutError::source_contract(
            "frame geometry composition",
            "child-frame nesting exceeds the supported depth",
        ))
    }
}
