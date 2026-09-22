use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use moli_browser_profile::BrowserIdentityProfile;
use moli_url::{is_potentially_trustworthy_url, origin_ascii_serialization};
use parking_lot::Mutex;
use url::Url;

use crate::{FetchConfig, Request};

pub(crate) type SharedClientHintPreferences = Arc<Mutex<ClientHintPreferences>>;
pub(crate) type SharedNavigationClientHintRestarts = Arc<Mutex<BTreeSet<String>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientHintResponseAction {
    Continue,
    RestartNavigation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ClientHint {
    Ua,
    UaArch,
    UaBitness,
    UaFormFactors,
    UaFullVersion,
    UaFullVersionList,
    UaMobile,
    UaModel,
    UaPlatform,
    UaPlatformVersion,
    UaWow64,
}

impl ClientHint {
    fn from_header_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "sec-ch-ua" => Some(Self::Ua),
            "sec-ch-ua-arch" => Some(Self::UaArch),
            "sec-ch-ua-bitness" => Some(Self::UaBitness),
            "sec-ch-ua-form-factors" => Some(Self::UaFormFactors),
            "sec-ch-ua-full-version" => Some(Self::UaFullVersion),
            "sec-ch-ua-full-version-list" => Some(Self::UaFullVersionList),
            "sec-ch-ua-mobile" => Some(Self::UaMobile),
            "sec-ch-ua-model" => Some(Self::UaModel),
            "sec-ch-ua-platform" => Some(Self::UaPlatform),
            "sec-ch-ua-platform-version" => Some(Self::UaPlatformVersion),
            "sec-ch-ua-wow64" => Some(Self::UaWow64),
            _ => None,
        }
    }

    fn header_name(self) -> &'static str {
        match self {
            Self::Ua => "Sec-CH-UA",
            Self::UaArch => "Sec-CH-UA-Arch",
            Self::UaBitness => "Sec-CH-UA-Bitness",
            Self::UaFormFactors => "Sec-CH-UA-Form-Factors",
            Self::UaFullVersion => "Sec-CH-UA-Full-Version",
            Self::UaFullVersionList => "Sec-CH-UA-Full-Version-List",
            Self::UaMobile => "Sec-CH-UA-Mobile",
            Self::UaModel => "Sec-CH-UA-Model",
            Self::UaPlatform => "Sec-CH-UA-Platform",
            Self::UaPlatformVersion => "Sec-CH-UA-Platform-Version",
            Self::UaWow64 => "Sec-CH-UA-WoW64",
        }
    }

    fn value(self, identity: &BrowserIdentityProfile) -> Option<String> {
        if !identity.has_user_agent_metadata() {
            return None;
        }
        match self {
            Self::Ua => identity.sec_ch_ua_value(),
            Self::UaArch => Some(quoted_header_value(identity.architecture())),
            Self::UaBitness => Some(quoted_header_value(identity.bitness())),
            Self::UaFormFactors => Some(
                identity
                    .form_factors()
                    .iter()
                    .map(|value| quoted_header_value(value))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::UaFullVersion => (!identity.full_version().is_empty())
                .then(|| quoted_header_value(identity.full_version())),
            Self::UaFullVersionList => identity.sec_ch_ua_full_version_list_value(),
            Self::UaMobile => Some(if identity.mobile() { "?1" } else { "?0" }.to_owned()),
            Self::UaModel => Some(quoted_header_value(identity.model())),
            Self::UaPlatform => Some(quoted_header_value(identity.platform())),
            Self::UaPlatformVersion => Some(quoted_header_value(identity.platform_version())),
            Self::UaWow64 => Some(if identity.wow64() { "?1" } else { "?0" }.to_owned()),
        }
    }

    fn is_low_entropy(self) -> bool {
        matches!(self, Self::Ua | Self::UaMobile | Self::UaPlatform)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ClientHintPreferences {
    enabled_by_origin: BTreeMap<String, BTreeSet<ClientHint>>,
}

impl ClientHintPreferences {
    fn enabled_for_url(&self, url: &Url) -> BTreeSet<ClientHint> {
        self.enabled_by_origin
            .get(&origin_ascii_serialization(url))
            .cloned()
            .unwrap_or_default()
    }

    fn replace_for_origin(&mut self, origin: String, enabled: BTreeSet<ClientHint>) {
        if enabled.is_empty() {
            self.enabled_by_origin.remove(&origin);
        } else {
            self.enabled_by_origin.insert(origin, enabled);
        }
    }

    fn clear_origin(&mut self, origin: &str) {
        self.enabled_by_origin.remove(origin);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ClientHintResponsePolicy {
    preferences: SharedClientHintPreferences,
    navigation_restarts: SharedNavigationClientHintRestarts,
    sent_hints: BTreeSet<ClientHint>,
    is_top_level_navigation: bool,
}

impl ClientHintResponsePolicy {
    pub(crate) fn observe_response(
        &self,
        response_url: &Url,
        response_headers: &[(String, Vec<u8>)],
    ) -> ClientHintResponseAction {
        // Chromium only persists Accept-CH from a main-frame navigation.
        // Subresource and child-frame responses must not mutate browser-context
        // preferences for later requests.
        if !self.is_top_level_navigation || !is_potentially_trustworthy_url(response_url) {
            return ClientHintResponseAction::Continue;
        }

        let origin = origin_ascii_serialization(response_url);
        let accept_ch = parse_client_hint_header(response_headers, "accept-ch");
        let clears_client_hints = clear_site_data_clears_client_hints(response_headers);
        let Some(accepted_hints) = accept_ch else {
            if clears_client_hints {
                self.preferences.lock().clear_origin(&origin);
            }
            return ClientHintResponseAction::Continue;
        };
        let critical_hints = parse_client_hint_header(response_headers, "critical-ch")
            .unwrap_or_default()
            .intersection(&accepted_hints)
            .copied()
            .collect::<BTreeSet<_>>();
        let critical_hint_missing = critical_hints
            .iter()
            .any(|hint| !self.sent_hints.contains(hint));

        let mut preferences = self.preferences.lock();
        if clears_client_hints {
            preferences.clear_origin(&origin);
        }
        preferences.replace_for_origin(origin.clone(), accepted_hints);
        drop(preferences);

        if !critical_hint_missing {
            return ClientHintResponseAction::Continue;
        }

        let mut restarted = self.navigation_restarts.lock();
        if restarted.insert(origin) {
            ClientHintResponseAction::RestartNavigation
        } else {
            ClientHintResponseAction::Continue
        }
    }
}

pub(crate) struct PreparedClientHintRequest {
    pub(crate) request: Request,
    pub(crate) response_policy: ClientHintResponsePolicy,
}

pub(crate) fn prepare_client_hint_request(
    preferences: &SharedClientHintPreferences,
    navigation_restarts: &SharedNavigationClientHintRestarts,
    config: &FetchConfig,
    request: &Request,
    request_url: &Url,
) -> PreparedClientHintRequest {
    let identity = request.browser_identity(config);
    let browser_request =
        request.is_navigation_request() || request.browser_request_metadata().is_some();
    let mut effective_request = request.clone();
    let mut sent_hints = configured_client_hint_names(config, request);

    if browser_request {
        sent_hints.extend(
            [ClientHint::Ua, ClientHint::UaMobile, ClientHint::UaPlatform]
                .into_iter()
                .filter(|hint| hint.value(identity).is_some()),
        );
    }

    if browser_request && is_potentially_trustworthy_url(request_url) {
        let enabled_hints = preferences.lock().enabled_for_url(request_url);
        for hint in enabled_hints {
            if hint.is_low_entropy() || sent_hints.contains(&hint) {
                continue;
            }
            let Some(value) = hint.value(identity) else {
                continue;
            };
            effective_request
                .request_headers
                .push((hint.header_name().to_owned(), value.into_bytes()));
            sent_hints.insert(hint);
        }
    }

    PreparedClientHintRequest {
        request: effective_request,
        response_policy: ClientHintResponsePolicy {
            preferences: Arc::clone(preferences),
            navigation_restarts: Arc::clone(navigation_restarts),
            sent_hints,
            is_top_level_navigation: request.is_top_level_navigation_request(),
        },
    }
}

fn quoted_header_value(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        if matches!(character, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

fn configured_client_hint_names(config: &FetchConfig, request: &Request) -> BTreeSet<ClientHint> {
    config
        .default_request_headers()
        .iter()
        .map(|(name, _)| name)
        .chain(request.request_headers.iter().map(|(name, _)| name))
        .filter_map(|name| ClientHint::from_header_name(name))
        .collect()
}

fn parse_client_hint_header(
    headers: &[(String, Vec<u8>)],
    header_name: &str,
) -> Option<BTreeSet<ClientHint>> {
    let values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(header_name))
        .map(|(_, value)| crate::decode_header_value(value))
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .flat_map(|value| value.split(','))
            .filter_map(ClientHint::from_header_name)
            .collect(),
    )
}

fn clear_site_data_clears_client_hints(headers: &[(String, Vec<u8>)]) -> bool {
    let headers = crate::headers_to_byte_strings(headers);
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("clear-site-data"))
        .flat_map(|(_, value)| value.split(','))
        .map(|directive| directive.trim().trim_matches('"'))
        .any(|directive| directive.eq_ignore_ascii_case("clientHints") || directive == "*")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_preferences() -> SharedClientHintPreferences {
        Arc::new(Mutex::new(ClientHintPreferences::default()))
    }

    fn shared_restarts() -> SharedNavigationClientHintRestarts {
        Arc::new(Mutex::new(BTreeSet::new()))
    }

    #[test]
    fn quoted_header_value_escapes_structured_header_string_delimiters() {
        assert_eq!(quoted_header_value("x86\"\\64"), "\"x86\\\"\\\\64\"");
    }

    #[test]
    fn critical_hints_restart_once_and_are_added_to_the_next_request() {
        let config = FetchConfig::default();
        let request = Request::get("https://example.test/").unwrap();
        let preferences = shared_preferences();
        let restarts = shared_restarts();
        let first =
            prepare_client_hint_request(&preferences, &restarts, &config, &request, &request.url);
        let headers: Vec<(String, Vec<u8>)> = vec![
            (
                "Accept-CH".to_owned(),
                b"Sec-CH-UA-Arch, Sec-CH-UA-Full-Version-List".to_vec(),
            ),
            (
                "Critical-CH".to_owned(),
                b"Sec-CH-UA-Arch, Sec-CH-UA-Full-Version-List".to_vec(),
            ),
        ];

        assert_eq!(
            first
                .response_policy
                .observe_response(&request.url, &headers),
            ClientHintResponseAction::RestartNavigation
        );
        let second =
            prepare_client_hint_request(&preferences, &restarts, &config, &request, &request.url);
        assert!(second.request.request_headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("sec-ch-ua-arch") && value == b"\"x86\""
        }));
        assert!(second.request.request_headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("sec-ch-ua-full-version-list")
                && value
                    .windows(b"145.0.0.0".len())
                    .any(|part| part == b"145.0.0.0")
        }));
        assert_eq!(
            second
                .response_policy
                .observe_response(&request.url, &headers),
            ClientHintResponseAction::Continue
        );
    }

    #[test]
    fn preferences_are_origin_scoped_and_clear_site_data_removes_them() {
        let config = FetchConfig::default();
        let request = Request::get("https://example.test/").unwrap();
        let preferences = shared_preferences();
        let restarts = shared_restarts();
        let first =
            prepare_client_hint_request(&preferences, &restarts, &config, &request, &request.url);
        assert_eq!(
            first.response_policy.observe_response(
                &request.url,
                &[("Accept-CH".to_owned(), b"Sec-CH-UA-Arch".to_vec())],
            ),
            ClientHintResponseAction::Continue
        );

        let other = Request::get("https://other.test/").unwrap();
        let other_prepared = prepare_client_hint_request(
            &preferences,
            &shared_restarts(),
            &config,
            &other,
            &other.url,
        );
        assert!(other_prepared.request.request_headers.is_empty());

        let clear = prepare_client_hint_request(
            &preferences,
            &shared_restarts(),
            &config,
            &request,
            &request.url,
        );
        clear.response_policy.observe_response(
            &request.url,
            &[("Clear-Site-Data".to_owned(), b"\"clientHints\"".to_vec())],
        );
        let after_clear = prepare_client_hint_request(
            &preferences,
            &shared_restarts(),
            &config,
            &request,
            &request.url,
        );
        assert!(after_clear.request.request_headers.is_empty());
    }

    #[test]
    fn subframe_navigation_response_does_not_persist_high_entropy_hints() {
        let config = FetchConfig::default();
        let request = Request::new("GET", "https://frame.test/", None, Vec::new())
            .unwrap()
            .with_subframe_navigation_cookie_context();
        let preferences = shared_preferences();
        let restarts = shared_restarts();
        let prepared =
            prepare_client_hint_request(&preferences, &restarts, &config, &request, &request.url);

        assert_eq!(
            prepared.response_policy.observe_response(
                &request.url,
                &[("Accept-CH".to_owned(), b"Sec-CH-UA-Arch".to_vec())],
            ),
            ClientHintResponseAction::Continue
        );

        let next =
            prepare_client_hint_request(&preferences, &restarts, &config, &request, &request.url);
        assert!(
            !next
                .request
                .request_headers
                .iter()
                .any(|(name, _)| { name.eq_ignore_ascii_case("sec-ch-ua-arch") })
        );
    }

    #[test]
    fn high_entropy_values_are_serialized_from_the_canonical_identity() {
        let identity = BrowserIdentityProfile::new(
            "Mozilla/5.0 Chrome/146.1.2.3 Safari/537.36",
            "fr-CA,fr;q=0.8",
        );

        assert_eq!(
            ClientHint::Ua.value(&identity).as_deref(),
            Some("\"Chromium\";v=\"146\", \"Not-A.Brand\";v=\"24\", \"Google Chrome\";v=\"146\"")
        );
        assert_eq!(
            ClientHint::UaFullVersionList.value(&identity).as_deref(),
            Some(
                "\"Chromium\";v=\"146.1.2.3\", \"Not-A.Brand\";v=\"24.0.0.0\", \"Google Chrome\";v=\"146.1.2.3\""
            )
        );
        assert_eq!(
            ClientHint::UaFullVersion.value(&identity).as_deref(),
            Some("\"146.1.2.3\"")
        );
        assert_eq!(
            ClientHint::UaFormFactors.value(&identity).as_deref(),
            Some("\"Desktop\"")
        );
    }
}
