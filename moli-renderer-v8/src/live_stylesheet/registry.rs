use super::*;

#[derive(Debug, Default)]
pub(crate) struct LiveStylesheetRegistry {
    next_id: Cell<u64>,
    entries: RefCell<HashMap<StylesheetId, RcWeak<LiveStylesheet>>>,
    initial_contents_cache:
        RefCell<HashMap<StylesheetContentsCacheKey, StdWeak<SharedStylesheetContents>>>,
    next_wrapper_lease_id: Cell<u64>,
    wrapper_leases:
        RefCell<HashMap<StylesheetWrapperLeaseId, RcWeak<RefCell<Option<LiveStylesheetRef>>>>>,
    next_rule_wrapper_lease_id: Cell<u64>,
    rule_wrapper_leases: RefCell<
        HashMap<
            StylesheetRuleWrapperLeaseId,
            RcWeak<RefCell<Option<StylesheetRuleWrapperBinding>>>,
        >,
    >,
}

impl LiveStylesheetRegistry {
    const MAX_INLINE_CONTENTS_CACHE_ENTRIES: usize = 256;

    pub(crate) fn create(
        &self,
        css_text: &str,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
        shared_lock: SharedRwLock,
    ) -> LiveStylesheetRef {
        let id = self.allocate_id();
        let stylesheet = Rc::new(LiveStylesheet::parse(
            id,
            css_text,
            base_url,
            quirks_mode,
            allow_import_rules,
            shared_lock,
        ));
        self.entries
            .borrow_mut()
            .insert(id, Rc::downgrade(&stylesheet));
        stylesheet
    }

    pub(crate) fn create_inline_with_shared_initial_contents(
        &self,
        css_text: &str,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        shared_lock: SharedRwLock,
    ) -> LiveStylesheetRef {
        self.create_with_shared_initial_contents(
            css_text,
            base_url,
            quirks_mode,
            AllowImportRules::Yes,
            shared_lock,
        )
    }

    pub(crate) fn create_with_shared_initial_contents(
        &self,
        css_text: &str,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
        shared_lock: SharedRwLock,
    ) -> LiveStylesheetRef {
        // Import-bearing sheets never enter this cache because every import
        // edge owns an independently mutable child stylesheet graph.
        let key =
            StylesheetContentsCacheKey::new(css_text, &base_url, quirks_mode, allow_import_rules);
        let shared_contents = self
            .initial_contents_cache
            .borrow()
            .get(&key)
            .and_then(StdWeak::upgrade);
        if let Some(shared_contents) = shared_contents {
            // The registry belongs to one JsContextHost and therefore one
            // author-lock domain. Assert that invariant before reusing parsed
            // contents so an accidental cross-engine call fails at the owner.
            {
                let guard = shared_lock.read();
                let _ = shared_contents.contents.rules(&guard);
            }
            let id = self.allocate_id();
            let stylesheet = Rc::new(LiveStylesheet::from_shared_initial_contents(
                id,
                shared_lock,
                shared_contents,
            ));
            self.entries
                .borrow_mut()
                .insert(id, Rc::downgrade(&stylesheet));
            return stylesheet;
        }
        self.initial_contents_cache.borrow_mut().remove(&key);

        let stylesheet = self.create(
            css_text,
            base_url,
            quirks_mode,
            allow_import_rules,
            shared_lock,
        );
        if !stylesheet.initial_contents_can_be_shared() {
            return stylesheet;
        }

        let mut cache = self.initial_contents_cache.borrow_mut();
        if cache.len() >= Self::MAX_INLINE_CONTENTS_CACHE_ENTRIES {
            cache.retain(|_, contents| contents.strong_count() != 0);
        }
        if cache.len() < Self::MAX_INLINE_CONTENTS_CACHE_ENTRIES {
            let shared_contents = stylesheet.share_initial_contents();
            cache.insert(key, StdArc::downgrade(&shared_contents));
        }
        stylesheet
    }

    pub(crate) fn create_linked_resource_template_with_shared_initial_contents(
        &self,
        css_text: &str,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        shared_lock: SharedRwLock,
    ) -> LiveStylesheetRef {
        self.create_with_shared_initial_contents(
            css_text,
            base_url,
            quirks_mode,
            AllowImportRules::Yes,
            shared_lock,
        )
    }

