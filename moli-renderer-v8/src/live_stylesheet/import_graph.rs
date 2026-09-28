use super::*;

impl LiveStylesheet {
    pub(crate) fn import_options(
        &self,
    ) -> Option<crate::stylesheet_blocking::StylesheetFetchOptions> {
        self.import_options.borrow().clone()
    }

    pub(crate) fn set_import_options(
        &self,
        options: crate::stylesheet_blocking::StylesheetFetchOptions,
    ) {
        self.import_options.replace(Some(options));
    }

    pub(crate) fn top_level_import_state(&self) -> (bool, Vec<url::Url>) {
        let edges = self.import_edges.borrow();
        let has_import_rules = !edges.is_empty();
        let mut urls = Vec::new();
        let guard = self.stylesheet.shared_lock.read();
        for edge in edges.iter() {
            if edge.state == LiveStylesheetImportState::Refused {
                continue;
            }
            let Some(url) = edge.rule.read_with(&guard).url.url() else {
                continue;
            };
            if !urls.iter().any(|existing| existing == url.as_ref()) {
                urls.push(url.as_ref().clone());
            }
        }
        (has_import_rules, urls)
    }

    pub(crate) fn pending_import_requests(&self) -> Vec<LiveStylesheetImportRequest> {
        let guard = self.stylesheet.shared_lock.read();
        self.import_edges
            .borrow()
            .iter()
            .filter(|edge| edge.state == LiveStylesheetImportState::Pending)
            .filter_map(|edge| {
                Some(LiveStylesheetImportRequest {
                    edge_id: edge.id,
                    url: edge.rule.read_with(&guard).url.url()?.as_ref().clone(),
                    import_options: self
                        .import_options()
                        .filter(|options| options.referrer_url().is_some()),
                })
            })
            .collect()
    }

    pub(crate) fn pending_import_requests_in_graph(&self) -> Vec<LiveStylesheetImportRequest> {
        let mut requests = self.pending_import_requests();
        let mut visited = HashSet::from([self.id]);
        let mut pending = self
            .loaded_import_children()
            .into_iter()
            .rev()
            .map(|(_, child, _)| child)
            .collect::<Vec<_>>();

        while let Some(stylesheet) = pending.pop() {
            if !visited.insert(stylesheet.id) {
                continue;
            }
            requests.extend(stylesheet.pending_import_requests());
            pending.extend(
                stylesheet
                    .loaded_import_children()
                    .into_iter()
                    .rev()
                    .map(|(_, child, _)| child),
            );
        }
        requests
    }

    #[cfg(test)]
    pub(crate) fn import_edge_state(
        &self,
        edge_id: StylesheetImportEdgeId,
    ) -> Option<LiveStylesheetImportState> {
        self.import_edges
            .borrow()
            .iter()
            .find(|edge| edge.id == edge_id)
            .map(|edge| edge.state)
    }

    pub(super) fn imported_child_for_rule(
        &self,
        rule: &ServoArc<Locked<ImportRule>>,
    ) -> Option<LiveStylesheetRef> {
        self.import_edges
            .borrow()
            .iter()
            .find(|edge| ServoArc::ptr_eq(&edge.rule, rule))
            .and_then(|edge| edge.child.clone())
    }

    pub(super) fn loaded_import_children(
        &self,
    ) -> Vec<(url::Url, LiveStylesheetRef, LiveStylesheetImportState)> {
        let guard = self.stylesheet.shared_lock.read();
        self.import_edges
            .borrow()
            .iter()
            .filter_map(|edge| {
                Some((
                    edge.rule.read_with(&guard).url.url()?.as_ref().clone(),
                    edge.child.clone()?,
                    edge.state,
                ))
            })
            .collect()
    }

    pub(crate) const fn id(&self) -> StylesheetId {
        self.id
    }

    fn native_top_level_import_rules(&self) -> Vec<ServoArc<Locked<ImportRule>>> {
        let guard = self.stylesheet.shared_lock.read();
        self.stylesheet
            .contents(&guard)
            .rules(&guard)
            .iter()
            .filter_map(|rule| match rule {
                CssRule::Import(rule) => Some(rule.clone()),
                _ => None,
            })
            .collect()
    }

    fn allocate_import_edge_id(&self) -> StylesheetImportEdgeId {
        let id = self
            .next_import_edge_id
            .get()
            .checked_add(1)
            .expect("stylesheet import edge identity space exhausted");
        self.next_import_edge_id.set(id);
        StylesheetImportEdgeId(id)
    }

