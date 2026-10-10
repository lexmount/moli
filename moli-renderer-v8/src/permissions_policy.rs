use std::sync::Arc;
use url::Url;

/// Policy-controlled features that currently have observable renderer behavior.
///
/// Keep the declared tools allowlist alongside effective permissions: a
/// container cannot delegate tools to an origin excluded by its parent header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DocumentPermissionsPolicy {
    fullscreen: bool,
    gamepad: bool,
    midi: bool,
    payment: bool,
    payment_allowlists: Vec<(url::Origin, OriginAllowlist)>,
    tools: bool,
    tools_allowlist: Option<OriginAllowlist>,
    synchronous_xhr: bool,
    focus_without_user_activation: bool,
}

impl Default for DocumentPermissionsPolicy {
    fn default() -> Self {
        Self {
            fullscreen: true,
            gamepad: true,
            midi: true,
            payment: true,
            payment_allowlists: Vec::new(),
            tools: true,
            tools_allowlist: None,
            synchronous_xhr: true,
            focus_without_user_activation: true,
        }
    }
}

impl DocumentPermissionsPolicy {
    pub(crate) const fn fullscreen_enabled(&self) -> bool {
        self.fullscreen
    }

    pub(crate) const fn synchronous_xhr_enabled(&self) -> bool {
        self.synchronous_xhr
    }

    pub(crate) const fn gamepad_enabled(&self) -> bool {
        self.gamepad
    }

    pub(crate) const fn focus_without_user_activation_enabled(&self) -> bool {
        self.focus_without_user_activation
    }

    pub(crate) const fn midi_enabled(&self) -> bool {
        self.midi
    }

    pub(crate) const fn payment_enabled(&self) -> bool {
        self.payment
    }

    pub(crate) const fn tools_enabled(&self) -> bool {
        self.tools
    }