    pub(crate) fn create_from_shared_initial_contents(
        &self,
        shared_contents: StdArc<SharedStylesheetContents>,
        shared_lock: SharedRwLock,
    ) -> LiveStylesheetRef {
        // Shared contents are scoped to one JsContextHost author lock. Reading
        // through the destination lock makes an accidental cross-domain use
        // fail at the ownership boundary before the stylesheet is installed.
        {
            let guard = shared_lock.read();
            let _ = shared_contents.contents.rules(&guard);
        }
        let id = self.allocate_id();
        let stylesheet = Rc::new(LiveStylesheet::from_shared_initial_contents(
            id,
            shared_lock,
            shared_contents,
        ));
        self.entries
            .borrow_mut()
            .insert(id, Rc::downgrade(&stylesheet));
        stylesheet
    }

    pub(crate) fn create_from_parsed_stylesheet(
        &self,
        parsed_stylesheet: &ServoArc<Stylesheet>,
        base_url: url::Url,
        quirks_mode: QuirksMode,
        allow_import_rules: AllowImportRules,
    ) -> LiveStylesheetRef {
        let shared_lock = parsed_stylesheet.shared_lock.clone();
        let (contents, media) = {
            let guard = shared_lock.read();
            (
                parsed_stylesheet
                    .contents(&guard)
                    .deep_clone(&shared_lock, None, &guard),
                parsed_stylesheet.media.read_with(&guard).clone(),
            )
        };
        let stylesheet = ServoArc::new(Stylesheet {
            contents: shared_lock.wrap(contents),
            shared_lock: shared_lock.clone(),
            media: ServoArc::new(shared_lock.wrap(media)),
            disabled: AtomicBool::new(false),
        });
        let id = self.allocate_id();
        let stylesheet = Rc::new(LiveStylesheet::from_stylesheet(
            id,
            stylesheet,
            base_url,
            quirks_mode,
            allow_import_rules,
            LiveStylesheetRuntimeStateKind::Cascade,
            None,
            None,
        ));
        self.entries
            .borrow_mut()
            .insert(id, Rc::downgrade(&stylesheet));
        stylesheet
    }

    pub(crate) fn get(&self, id: StylesheetId) -> Option<LiveStylesheetRef> {
        let stylesheet = self.entries.borrow().get(&id)?.upgrade();
        if stylesheet.is_none() {
            self.entries.borrow_mut().remove(&id);
        }
        stylesheet
    }

    pub(crate) fn install_import_response(
        &self,
        parent_id: StylesheetId,
        expected_contents_revision: u64,
        edge_id: StylesheetImportEdgeId,
        css_text: &str,
        response_url: url::Url,
        successful: bool,
        origin_clean: bool,
    ) -> Option<LiveStylesheetRef> {
        let parent = self.get(parent_id)?;
        if parent.contents_revision() != expected_contents_revision {
            return None;
        }
        let media = parent.import_edge_media(edge_id)?;
        let id = self.allocate_id();
        let child = Rc::new(LiveStylesheet::parse_with_media(
            id,
            if successful { css_text } else { "" },
            response_url,
            parent.quirks_mode,
            AllowImportRules::Yes,
            parent.stylesheet.shared_lock.clone(),
            media,
            LiveStylesheetRuntimeStateKind::IndependentCssom,
        ));
        child.set_origin_clean(origin_clean);
        self.entries.borrow_mut().insert(id, Rc::downgrade(&child));
        if !parent.install_import_child(edge_id, child.clone(), successful) {
            self.entries.borrow_mut().remove(&id);
            return None;
        }
        Some(child)
    }

    pub(crate) fn install_import_graph(
        &self,
        root_id: StylesheetId,
        expected_contents_revision: u64,
        expected_import_generation: u64,
        responses: &[LiveStylesheetImportResponse],
        root_resource_url: Option<&url::Url>,
    ) -> Option<bool> {
        let root = self.get(root_id)?;
        if root.contents_revision() != expected_contents_revision
            || root.import_generation() != expected_import_generation
        {
            return None;
        }
        let mut ancestors = HashSet::new();
        if let Some(url) = root_resource_url {
            ancestors.insert(import_url_identity(url));
        }
        let mut responses_by_request_identity = HashMap::new();
        for response in responses {
            responses_by_request_identity
                .entry(import_url_identity(&response.request_url))
                .or_insert(response);
        }
        let mut data_expansions = 0;
        Some(self.install_import_graph_for_stylesheet(
            &root,
            &responses_by_request_identity,
            &ancestors,
            &mut data_expansions,
        ))
    }

