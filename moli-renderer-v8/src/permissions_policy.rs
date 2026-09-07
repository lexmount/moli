use url::Url;

/// Policy-controlled features that currently have observable renderer behavior.
///
/// This intentionally stores the effective policy for one committed Document,
/// rather than the raw header/container allowlists. Extend the record when a
/// newly implemented feature needs enforcement at its API boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentPermissionsPolicy {
    gamepad: bool,
}

impl Default for DocumentPermissionsPolicy {
    fn default() -> Self {
        Self { gamepad: true }
    }
}

impl DocumentPermissionsPolicy {
    pub(crate) const fn gamepad_enabled(self) -> bool {
        self.gamepad
    }

    pub(crate) const fn intersect(self, other: Self) -> Self {
        Self {
            gamepad: self.gamepad && other.gamepad,
        }
    }

    pub(crate) fn from_navigation_response_headers(
        headers: &[(String, String)],
        document_url: &Url,
    ) -> Self {
        let mut policy = Self::default();
        for (_, value) in headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
        {
            for directive in value.split(',') {
                let Some((feature, allowlist)) = directive.split_once('=') else {
                    continue;
                };
                if feature.trim().eq_ignore_ascii_case("gamepad") {
                    policy.gamepad = response_allowlist_allows_document(allowlist, document_url);
                }
            }
        }
        policy
    }

    pub(crate) fn delegated_to_child(
        self,
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
        Self {
            gamepad: self.gamepad && gamepad,
        }
    }
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
    fn permissions_policy_header_takes_precedence_over_legacy_feature_policy() {
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[
                ("Permissions-Policy".to_owned(), "gamepad=*".to_owned()),
                ("Feature-Policy".to_owned(), "gamepad 'none'".to_owned()),
            ],
            &url("https://example.test/document"),
        );
        assert!(policy.gamepad_enabled());
    }

    #[test]
    fn permissions_policy_response_none_disables_recognized_features() {
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("permissions-policy".to_owned(), "gamepad=()".to_owned())],
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

        let parent_denied = DocumentPermissionsPolicy { gamepad: false };
        let delegated = parent_denied.delegated_to_child(
            &parent,
            &url("https://other.test/child"),
            false,
            Some("gamepad *"),
        );
        assert!(!delegated.gamepad_enabled());
    }
}
