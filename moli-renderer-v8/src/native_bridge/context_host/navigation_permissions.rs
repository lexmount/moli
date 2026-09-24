use super::{JsContextHost, OwnerDispatchScope, window_security_tokens::WindowAccessOrigin};
use url::Url;

impl JsContextHost {
    /// Admission for a child navigating one of its ancestors. Location,
    /// hyperlinks and forms must check the source before entering the target
    /// realm or publishing navigation work.
    pub(crate) fn blocks_ancestor_navigation(
        &self,
        source: OwnerDispatchScope,
        target: OwnerDispatchScope,
        destination: &Url,
    ) -> bool {
        let OwnerDispatchScope::Child(source_handle) = source else {
            return false;
        };
        let mut ancestor = self.navigation_parent_scope(source);
        while let Some(owner) = ancestor {
            if owner == target {
                break;
            }
            ancestor = self.navigation_parent_scope(owner);
        }
        if ancestor.is_none() {
            return false;
        }
        let Some(entry) = self.child_browsing_contexts.get(&source_handle) else {
            return true;
        };
        let sandbox = entry.document_sandbox_policy();
        let target_is_top = !matches!(target, OwnerDispatchScope::Child(_));
        if destination.scheme() == "javascript"
            && !self.window_scopes_have_same_origin_domain(source, target)
        {
            return true;
        }
        // Consult the active Document's flags, including inherited/CSP flags.
        // Mutating the container's sandbox attribute does not change them.
        if sandbox.restricts_navigation {
            if !target_is_top {
                return true;
            }
            return if sandbox.allows_top_navigation {
                !entry.can_navigate_top_without_user_gesture
                    && !self.window_user_activation_state(source).1
            } else {
                !(sandbox.allows_top_navigation_by_user_activation
                    && self.window_has_transient_user_activation(source))
            };
        }
        let mut accessible_ancestor = Some(target);
        while let Some(owner) = accessible_ancestor {
            if self.window_scopes_have_same_origin_domain(source, owner) {
                return false;
            }
            accessible_ancestor = self.navigation_parent_scope(owner);
        }
        // An unrelated intermediate ancestor cannot be navigated, even with
        // activation. A top-level ancestor additionally permits sticky
        // activation and destinations related to its existing origin/site.
        !(target_is_top
            && (self.window_user_activation_state(source).1
                || self.ancestor_navigation_destination_is_related(target, destination)))
    }

    pub(super) fn capture_child_top_navigation_permission(
        &mut self,
        handle: crate::document_runtime::DomHandle,
    ) {
        let source = OwnerDispatchScope::Child(handle);
        let Some(parent) = self.navigation_parent_scope(source) else {
            return;
        };
        let mut top = parent;
        while let Some(ancestor) = self.navigation_parent_scope(top) {
            top = ancestor;
        }
        let parent_allows = match parent {
            OwnerDispatchScope::Child(parent) => self
                .child_browsing_contexts
                .get(&parent)
                .is_some_and(|entry| entry.can_navigate_top_without_user_gesture),
            OwnerDispatchScope::Top | OwnerDispatchScope::LightweightPopup(_) => true,
        };
        // A cross-origin parent cannot delegate a permission it does not have.
        // Unlike access checks, this committed grant ignores document.domain.
        let allowed = self.window_scopes_have_same_origin(source, top)
            || (parent_allows
                && self
                    .dom_host()
                    .get_attribute(handle, "sandbox")
                    .is_some_and(|value| {
                        value
                            .split_ascii_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("allow-top-navigation"))
                    }));
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.can_navigate_top_without_user_gesture = allowed;
        }
    }

    fn navigation_parent_scope(&self, owner: OwnerDispatchScope) -> Option<OwnerDispatchScope> {
        match owner {
            OwnerDispatchScope::Child(handle) => self.owner_dispatch_scope_for_node(handle),
            OwnerDispatchScope::Top | OwnerDispatchScope::LightweightPopup(_) => None,
        }
    }

    fn ancestor_navigation_destination_is_related(
        &self,
        target: OwnerDispatchScope,
        destination: &Url,
    ) -> bool {
        let Some(origin) = self.window_access_origin_for_dispatch_scope(target) else {
            return false;
        };
        if WindowAccessOrigin::from_serialized_origin(
            moli_url::origin_ascii_serialization(destination),
            None,
        )
        .is_some_and(|destination_origin| origin.can_access(&destination_origin))
        {
            return true;
        }
        let WindowAccessOrigin::Tuple {
            serialized_origin,
            scheme,
            document_domain,
        } = origin
        else {
            return false;
        };
        if scheme != destination.scheme() {
            return false;
        }
        let target_host = document_domain.or_else(|| {
            Url::parse(&serialized_origin)
                .ok()?
                .host_str()
                .map(str::to_owned)
        });
        target_host
            .as_deref()
            .and_then(navigation_registrable_domain)
            .zip(
                destination
                    .host_str()
                    .and_then(navigation_registrable_domain),
            )
            .is_some_and(|(target, destination)| target == destination)
    }
}

fn navigation_registrable_domain(host: &str) -> Option<&str> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.parse::<std::net::IpAddr>().is_ok()
        || !host.contains('.')
        || moli_site::host_is_public_suffix(host)
    {
        return None;
    }
    Some(moli_site::registrable_site_host(host))
}