    fn install_import_graph_for_stylesheet(
        &self,
        parent: &LiveStylesheetRef,
        responses_by_request_identity: &HashMap<url::Url, &LiveStylesheetImportResponse>,
        ancestors: &HashSet<url::Url>,
        data_expansions: &mut usize,
    ) -> bool {
        enum ImportGraphTraversal {
            Visit {
                stylesheet: LiveStylesheetRef,
                ancestor_identities: Option<(url::Url, url::Url)>,
            },
            Leave {
                inserted_ancestor_identities: [Option<url::Url>; 2],
            },
        }

        let mut active_ancestors = ancestors.clone();
        let mut traversal = vec![ImportGraphTraversal::Visit {
            stylesheet: parent.clone(),
            ancestor_identities: None,
        }];
        let mut all_successful = true;

        while let Some(step) = traversal.pop() {
            let (parent, inserted_ancestor_identities) = match step {
                ImportGraphTraversal::Leave {
                    inserted_ancestor_identities,
                } => {
                    for identity in inserted_ancestor_identities.into_iter().flatten() {
                        active_ancestors.remove(&identity);
                    }
                    continue;
                }
                ImportGraphTraversal::Visit {
                    stylesheet,
                    ancestor_identities,
                } => {
                    let mut inserted_ancestor_identities = [None, None];
                    if let Some((request_identity, response_identity)) = ancestor_identities {
                        if active_ancestors.insert(request_identity.clone()) {
                            inserted_ancestor_identities[0] = Some(request_identity);
                        }
                        if active_ancestors.insert(response_identity.clone()) {
                            inserted_ancestor_identities[1] = Some(response_identity);
                        }
                    }
                    (stylesheet, inserted_ancestor_identities)
                }
            };
            traversal.push(ImportGraphTraversal::Leave {
                inserted_ancestor_identities,
            });

            for request in parent.pending_import_requests() {
                let request_identity = import_url_identity(&request.url);
                if active_ancestors.contains(&request_identity) {
                    all_successful = false;
                    let _ = self.install_import_response(
                        parent.id(),
                        parent.contents_revision(),
                        request.edge_id,
                        "",
                        request.url,
                        false,
                        true,
                    );
                    continue;
                }

                let response = if request.url.scheme() == "data" {
                    *data_expansions = data_expansions.saturating_add(1);
                    let decoded = (*data_expansions <= MAX_DATA_STYLESHEET_IMPORT_EXPANSIONS)
                        .then(|| decode_data_stylesheet(&request.url))
                        .flatten();
                    match decoded {
                        Some(css_text) => LiveStylesheetImportResponse {
                            request_url: request.url.clone(),
                            response_url: request.url.clone(),
                            css_text,
                            successful: true,
                            origin_clean: true,
                        },
                        None => LiveStylesheetImportResponse {
                            request_url: request.url.clone(),
                            response_url: request.url.clone(),
                            css_text: String::new(),
                            successful: false,
                            origin_clean: true,
                        },
                    }
                } else {
                    responses_by_request_identity
                        .get(&request_identity)
                        .map(|response| (**response).clone())
                        .unwrap_or_else(|| LiveStylesheetImportResponse {
                            request_url: request.url.clone(),
                            response_url: request.url.clone(),
                            css_text: String::new(),
                            successful: false,
                            origin_clean: false,
                        })
                };
                all_successful &= response.successful;
                if self
                    .install_import_response(
                        parent.id(),
                        parent.contents_revision(),
                        request.edge_id,
                        &response.css_text,
                        response.response_url.clone(),
                        response.successful,
                        response.origin_clean,
                    )
                    .is_none()
                {
                    all_successful = false;
                }
            }

            let mut children = Vec::new();
            for (request_url, child, state) in parent.loaded_import_children() {
                let successful = matches!(
                    state,
                    LiveStylesheetImportState::Loaded { successful: true }
                );
                all_successful &= successful;
                if successful {
                    children.push((request_url, child));
                }
            }
            for (request_url, child) in children.into_iter().rev() {
                let child_base_url = import_url_identity(child.base_url());
                traversal.push(ImportGraphTraversal::Visit {
                    stylesheet: child,
                    ancestor_identities: Some((import_url_identity(&request_url), child_base_url)),
                });
            }
        }
        all_successful
    }

