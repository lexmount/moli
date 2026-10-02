use std::sync::Arc;
use url::Url;

/// Policy-controlled features that currently have observable renderer behavior.
///
/// Keep the declared tools allowlist alongside effective permissions: a
/// container cannot delegate tools to an origin excluded by its parent header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DocumentPermissionsPolicy {
    gamepad: bool,
    tools: bool,
    tools_allowed_origins: Option<Arc<[url::Origin]>>,
}

impl Default for DocumentPermissionsPolicy {
    fn default() -> Self {
        Self {
            gamepad: true,
            tools: true,
            tools_allowed_origins: None,
        }
    }
}

impl DocumentPermissionsPolicy {
    pub(crate) const fn gamepad_enabled(&self) -> bool {
        self.gamepad
    }

    pub(crate) const fn tools_enabled(&self) -> bool {
        self.tools
    }

    pub(crate) fn intersect(&self, other: &Self) -> Self {
        Self {
            gamepad: self.gamepad && other.gamepad,
            tools: self.tools && other.tools,
            tools_allowed_origins: other.tools_allowed_origins.clone(),
        }
    }

    pub(crate) fn from_navigation_response_headers(
        headers: &[(String, Vec<u8>)],
        document_url: &Url,
    ) -> Self {
        let mut policy = Self::default();
        for (_, value) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
        {
            for directive in moli_fetch::decode_header_value(value).split(',') {
                let Some((feature, allowlist)) = directive.split_once('=') else {
                    continue;
                };
                if feature.trim().eq_ignore_ascii_case("gamepad") {
                    policy.gamepad = response_allowlist_allows_document(allowlist, document_url);
                }
                if feature.trim().eq_ignore_ascii_case("tools") {
                    policy.tools_allowed_origins =
                        response_tools_allowlist(allowlist, document_url);
                    policy.tools = policy.tools_allows_origin(&document_url.origin());
                }
            }
        }
        policy
    }

    pub(crate) fn delegated_to_child(
        &self,
        parent_url: &Url,
        child_url: &Url,
        child_inherits_origin: bool,
        allow_attribute: Option<&str>,
    ) -> Self {
        let same_origin = child_inherits_origin || moli_url::same_origin(parent_url, child_url);
        let gamepad = iframe_allow_feature(
            allow_attribute,
            "gamepad",
            parent_url,
            child_url,
            same_origin,
        )
        .unwrap_or(true);
        let tools =
            iframe_allow_feature(allow_attribute, "tools", parent_url, child_url, same_origin)
                .unwrap_or(same_origin);
        Self {
            gamepad: self.gamepad && gamepad,
            // Effective tools already checks the parent's own origin. An
            // inherited-origin child uses that same origin, including about:blank.
            tools: self.tools
                && (child_inherits_origin || self.tools_allows_origin(&child_url.origin()))
                && tools,
            tools_allowed_origins: None,
        }
    }
    fn tools_allows_origin(&self, origin: &url::Origin) -> bool {
        self.tools_allowed_origins
            .as_ref()
            .is_none_or(|allowed| allowed.contains(origin))
    }
}

fn response_tools_allowlist(value: &str, document_url: &Url) -> Option<Arc<[url::Origin]>> {
    let value = value.trim();
    if value == "*" {
        return None;
    }
    let Some(value) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Some(Arc::from([]));
    };
    let mut origins = Vec::new();
    for token in value.split_ascii_whitespace() {
        if token == "*" {
            return None;
        }
        if token.eq_ignore_ascii_case("self") {
            origins.push(document_url.origin());
        } else if let Ok(url) = Url::parse(token.trim_matches('"'))
            && matches!(url.origin(), url::Origin::Tuple(..))
        {
            origins.push(url.origin());
        }
    }
    Some(origins.into())
}

fn response_allowlist_allows_document(value: &str, document_url: &Url) -> bool {
    let value = value.trim();
    if value == "*" {
        return true;
    }
    let Some(value) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    value.split_ascii_whitespace().any(|token| {
        token.eq_ignore_ascii_case("self")
            || token == "*"
            || token_origin_matches(token, document_url)
    })
}

fn iframe_allow_feature(
    allow_attribute: Option<&str>,
    feature: &str,
    parent_url: &Url,
    child_url: &Url,
    same_origin: bool,
) -> Option<bool> {
    let allow_attribute = allow_attribute?;
    allow_attribute.split(';').find_map(|directive| {
        let mut tokens = directive.split_ascii_whitespace();
        let name = tokens.next()?;
        if !name.eq_ignore_ascii_case(feature) {
            return None;
        }
        let allowlist = tokens.collect::<Vec<_>>();
        if allowlist.is_empty() {
            return Some(true);
        }
        if allowlist
            .iter()
            .any(|token| token.eq_ignore_ascii_case("'none'") || *token == "()")
        {
            return Some(false);
        }
        Some(allowlist.into_iter().any(|token| {
            token == "*"
                || token.eq_ignore_ascii_case("'src'")
                || (token.eq_ignore_ascii_case("'self'") && same_origin)
                || token_origin_matches(token, child_url)
                || token_origin_matches(token, parent_url) && same_origin
        }))
    })
}