    pub(super) fn reconcile_import_edges(&self) {
        let rules = self.native_top_level_import_rules();
        let mut previous = std::mem::take(&mut *self.import_edges.borrow_mut());
        let previous_ids = previous.iter().map(|edge| edge.id).collect::<Vec<_>>();
        let mut current = Vec::with_capacity(rules.len());

        for rule in rules {
            if let Some(index) = previous
                .iter()
                .position(|edge| ServoArc::ptr_eq(&edge.rule, &rule))
            {
                current.push(previous.swap_remove(index));
                continue;
            }

            let state = {
                let guard = self.stylesheet.shared_lock.read();
                let rule = rule.read_with(&guard);
                if rule
                    .supports
                    .as_ref()
                    .is_some_and(|supports| !supports.enabled)
                    || rule.url.url().is_none()
                    || matches!(rule.stylesheet, ImportSheet::Refused)
                {
                    LiveStylesheetImportState::Refused
                } else {
                    LiveStylesheetImportState::Pending
                }
            };
            current.push(LiveStylesheetImportEdge {
                id: self.allocate_import_edge_id(),
                rule,
                state,
                child: None,
            });
        }

        let graph_changed = previous_ids != current.iter().map(|edge| edge.id).collect::<Vec<_>>();
        for removed in previous {
            if let Some(child) = removed.child {
                child.clear_parent_if(self.id, removed.id);
            }
        }
        *self.import_edges.borrow_mut() = current;
        if graph_changed {
            self.note_import_graph_mutation();
        }
    }

    fn clear_parent_if(&self, parent_id: StylesheetId, edge_id: StylesheetImportEdgeId) {
        let should_clear = self.parent.borrow().as_ref().is_some_and(|parent| {
            parent.edge_id == edge_id
                && parent
                    .stylesheet
                    .upgrade()
                    .is_some_and(|stylesheet| stylesheet.id == parent_id)
        });
        if should_clear {
            self.parent.borrow_mut().take();
        }
    }

    pub(super) fn import_edge_media(
        &self,
        edge_id: StylesheetImportEdgeId,
    ) -> Option<ServoArc<Locked<MediaList>>> {
        let edges = self.import_edges.borrow();
        let edge = edges.iter().find(|edge| edge.id == edge_id)?;
        if edge.state != LiveStylesheetImportState::Pending {
            return None;
        }
        let guard = self.stylesheet.shared_lock.read();
        edge.rule
            .read_with(&guard)
            .stylesheet
            .as_sheet()
            .map(|stylesheet| stylesheet.media.clone())
    }

    pub(super) fn install_import_child(
        self: &LiveStylesheetRef,
        edge_id: StylesheetImportEdgeId,
        child: LiveStylesheetRef,
        successful: bool,
    ) -> bool {
        let import_rule = {
            let edges = self.import_edges.borrow();
            let Some(edge) = edges.iter().find(|edge| edge.id == edge_id) else {
                return false;
            };
            if edge.state != LiveStylesheetImportState::Pending {
                return false;
            }
            edge.rule.clone()
        };

        {
            let mut guard = self.stylesheet.shared_lock.write();
            import_rule.write_with(&mut guard).stylesheet = ImportSheet::new(child.stylesheet());
        }
        child.parent.replace(Some(LiveStylesheetParent {
            stylesheet: Rc::downgrade(self),
            edge_id,
        }));
        let mut edges = self.import_edges.borrow_mut();
        let Some(edge) = edges.iter_mut().find(|edge| edge.id == edge_id) else {
            child.parent.borrow_mut().take();
            return false;
        };
        edge.state = LiveStylesheetImportState::Loaded { successful };
        edge.child = Some(child);
        drop(edges);
        self.note_import_graph_mutation();
        self.note_import_descendant_mutation();
        true
    }

    fn note_import_graph_mutation(&self) {
        self.for_each_import_ancestor_including_self(|stylesheet| {
            stylesheet
                .import_generation
                .set(stylesheet.import_generation.get().saturating_add(1));
        });
    }

    pub(super) fn note_import_descendant_mutation(&self) {
        self.for_each_import_ancestor_including_self(|stylesheet| {
            let generation = stylesheet.cascade_generation.get().saturating_add(1);
            stylesheet.cascade_generation.set(generation);
            stylesheet.publish_cascade_mutation(generation, LiveStylesheetCascadeMutation::Full);
            stylesheet.derived_state.clear_dependency_summary();
            stylesheet.font_face_cache.borrow_mut().take();
        });
    }

    fn for_each_import_ancestor_including_self(&self, mut visit: impl FnMut(&LiveStylesheet)) {
        let mut visited = HashSet::from([self.id]);
        visit(self);
        let mut parent = self
            .parent
            .borrow()
            .as_ref()
            .and_then(|parent| parent.stylesheet.upgrade());
        while let Some(stylesheet) = parent {
            if !visited.insert(stylesheet.id) {
                break;
            }
            visit(&stylesheet);
            parent = stylesheet
                .parent
                .borrow()
                .as_ref()
                .and_then(|parent| parent.stylesheet.upgrade());
        }
    }
}