    pub(crate) fn imported_stylesheet_for_rule_wrapper(
        &self,
        id: StylesheetRuleWrapperLeaseId,
        expected_stylesheet_id: StylesheetId,
    ) -> Option<LiveStylesheetRef> {
        let lease = self
            .rule_wrapper_leases
            .borrow()
            .get(&id)
            .and_then(RcWeak::upgrade);
        let Some(lease) = lease else {
            self.rule_wrapper_leases.borrow_mut().remove(&id);
            return None;
        };
        let import_rule = {
            let binding = lease.borrow();
            let binding = binding.as_ref()?;
            if binding.stylesheet_id != Some(expected_stylesheet_id) {
                return None;
            }
            match &binding.rule {
                NativeStylesheetRule::Css(CssRule::Import(rule)) => rule.clone(),
                _ => return None,
            }
        };
        self.get(expected_stylesheet_id)?
            .imported_child_for_rule(&import_rule)
    }

    fn allocate_id(&self) -> StylesheetId {
        loop {
            let id = self
                .next_id
                .get()
                .checked_add(1)
                .expect("live stylesheet id space exhausted");
            self.next_id.set(if id == u64::MAX { 0 } else { id });
            let id = StylesheetId(id);
            if !self.entries.borrow().contains_key(&id) {
                return id;
            }
        }
    }

    pub(crate) fn create_wrapper_lease(
        &self,
        stylesheet: LiveStylesheetRef,
    ) -> (StylesheetWrapperLeaseId, StylesheetWrapperLease) {
        self.wrapper_leases
            .borrow_mut()
            .retain(|_, lease| lease.strong_count() != 0);
        let id = self.allocate_wrapper_lease_id();
        let lease = Rc::new(RefCell::new(Some(stylesheet)));
        self.wrapper_leases
            .borrow_mut()
            .insert(id, Rc::downgrade(&lease));
        (id, lease)
    }

    pub(crate) fn replace_wrapper_lease(
        &self,
        id: StylesheetWrapperLeaseId,
        stylesheet: Option<LiveStylesheetRef>,
    ) -> bool {
        let lease = self
            .wrapper_leases
            .borrow()
            .get(&id)
            .and_then(RcWeak::upgrade);
        let Some(lease) = lease else {
            self.wrapper_leases.borrow_mut().remove(&id);
            return false;
        };
        *lease.borrow_mut() = stylesheet;
        true
    }

    fn allocate_wrapper_lease_id(&self) -> StylesheetWrapperLeaseId {
        loop {
            let id = self
                .next_wrapper_lease_id
                .get()
                .checked_add(1)
                .expect("live stylesheet wrapper-lease id space exhausted");
            self.next_wrapper_lease_id
                .set(if id == u64::MAX { 0 } else { id });
            let id = StylesheetWrapperLeaseId(id);
            if !self.wrapper_leases.borrow().contains_key(&id) {
                return id;
            }
        }
    }

    pub(crate) fn bind_rule_wrapper(
        &self,
        existing_id: Option<StylesheetRuleWrapperLeaseId>,
        stylesheet: &LiveStylesheetRef,
        path: Vec<usize>,
    ) -> Option<(
        StylesheetRuleWrapperLeaseId,
        Option<StylesheetRuleWrapperLease>,
    )> {
        let rule = stylesheet.native_rule_at_path(&path)?;
        let binding = StylesheetRuleWrapperBinding {
            stylesheet_id: Some(stylesheet.id()),
            path,
            rule,
            shared_lock: stylesheet.stylesheet.shared_lock.clone(),
            source_declaration_override: Rc::clone(&stylesheet.source_declaration_override),
            detached_source_declaration_override: None,
        };

        if let Some(id) = existing_id {
            let lease = self
                .rule_wrapper_leases
                .borrow()
                .get(&id)
                .and_then(RcWeak::upgrade);
            if let Some(lease) = lease {
                *lease.borrow_mut() = Some(binding);
                stylesheet.track_rule_wrapper_lease(id, &lease);
                return Some((id, None));
            }
            self.rule_wrapper_leases.borrow_mut().remove(&id);
        }

        if self.next_rule_wrapper_lease_id.get().is_multiple_of(256) {
            self.rule_wrapper_leases
                .borrow_mut()
                .retain(|_, lease| lease.strong_count() != 0);
        }
        let id = self.allocate_rule_wrapper_lease_id();
        let lease = Rc::new(RefCell::new(Some(binding)));
        self.rule_wrapper_leases
            .borrow_mut()
            .insert(id, Rc::downgrade(&lease));
        stylesheet.track_rule_wrapper_lease(id, &lease);
        Some((id, Some(lease)))
    }

