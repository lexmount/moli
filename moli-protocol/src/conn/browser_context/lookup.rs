use super::*;

impl CdpConnection {
    pub fn browser_contexts(&self) -> impl Iterator<Item = &BrowserContext> {
        self.browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
    }

    pub fn browser_context_by_id(&self, browser_context_id: &str) -> Option<&BrowserContext> {
        self.browser_context
            .as_ref()
            .filter(|bc| bc.id == browser_context_id)
            .or_else(|| {
                self.inactive_browser_contexts
                    .iter()
                    .find(|bc| bc.id == browser_context_id)
            })
    }

    pub fn browser_context_by_id_mut(
        &mut self,
        browser_context_id: &str,
    ) -> Option<&mut BrowserContext> {
        if self
            .browser_context
            .as_ref()
            .is_some_and(|bc| bc.id == browser_context_id)
        {
            return self.browser_context.as_mut();
        }
        self.inactive_browser_contexts
            .iter_mut()
            .find(|bc| bc.id == browser_context_id)
    }

    pub(crate) fn browser_context_for_command_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut BrowserContext, (i32, &'static str)> {
        let route = match session_id {
            Some(session_id) => self
                .session_route(Some(session_id))
                .ok_or((-32001, "Unknown sessionId"))?,
            None => CdpSessionRoute::Browser,
        };
        match route {
            CdpSessionRoute::Browser => self
                .browser_context
                .as_mut()
                .ok_or((-31998, "BrowserContextNotLoaded")),
            CdpSessionRoute::BrowserContext { browser_context_id } => self
                .browser_context_by_id_mut(&browser_context_id)
                .ok_or((-31998, "UnknownBrowserContextId")),
            CdpSessionRoute::TabTarget { .. } => Err((-31998, "DirectSessionRouteRequired")),
            CdpSessionRoute::PageTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::SharedWorkerTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id, ..
            } => self
                .browser_context_by_id_mut(&browser_context_id)
                .ok_or((-31998, "UnknownBrowserContextId")),
        }
    }

    pub fn has_browser_context_id(&self, browser_context_id: &str) -> bool {
        self.browser_contexts()
            .any(|bc| bc.id == browser_context_id)
    }

    pub(crate) fn browser_context_id_for_target(&self, target_id: &str) -> Option<&str> {
        self.browser_contexts()
            .find(|bc| bc.page_target(target_id).is_some())
            .map(|bc| bc.id.as_str())
    }

    pub(crate) fn browser_context_id_for_browsing_context(&self, context_id: &str) -> Option<&str> {
        self.page_for_browsing_context(context_id)
            .map(|(context, _)| context.id.as_str())
    }

    pub(crate) fn page_for_browsing_context(
        &self,
        context_id: &str,
    ) -> Option<(&BrowserContext, &crate::conn::PageTargetHost)> {
        self.browser_contexts().find_map(|context| {
            context
                .page_targets
                .iter()
                .find(|page| {
                    page.target_id() == context_id
                        || page.owner_state.frame_index.get(context_id).is_some()
                })
                .map(|page| (context, page))
        })
    }

    pub fn browsing_context_owner(
        &self,
        context_id: &str,
    ) -> Option<crate::frame_index::FrameRecord> {
        let (context, page) = self.page_for_browsing_context(context_id)?;
        if let Some(record) = page.owner_state.frame_index.get(context_id) {
            return Some(record.clone());
        }
        Some(page_frame_record(context, page))
    }

    pub fn frame_index_snapshot(&self) -> crate::frame_index::FrameIndex {
        let mut index = crate::frame_index::FrameIndex::default();
        for context in self.browser_contexts() {
            for page in context.page_targets.iter() {
                index.insert(page_frame_record(context, page));
                for record in page.owner_state.frame_index.iter() {
                    index.insert(record.clone());
                }
            }
        }
        index
    }

}