    pub(crate) fn intersect(&self, other: &Self) -> Self {
        Self {
            fullscreen: self.fullscreen && other.fullscreen,
            gamepad: self.gamepad && other.gamepad,
            midi: self.midi && other.midi,
            payment: self.payment && other.payment,
            payment_allowlists: self
                .payment_allowlists
                .iter()
                .chain(&other.payment_allowlists)
                .cloned()
                .collect(),
            tools: self.tools && other.tools,
            tools_allowlist: other.tools_allowlist.clone(),
            synchronous_xhr: self.synchronous_xhr && other.synchronous_xhr,
            focus_without_user_activation: self.focus_without_user_activation
                && other.focus_without_user_activation,
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
                let allowed = response_allowlist_allows_document(allowlist, document_url);
                if feature.trim().eq_ignore_ascii_case("fullscreen") {
                    policy.fullscreen = allowed;
                } else if feature.trim().eq_ignore_ascii_case("gamepad") {
                    policy.gamepad = allowed;
                } else if feature.trim().eq_ignore_ascii_case("midi") {
                    policy.midi = allowed;
                } else if feature.trim().eq_ignore_ascii_case("sync-xhr") {
                    policy.synchronous_xhr = allowed;
                } else if feature
                    .trim()
                    .eq_ignore_ascii_case("focus-without-user-activation")
                {
                    policy.focus_without_user_activation = allowed;
                }
                if feature.trim().eq_ignore_ascii_case("payment") {
                    policy.payment = allowed;
                    policy.payment_allowlists = response_origin_allowlist(allowlist)
                        .map(|list| (document_url.origin(), list))
                        .into_iter()
                        .collect();
                }
                if feature.trim().eq_ignore_ascii_case("tools") {
                    policy.tools_allowlist = response_origin_allowlist(allowlist);
                    policy.tools =
                        policy.tools_allows_origin(&document_url.origin(), &document_url.origin());
                }
            }
        }
        policy
    }

    pub(crate) fn delegated_to_child(
        &self,
        parent_origin: &url::Origin,
        child_origin: &url::Origin,
        source_origin: &url::Origin,
        allow_attribute: Option<&str>,
        allow_fullscreen: bool,
    ) -> Self {
        let same_origin = parent_origin == child_origin;
        let fullscreen = iframe_allow_feature(
            allow_attribute,
            "fullscreen",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(same_origin || allow_fullscreen);
        let synchronous_xhr = iframe_allow_feature(
            allow_attribute,
            "sync-xhr",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(true);
        let gamepad = iframe_allow_feature(
            allow_attribute,
            "gamepad",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(true);
        let midi = iframe_allow_feature(
            allow_attribute,
            "midi",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(same_origin);
        let payment = iframe_allow_feature(
            allow_attribute,
            "payment",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(same_origin);
        let tools = iframe_allow_feature(
            allow_attribute,
            "tools",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(same_origin);
        let focus_without_user_activation = iframe_allow_feature(
            allow_attribute,
            "focus-without-user-activation",
            parent_origin,
            child_origin,
            source_origin,
        )
        .unwrap_or(same_origin);
        Self {
            fullscreen: self.fullscreen && fullscreen,
            gamepad: self.gamepad && gamepad,
            midi: self.midi && midi,
            payment: self.payment && self.payment_allows_origin(child_origin) && payment,
            payment_allowlists: self.payment_allowlists.clone(),
            tools: self.tools
                && self.tools_allows_origin(parent_origin, parent_origin)
                && self.tools_allows_origin(child_origin, parent_origin)
                && tools,
            tools_allowlist: None,
            synchronous_xhr: self.synchronous_xhr && synchronous_xhr,
            focus_without_user_activation: self.focus_without_user_activation
                && focus_without_user_activation,
        }
    }

    pub(crate) fn for_document_origin(&self, origin: &url::Origin) -> Self {
        Self {
            tools: self.tools && self.tools_allows_origin(origin, origin),
            payment: self.payment && self.payment_allows_origin(origin),
            ..self.clone()
        }
    }

    fn payment_allows_origin(&self, origin: &url::Origin) -> bool {
        self.payment_allowlists.iter().all(|(anchor, allowed)| {
            allowed.matches_self && origin == anchor || allowed.origins.contains(origin)
        })
    }

    fn tools_allows_origin(&self, origin: &url::Origin, self_origin: &url::Origin) -> bool {
        self.tools_allowlist.as_ref().is_none_or(|allowed| {
            allowed.matches_self && origin == self_origin || allowed.origins.contains(origin)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OriginAllowlist {
    matches_self: bool,
    origins: Arc<[url::Origin]>,
}

fn response_origin_allowlist(value: &str) -> Option<OriginAllowlist> {
    let value = value.trim();
    if value == "*" {
        return None;
    }
    let Some(value) = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
    else {
        return Some(OriginAllowlist {
            matches_self: false,
            origins: Arc::from([]),
        });
    };
    let mut origins = Vec::new();
    let mut matches_self = false;
    for token in value.split_ascii_whitespace() {
        if token == "*" {
            return None;
        }
        if token.eq_ignore_ascii_case("self") {
            matches_self = true;
        } else if let Ok(url) = Url::parse(token.trim_matches('"'))
            && matches!(url.origin(), url::Origin::Tuple(..))
        {
            origins.push(url.origin());
        }
    }
    Some(OriginAllowlist {
        matches_self,
        origins: origins.into(),
    })
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
    parent_origin: &url::Origin,
    child_origin: &url::Origin,
    source_origin: &url::Origin,
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
            return Some(child_origin == source_origin);
        }
        if allowlist
            .iter()
            .any(|token| token.eq_ignore_ascii_case("'none'") || *token == "()")
        {
            return Some(false);
        }
        Some(allowlist.into_iter().any(|token| {
            token == "*"
                || (token.eq_ignore_ascii_case("'src'") && child_origin == source_origin)
                || (token.eq_ignore_ascii_case("'self'") && child_origin == parent_origin)
                || Url::parse(token.trim_matches(['\'', '"']))
                    .ok()
                    .is_some_and(|url| {
                        matches!(url.origin(), url::Origin::Tuple(..))
                            && &url.origin() == child_origin
                    })
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

    impl DocumentPermissionsPolicy {
        fn delegated_to_child_urls(
            &self,
            parent: &Url,
            child: &Url,
            inherits: bool,
            allow: Option<&str>,
        ) -> Self {
            self.delegated_to_child_urls_with_fullscreen(parent, child, inherits, allow, false)
        }

        fn delegated_to_child_urls_with_fullscreen(
            &self,
            parent: &Url,
            child: &Url,
            inherits: bool,
            allow: Option<&str>,
            allow_fullscreen: bool,
        ) -> Self {
            let parent_origin = parent.origin();
            let child_origin = if inherits {
                parent_origin.clone()
            } else {
                child.origin()
            };
            self.delegated_to_child(
                &parent_origin,
                &child_origin,
                &child_origin,
                allow,
                allow_fullscreen,
            )
        }
    }

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn tools_headers_bind_self_to_the_sandboxed_document_origin() {
        let document = url("https://parent.test/child");
        for (allowlist, expected) in [
            ("tools=(self)", true),
            ("tools=*", true),
            ("tools=(\"https://parent.test\")", false),
        ] {
            let policy =
                crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                    &[
                        ("permissions-policy".into(), allowlist.as_bytes().to_vec()),
                        (
                            "content-security-policy".into(),
                            b"sandbox allow-scripts".to_vec(),
                        ),
                    ],
                    &document,
                );
            assert!(policy.sandbox.forces_opaque_origin);
            assert_eq!(
                policy.permissions_policy.tools_enabled(),
                expected,
                "{allowlist}"
            );
        }
    }

    #[test]
    fn tools_policy_defaults_to_self_and_requires_both_parent_and_container_permission() {
        let parent = url("https://parent.test/");
        let same = url("https://parent.test/child");
        let other = url("https://other.test/");
        let allowed = DocumentPermissionsPolicy::default();
        assert!(
            allowed
                .delegated_to_child_urls(&parent, &same, false, None)
                .tools_enabled()
        );
        assert!(
            !allowed
                .delegated_to_child_urls(&parent, &other, false, None)
                .tools_enabled()
        );
        assert!(
            allowed
                .delegated_to_child_urls(&parent, &other, false, Some("tools *"))
                .tools_enabled()
        );
        assert!(
            !allowed
                .delegated_to_child_urls(&parent, &same, false, Some("tools 'none'"))
                .tools_enabled()
        );
        let denied = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"tools=()".to_vec())],
            &parent,
        );
        assert!(
            !denied
                .delegated_to_child_urls(&parent, &other, false, Some("tools *"))
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
                    .delegated_to_child_urls(&parent, &url(child), false, Some("tools *"))
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
                .delegated_to_child_urls(&parent, &url("about:blank"), true, None)
                .tools_enabled()
        );
        let inherited = DocumentPermissionsPolicy::default().delegated_to_child_urls(
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
                .delegated_to_child_urls(
                    &url("https://child.test/"),
                    &parent,
                    false,
                    Some("tools *")
                )
                .tools_enabled()
        );
    }

    #[test]
    fn permissions_policy_header_takes_precedence_over_legacy_feature_policy() {
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[
                (
                    "Permissions-Policy".to_owned(),
                    b"gamepad=*, sync-xhr=*".to_vec(),
                ),
                (
                    "Feature-Policy".to_owned(),
                    b"gamepad 'none'; sync-xhr 'none'".to_vec(),
                ),
            ],
            &url("https://example.test/document"),
        );
        assert!(policy.gamepad_enabled());
        assert!(policy.synchronous_xhr_enabled());
    }

    #[test]
    fn midi_policy_preserves_header_denial_and_same_origin_default() {
        let parent = url("https://parent.test/page");
        let same = url("https://parent.test/child");
        let cross = url("https://child.test/");
        let policy = DocumentPermissionsPolicy::default();
        assert!(policy.midi_enabled());
        assert!(
            policy
                .delegated_to_child_urls(&parent, &same, false, None)
                .midi_enabled()
        );
        assert!(
            !policy
                .delegated_to_child_urls(&parent, &cross, false, None)
                .midi_enabled()
        );
        assert!(
            policy
                .delegated_to_child_urls(&parent, &cross, false, Some("midi *"))
                .midi_enabled()
        );
        assert!(
            !policy
                .delegated_to_child_urls(&parent, &same, false, Some("midi 'none'"))
                .midi_enabled()
        );
        let denied = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"midi=()".to_vec())],
            &parent,
        );
        assert!(!denied.midi_enabled());
        assert!(!denied.intersect(&policy).midi_enabled());
        assert!(
            !denied
                .delegated_to_child_urls(&parent, &same, false, Some("midi *"))
                .midi_enabled()
        );
    }

    #[test]
    fn permissions_policy_response_none_disables_recognized_features() {
        let policy = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[(
                "permissions-policy".to_owned(),
                b"fullscreen=(), gamepad=(), sync-xhr=(), focus-without-user-activation=()"
                    .to_vec(),
            )],
            &url("https://example.test/document"),
        );
        assert!(!policy.fullscreen_enabled());
        assert!(!policy.gamepad_enabled());
        assert!(!policy.synchronous_xhr_enabled());
        assert!(!policy.focus_without_user_activation_enabled());
    }

    #[test]
    fn iframe_policy_uses_feature_defaults_and_explicit_delegation() {
        let parent = url("https://parent.test/page");
        let same_origin = url("https://parent.test/child");
        let cross_origin = url("data:text/html,child");
        let policy = DocumentPermissionsPolicy::default();

        let same = policy.delegated_to_child_urls(&parent, &same_origin, false, None);
        assert!(same.fullscreen_enabled());
        assert!(same.gamepad_enabled());
        assert!(same.synchronous_xhr_enabled());
        assert!(same.focus_without_user_activation_enabled());

        let denied = policy.delegated_to_child_urls_with_fullscreen(
            &parent,
            &cross_origin,
            false,
            None,
            false,
        );
        assert!(!denied.fullscreen_enabled());
        assert!(denied.gamepad_enabled());
        assert!(denied.synchronous_xhr_enabled());
        assert!(!denied.focus_without_user_activation_enabled());

        let delegated = policy.delegated_to_child_urls_with_fullscreen(
            &parent,
            &cross_origin,
            false,
            Some("payment; fullscreen; gamepad *; focus-without-user-activation *"),
            false,
        );
        assert!(delegated.fullscreen_enabled());
        assert!(delegated.gamepad_enabled());
        assert!(delegated.synchronous_xhr_enabled());
        assert!(delegated.focus_without_user_activation_enabled());
    }

    #[test]
    fn iframe_none_and_parent_policy_cannot_be_overridden() {
        let parent = url("https://parent.test/page");
        let child = url("https://parent.test/child");
        let denied = DocumentPermissionsPolicy::default().delegated_to_child_urls_with_fullscreen(
            &parent,
            &child,
            false,
            Some("gamepad 'none'; sync-xhr 'none'; focus-without-user-activation 'none'"),
            false,
        );
        assert!(!denied.gamepad_enabled());
        assert!(!denied.synchronous_xhr_enabled());
        assert!(!denied.focus_without_user_activation_enabled());

        let parent_denied = DocumentPermissionsPolicy {
            fullscreen: false,
            gamepad: false,
            synchronous_xhr: false,
            focus_without_user_activation: false,
            ..Default::default()
        };
        let delegated = parent_denied.delegated_to_child_urls_with_fullscreen(
            &parent,
            &url("https://other.test/child"),
            false,
            Some("fullscreen *; gamepad *; sync-xhr *; focus-without-user-activation *"),
            true,
        );
        assert!(!delegated.fullscreen_enabled());
        assert!(!delegated.gamepad_enabled());
        assert!(!delegated.synchronous_xhr_enabled());
        assert!(!delegated.focus_without_user_activation_enabled());
    }
    #[test]
    fn payment_defaults_to_self_and_requires_cross_origin_delegation() {
        let parent = url("https://parent.test/");
        let same = url("https://parent.test/frame");
        let other = url("https://other.test/frame");
        let policy = DocumentPermissionsPolicy::default();
        assert!(policy.payment_enabled());
        assert!(
            policy
                .delegated_to_child_urls(&parent, &same, false, None)
                .payment_enabled()
        );
        assert!(
            !policy
                .delegated_to_child_urls(&parent, &other, false, None)
                .payment_enabled()
        );
        assert!(
            policy
                .delegated_to_child_urls(&parent, &other, false, Some("payment"))
                .payment_enabled()
        );
        assert!(
            !policy
                .delegated_to_child_urls(&parent, &same, false, Some("payment 'none'"))
                .payment_enabled()
        );
    }

    #[test]
    fn payment_headers_restrict_delegation_and_keep_their_origin_after_intersection() {
        let parent = url("https://parent.test/");
        let other = url("https://other.test/frame");
        let blocked = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"payment=()".to_vec())],
            &parent,
        );
        assert!(!blocked.payment_enabled());
        assert!(
            !blocked
                .delegated_to_child_urls(&parent, &other, false, Some("payment *"))
                .payment_enabled()
        );
        let only_self = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), b"payment=(self)".to_vec())],
            &parent,
        );
        let combined = only_self.intersect(&DocumentPermissionsPolicy::default());
        assert!(
            !combined
                .delegated_to_child_urls(&parent, &other, false, Some("payment *"))
                .payment_enabled()
        );
        let both = DocumentPermissionsPolicy::from_navigation_response_headers(
            &[(
                "Permissions-Policy".into(),
                b"payment=(self \"https://other.test\")".to_vec(),
            )],
            &parent,
        );
        assert!(
            both.delegated_to_child_urls(&parent, &other, false, Some("payment"))
                .payment_enabled()
        );
        assert!(
            !both
                .for_document_origin(&url("https://unlisted.test/").origin())
                .payment_enabled()
        );
    }
}
