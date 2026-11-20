use super::{ToolsAllowlist, response_tools_allowlist};
use url::{Origin, Url};

/// A Document's own declarations remain separate from its inherited decision.
/// A child receives the evaluated decision, rather than its parent's header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DocumentFeaturePolicy {
    inherited: bool,
    origin: Option<Origin>,
    declarations: Vec<ToolsAllowlist>,
}

impl Default for DocumentFeaturePolicy {
    fn default() -> Self {
        Self {
            inherited: true,
            origin: None,
            declarations: Vec::new(),
        }
    }
}

impl DocumentFeaturePolicy {
    pub(super) fn enabled(&self) -> bool {
        self.origin
            .as_ref()
            .map_or(self.inherited, |origin| self.allows_origin(origin, origin))
    }

    fn allows_origin(&self, declaring_origin: &Origin, origin: &Origin) -> bool {
        self.inherited
            && self.declarations.iter().all(|allowed| {
                allowed.matches_self && origin == declaring_origin
                    || allowed.origins.contains(origin)
            })
    }

    pub(super) fn declare(&mut self, allowlist: &str, url: &Url) {
        self.declarations = response_tools_allowlist(allowlist).into_iter().collect();
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
            origin: other.origin.clone().or_else(|| self.origin.clone()),
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
        container_allows: bool,
    ) -> Self {
        Self {
            inherited: self.allows_origin(parent, parent)
                && self.allows_origin(parent, child)
                && container_allows,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_constrain_parent_and_child_before_container_delegation() {
        let url = Url::parse("https://parent.test/").unwrap();
        let parent = url.origin();
        let other = Url::parse("https://other.test/").unwrap().origin();
        let opaque = Origin::new_opaque();
        for (header, same, cross, data) in [
            ("()", false, false, false),
            ("(self)", true, false, false),
            ("*", true, true, true),
            ("(self \"https://other.test\")", true, true, false),
            ("(\"https://other.test\")", false, false, false),
        ] {
            let mut policy = DocumentFeaturePolicy::default();
            policy.declare(header, &url);
            for (child, expected) in [(&parent, same), (&other, cross), (&opaque, data)] {
                assert_eq!(
                    policy.delegated_to_child(&parent, child, true).enabled(),
                    expected,
                    "{header}: {child:?}"
                );
                assert!(!policy.delegated_to_child(&parent, child, false).enabled());
            }
        }
    }

    #[test]
    fn self_refers_to_its_document_and_does_not_merge_distinct_opaque_origins() {
        let url = Url::parse("https://parent.test/").unwrap();
        let parent = Origin::new_opaque();
        let different = Origin::new_opaque();
        let mut declared = DocumentFeaturePolicy::default();
        declared.declare("(self)", &url);
        let policy = declared.for_document_origin(&parent);
        assert!(policy.enabled());
        assert!(policy.delegated_to_child(&parent, &parent, true).enabled());
        assert!(
            !policy
                .delegated_to_child(&parent, &different, true)
                .enabled()
        );
        assert_eq!(
            DocumentFeaturePolicy::default().for_document_origin(&parent),
            DocumentFeaturePolicy::default()
        );
        let denied = policy.delegated_to_child(&parent, &different, true);
        assert!(denied.declarations.is_empty());
        assert!(
            !denied
                .intersect(&DocumentFeaturePolicy::default())
                .enabled()
        );
    }
}
