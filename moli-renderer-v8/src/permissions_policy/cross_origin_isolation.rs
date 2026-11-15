use super::{OriginAllowlist, iframe_allow_feature, response_origin_allowlist};
use url::{Origin, Url};

/// Bind response allowlists to their declaring origins and preserve inherited
/// denials when a Document creates a child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CrossOriginIsolationPermission {
    inherited: bool,
    origin: Option<Origin>,
    declarations: Vec<(Origin, OriginAllowlist)>,
}

impl Default for CrossOriginIsolationPermission {
    fn default() -> Self {
        Self {
            inherited: true,
            origin: None,
            declarations: Vec::new(),
        }
    }
}

impl CrossOriginIsolationPermission {
    fn allows_origin(&self, origin: &Origin) -> bool {
        self.declarations.iter().all(|(anchor, allowed)| {
            allowed.matches_self && origin == anchor || allowed.origins.contains(origin)
        })
    }

    pub(super) fn enabled(&self) -> bool {
        self.inherited
            && self
                .origin
                .as_ref()
                .is_none_or(|origin| self.allows_origin(origin))
    }

    pub(super) fn declare(&mut self, allowlist: &str, url: &Url) {
        self.declarations = response_origin_allowlist(allowlist)
            .map(|allowed| (url.origin(), allowed))
            .into_iter()
            .collect();
        self.origin = (!self.declarations.is_empty()).then(|| url.origin());
    }

    pub(super) fn for_document_origin(&self, origin: &Origin) -> Self {
        Self {
            origin: (!self.declarations.is_empty()).then(|| origin.clone()),
            ..self.clone()
        }
    }

    pub(super) fn intersect(&self, other: &Self) -> Self {
        Self {
            inherited: self.inherited && other.inherited,
            origin: self.origin.clone().or_else(|| other.origin.clone()),
            declarations: self
                .declarations
                .iter()
                .chain(&other.declarations)
                .cloned()
                .collect(),
        }
    }

    pub(super) fn delegated_to_child(
        &self,
        parent: &Origin,
        child: &Origin,
        source: &Origin,
        allow: Option<&str>,
    ) -> Self {
        let container = iframe_allow_feature(allow, "cross-origin-isolated", parent, child, source)
            .unwrap_or(parent == child);
        Self {
            inherited: self.inherited
                && self.allows_origin(parent)
                && self.allows_origin(child)
                && container,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::permissions_policy::DocumentPermissionsPolicy;
    use url::Url;

    #[test]
    fn isolation_permission_preserves_header_bounds_and_child_origin_delegation() {
        let parent = Url::parse("https://parent.test/").unwrap();
        let child = Url::parse("https://child.test/").unwrap();
        let policy = |value: &str| {
            DocumentPermissionsPolicy::from_navigation_response_headers(
                &[("Permissions-Policy".into(), value.as_bytes().to_vec())],
                &parent,
            )
        };
        let delegate = |policy: &DocumentPermissionsPolicy, allow| {
            policy
                .delegated_to_child(&parent.origin(), &child.origin(), &child.origin(), allow)
                .cross_origin_isolated_enabled()
        };
        assert!(!delegate(&DocumentPermissionsPolicy::default(), None));
        assert!(delegate(
            &DocumentPermissionsPolicy::default(),
            Some("cross-origin-isolated")
        ));
        assert!(!delegate(
            &policy("cross-origin-isolated=()"),
            Some("cross-origin-isolated *")
        ));
        let restricted =
            policy("cross-origin-isolated=(self)").intersect(&DocumentPermissionsPolicy::default());
        assert!(restricted.cross_origin_isolated_enabled());
        assert!(!delegate(&restricted, Some("cross-origin-isolated *")));
        // A header must allow both the parent and the child before the parent
        // can delegate; listing only the child cannot restore the permission.
        let child_only = policy("cross-origin-isolated=(\"https://child.test\")");
        assert!(!child_only.cross_origin_isolated_enabled());
        assert!(!delegate(&child_only, Some("cross-origin-isolated")));
        assert!(delegate(
            &policy("cross-origin-isolated=(self \"https://child.test\")"),
            Some("cross-origin-isolated")
        ));
        let inherited_denial = policy("cross-origin-isolated=()")
            .delegated_to_child(
                &parent.origin(),
                &child.origin(),
                &child.origin(),
                Some("cross-origin-isolated *"),
            )
            .intersect(&DocumentPermissionsPolicy::default());
        assert!(!inherited_denial.cross_origin_isolated_enabled());
    }
}
