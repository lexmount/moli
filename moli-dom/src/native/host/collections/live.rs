use super::*;

impl DomHost {
    pub fn resolve_live_collection(
        &self,
        root: DomHandle,
        kind: &str,
        query: Option<&str>,
        include_root: bool,
    ) -> Option<Vec<DomHandle>> {
        self.node(root)?;
        let kind = LiveCollectionCacheKind::parse(kind)?;
        let key = LiveCollectionCacheKey {
            root,
            kind,
            query: query.map(str::to_owned),
            include_root,
        };
        let version = self.query_version.get();
        if let Some(cached) = self
            .live_collection_cache
            .borrow()
            .get(&key)
            .filter(|cached| cached.version == version)
        {
            return Some(cached.handles.clone());
        }

        let handles = match kind {
            LiveCollectionCacheKind::ChildNodes => self.child_nodes(root)?,
            LiveCollectionCacheKind::Children => self.child_element_nodes(root),
            LiveCollectionCacheKind::FormControls => self.form_control_elements(root),
            LiveCollectionCacheKind::FormControlsByName => {
                let name = query.unwrap_or_default();
                self.form_control_elements(root)
                    .into_iter()
                    .filter(|handle| {
                        self.node(*handle)
                            .and_then(Node::as_element)
                            .is_some_and(|el| el.matches_named_item_key(name))
                    })
                    .collect()
            }
            LiveCollectionCacheKind::Options => self.select_option_elements(root),
            LiveCollectionCacheKind::SelectedOptions => self.select_selected_option_elements(root),
            LiveCollectionCacheKind::TagName => {
                self.elements_by_tag_name(root, query.unwrap_or("*"), include_root)
            }
            LiveCollectionCacheKind::TagNameNs => {
                let (namespace, local_name) = parse_tag_name_ns_query(query.unwrap_or_default())?;
                self.elements_by_tag_name_ns(root, namespace, local_name, include_root)
            }
            LiveCollectionCacheKind::ClassName => {
                self.elements_by_class_name(root, query.unwrap_or_default(), include_root)
            }
            LiveCollectionCacheKind::Name => {
                self.elements_by_name(root, query.unwrap_or_default(), include_root)
            }
            LiveCollectionCacheKind::Forms => {
                self.collect_matching_elements(root, include_root, |handle| {
                    self.is_html_element_named(handle, "form")
                })
            }
            LiveCollectionCacheKind::Images => {
                self.collect_matching_elements(root, include_root, |handle| {
                    self.is_html_element_named(handle, "img")
                })
            }
            LiveCollectionCacheKind::Scripts => {
                self.collect_matching_elements(root, include_root, |handle| {
                    self.is_html_element_named(handle, "script")
                })
            }
            LiveCollectionCacheKind::Links => {
                self.collect_matching_elements(root, include_root, |handle| {
                    self.node(handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            (element.is_html_element("a") || element.is_html_element("area"))
                                && element.has_attribute_ns("", "href")
                        })
                })
            }
            LiveCollectionCacheKind::Anchors => {
                self.collect_matching_elements(root, include_root, |handle| {
                    self.node(handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("a") && element.has_attribute_ns("", "name")
                        })
                })
            }
            LiveCollectionCacheKind::Embeds => {
                self.collect_matching_elements(root, include_root, |handle| {
                    self.is_html_element_named(handle, "embed")
                })
            }
            LiveCollectionCacheKind::Applets => Vec::new(),
            LiveCollectionCacheKind::TableRows => self.table_row_elements(root),
            LiveCollectionCacheKind::TableBodies => self.table_body_elements(root),
            LiveCollectionCacheKind::TableSectionRows => self.table_section_row_elements(root),
            LiveCollectionCacheKind::TableRowCells => self.table_row_cell_elements(root),
        };
        self.live_collection_cache.borrow_mut().insert(
            key,
            CachedLiveCollection {
                version,
                handles: handles.clone(),
            },
        );
        Some(handles)
    }
}

fn parse_tag_name_ns_query(query: &str) -> Option<(Option<&str>, &str)> {
    let (namespace, local_name) = query.split_once('\u{0}')?;
    let namespace = if namespace == "*" {
        Some("*")
    } else if namespace.is_empty() {
        None
    } else {
        Some(namespace)
    };
    Some((namespace, local_name))
}