fn page_frame_record(
    context: &BrowserContext,
    page: &crate::conn::PageTargetHost,
) -> crate::frame_index::FrameRecord {
    let mut record = crate::frame_index::FrameRecord::new(
        page.target_id(),
        None,
        page.target_id(),
        Some(context.id.clone()),
    );
    if let Some(info) = context.devtools_target_info(page.target_id()) {
        record.original_opener = info.original_opener_id().map(str::to_owned);
        record.url = info.url;
    }
    record.loader_id = page
        .runtime_slot
        .committed_document_loader_id()
        .map(str::to_owned);
    record.document = page
        .runtime_slot
        .page_slot()
        .renderer_document_lifecycle_binding()
        .map(
            |binding| moli_core::page::RendererDocumentLifecycleIdentity {
                frame: binding.renderer_frame,
                document: binding.renderer_document,
                epoch: binding.renderer_epoch,
            },
        );
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_ownership_snapshot_preserves_popup_opener_from_target_info() {
        let mut conn = CdpConnection::new();
        let mut context = BrowserContext::new_with_page_for_test("BID-popup", "TID-popup");
        context.remember_target_opener(
            "TID-popup",
            "TID-opener".into(),
            "FRAME-opener".into(),
            false,
        );
        conn.browser_context = Some(context);

        let frames = conn.frame_index_snapshot();
        let popup = frames.get("TID-popup").unwrap();
        assert_eq!(popup.original_opener.as_deref(), Some("FRAME-opener"));
        assert_eq!(popup.parent, None);
        assert_eq!(
            conn.browsing_context_owner("TID-popup").as_ref(),
            Some(popup)
        );
        conn.browser_context
            .as_mut()
            .unwrap()
            .forget_target_opener_references_for_target("TID-opener");
        let after_close = conn.frame_index_snapshot();
        assert_eq!(
            after_close.get("TID-popup").unwrap().original_opener,
            popup.original_opener
        );
    }

    #[test]
    fn browsing_context_owner_lookup_resolves_attached_frames_without_selecting_their_page() {
        let mut conn = CdpConnection::new();
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        active
            .active_page_target_mut()
            .owner_state
            .frame_index
            .insert(crate::frame_index::FrameRecord::new(
                "FRAME-active",
                Some("TID-active".into()),
                "TID-active",
                Some("BID-active".into()),
            ));
        let background = BrowserContext::new_with_page_for_test("BID-background", "TID-background");
        conn.browser_context = Some(active);
        conn.inactive_browser_contexts.push(background);
        let owner =
            crate::conn::CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
                browser_context_id: "BID-background".into(),
                target_id: "TID-background".into(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            });
        let mut events = Vec::new();
        crate::domains::page::emit_prepared_child_frame_tree_background_events(
            &mut conn,
            &mut events,
            &owner,
            vec![
                crate::domains::page::PagePreparedChildFrameTreeEvent::Attached {
                    frame_id: "FRAME-background".into(),
                    parent_frame_id: "TID-background".into(),
                },
            ],
        );

        for (context, owner) in [
            ("TID-active", "BID-active"),
            ("FRAME-active", "BID-active"),
            ("TID-background", "BID-background"),
            ("FRAME-background", "BID-background"),
        ] {
            assert_eq!(
                conn.browser_context_id_for_browsing_context(context),
                Some(owner)
            );
        }
        assert_eq!(
            conn.browser_context_id_for_browsing_context("FRAME-missing"),
            None
        );
        crate::domains::page::emit_prepared_child_frame_tree_background_events(
            &mut conn,
            &mut events,
            &owner,
            vec![
                crate::domains::page::PagePreparedChildFrameTreeEvent::Detached {
                    frame_id: "FRAME-background".into(),
                },
            ],
        );
        assert_eq!(
            conn.browser_context_id_for_browsing_context("FRAME-background"),
            None
        );
        assert_eq!(
            conn.browser_context
                .as_ref()
                .map(|context| context.id.as_str()),
            Some("BID-active")
        );
    }
}