    #[cfg(test)]
    pub(crate) fn rule_wrapper_binding(
        &self,
        id: StylesheetRuleWrapperLeaseId,
    ) -> Option<StylesheetRuleWrapperBinding> {
        let lease = self
            .rule_wrapper_leases
            .borrow()
            .get(&id)
            .and_then(RcWeak::upgrade);
        let Some(lease) = lease else {
            self.rule_wrapper_leases.borrow_mut().remove(&id);
            return None;
        };
        lease.borrow().as_ref().cloned()
    }

    pub(crate) fn release_rule_wrapper(&self, id: StylesheetRuleWrapperLeaseId) -> bool {
        let lease = self
            .rule_wrapper_leases
            .borrow_mut()
            .remove(&id)
            .and_then(|lease| lease.upgrade());
        lease.is_some_and(|lease| lease.borrow_mut().take().is_some())
    }

    pub(crate) fn retained_rule_wrapper_snapshot_for_detach(
        &self,
        id: StylesheetRuleWrapperLeaseId,
    ) -> Option<CssRuleSnapshot> {
        let lease = self
            .rule_wrapper_leases
            .borrow()
            .get(&id)
            .and_then(RcWeak::upgrade);
        let Some(lease) = lease else {
            self.rule_wrapper_leases.borrow_mut().remove(&id);
            return None;
        };
        lease
            .borrow()
            .as_ref()
            .map(StylesheetRuleWrapperBinding::snapshot)
    }

    pub(crate) fn with_attached_rule_wrapper<R>(
        &self,
        id: StylesheetRuleWrapperLeaseId,
        expected_stylesheet_id: StylesheetId,
        read: impl FnOnce(&StylesheetRuleWrapperBinding) -> R,
    ) -> Option<R> {
        let lease = self
            .rule_wrapper_leases
            .borrow()
            .get(&id)
            .and_then(RcWeak::upgrade);
        let Some(lease) = lease else {
            self.rule_wrapper_leases.borrow_mut().remove(&id);
            return None;
        };
        let binding = lease.borrow();
        let binding = binding.as_ref()?;
        (binding.stylesheet_id == Some(expected_stylesheet_id)).then(|| read(binding))
    }

    pub(crate) fn attached_rule_wrapper_path(
        &self,
        id: StylesheetRuleWrapperLeaseId,
        expected_stylesheet_id: StylesheetId,
    ) -> Option<Vec<usize>> {
        let lease = self
            .rule_wrapper_leases
            .borrow()
            .get(&id)
            .and_then(RcWeak::upgrade);
        let Some(lease) = lease else {
            self.rule_wrapper_leases.borrow_mut().remove(&id);
            return None;
        };
        let binding = lease.borrow();
        let binding = binding.as_ref()?;
        (binding.stylesheet_id == Some(expected_stylesheet_id)).then(|| binding.path.clone())
    }

    fn allocate_rule_wrapper_lease_id(&self) -> StylesheetRuleWrapperLeaseId {
        loop {
            let id = self
                .next_rule_wrapper_lease_id
                .get()
                .saturating_add(1)
                .max(1);
            self.next_rule_wrapper_lease_id
                .set(if id == u64::MAX { 0 } else { id });
            let id = StylesheetRuleWrapperLeaseId(id);
            if !self.rule_wrapper_leases.borrow().contains_key(&id) {
                return id;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn live_entry_count_for_test(&self) -> usize {
        self.entries
            .borrow_mut()
            .retain(|_, stylesheet| stylesheet.strong_count() != 0);
        self.entries.borrow().len()
    }
}