fn token_origin_matches(token: &str, url: &Url) -> bool {
    let token = token.trim_matches(['\'', '"']);
    Url::parse(token)
        .ok()
        .is_some_and(|origin| moli_url::same_origin(&origin, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn tools_policy_defaults_to_self_and_requires_both_parent_and_container_permission() {
        let parent = url("https://parent.test/");
        let same = url("https://parent.test/child");
        let other = url("https://other.test/");
        let allowed = DocumentPermissionsPolicy::default();
        assert!(
            allowed
                .delegated_to_child(&parent, &same, false, None)
                .tools_enabled()
        );
        assert!(
            !allowed
                .delegated_to_child(&parent, &other, false, None)
                .tools_enabled()
        );
        assert!(
            allowed
                .delegated_to_child(&parent, &other, false, Some("tools *"))
                .tools_enabled()
        );
        assert!(
            !allowed
                .delegated_to_child(&parent, &same, false, Some("tools 'none'"))
                .tools_enabled()
        );
        let denied = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"tools=()".to_vec())],
            &parent,
        );
        assert!(
            !denied
                .delegated_to_child(&parent, &other, false, Some("tools *"))
                .tools_enabled()
        );
        assert!(denied.gamepad_enabled());
    }

    #[test]
    fn tools_response_allowlist_bounds_container_delegation() {
        let parent = url("https://parent.test/");
        for (header, child, allowed) in [
            ("tools=(self)", "https://parent.test/child", true),
            ("tools=(self)", "https://other.test/child", false),
            ("tools=()", "https://parent.test/child", false),
            ("tools=*", "https://other.test/child", true),
            (
                "tools=(self \"https://allowed.test\")",
                "https://allowed.test/child",
                true,
            ),
            (
                "tools=(self \"https://allowed.test\")",
                "https://other.test/child",
                false,
            ),
            (
                "tools=(\"https://allowed.test\")",
                "https://allowed.test/child",
                false,
            ),
        ] {
            let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
                &[("Permissions-Policy".into(), header.as_bytes().to_vec())],
                &parent,
            );
            assert_eq!(
                policy
                    .delegated_to_child(&parent, &url(child), false, Some("tools *"))
                    .tools_enabled(),
                allowed,
                "{header} -> {child}"
            );
        }
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"tools=(self)".to_vec())],
            &parent,
        );
        assert!(
            policy
                .delegated_to_child(&parent, &url("about:blank"), true, None)
                .tools_enabled()
        );
        let inherited = DocumentPermissionsPolicy::default().delegated_to_child(
            &parent,
            &url("https://child.test/"),
            false,
            Some("tools *"),
        );
        let child_header = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"tools=(self)".to_vec())],
            &url("https://child.test/"),
        );
        let child = inherited.intersect(&child_header);
        assert!(
            !child
                .delegated_to_child(&url("https://child.test/"), &parent, false, Some("tools *"))
                .tools_enabled()
        );
    }

    #[test]
    fn permissions_policy_header_takes_precedence_over_legacy_feature_policy() {
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[
                ("Permissions-Policy".to_owned(), b"gamepad=*".to_vec()),
                ("Feature-Policy".to_owned(), b"gamepad 'none'".to_vec()),
            ],
            &url("https://example.test/document"),
        );
        assert!(policy.gamepad_enabled());
    }

    #[test]
    fn permissions_policy_response_none_disables_recognized_features() {
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("permissions-policy".to_owned(), b"gamepad=()".to_vec())],
            &url("https://example.test/document"),
        );
        assert!(!policy.gamepad_enabled());
    }

    #[test]
    fn iframe_gamepad_policy_uses_default_wildcard_allowlist_and_explicit_delegation() {
        let parent = url("https://parent.test/page");
        let same_origin = url("https://parent.test/child");
        let cross_origin = url("data:text/html,child");
        let policy = DocumentPermissionsPolicy::default();

        let same = policy.delegated_to_child(&parent, &same_origin, false, None);
        assert!(same.gamepad_enabled());

        let cross = policy.delegated_to_child(&parent, &cross_origin, false, None);
        assert!(cross.gamepad_enabled());

        let delegated =
            policy.delegated_to_child(&parent, &cross_origin, false, Some("payment; gamepad *"));
        assert!(delegated.gamepad_enabled());
    }

    #[test]
    fn iframe_none_and_parent_policy_cannot_be_overridden() {
        let parent = url("https://parent.test/page");
        let child = url("https://parent.test/child");
        let denied = DocumentPermissionsPolicy::default().delegated_to_child(
            &parent,
            &child,
            false,
            Some("gamepad 'none'"),
        );
        assert!(!denied.gamepad_enabled());

        let parent_denied = DocumentPermissionsPolicy {
            gamepad: false,
            ..Default::default()
        };
        let delegated = parent_denied.delegated_to_child(
            &parent,
            &url("https://other.test/child"),
            false,
            Some("gamepad *"),
        );
        assert!(!delegated.gamepad_enabled());
    }
}
