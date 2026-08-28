use std::sync::atomic::{AtomicU64, Ordering};

use moli_fetch::{
    Request, RequestCredentialsMode, RequestMode, RequestRedirectMode, RequestResourceType,
};
use moli_page_types::NavigationRedirect;
use serde_json::{Map, Value, json};
use url::Url;

static NEXT_VIRTUAL_BROWSING_CONTEXT_GROUP_ID: AtomicU64 = AtomicU64::new(1);

/// Enforced Cross-Origin-Opener-Policy value used by the top-level Page owner.
///
/// The variants and swap matrix mirror Chromium's
/// `network::mojom::CrossOriginOpenerPolicyValue`. Report-only policy and
/// reporting endpoints do not alter the real browsing-context group and are
/// intentionally kept out of this owner value.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CrossOriginOpenerPolicyValue {
    #[default]
    UnsafeNone,
    SameOriginAllowPopups,
    SameOrigin,
    SameOriginPlusCoep,
    NoopenerAllowPopups,
}

/// Parsed COOP response policy, including the report-only deployment surface.
///
/// Endpoint URLs are resolved while the response's transient
/// `Reporting-Endpoints` source is available. Keeping the resolved URL here
/// lets the next navigation use the committed Document's reporter without
/// consulting mutable response state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResponseCrossOriginOpenerPolicy {
    value: CrossOriginOpenerPolicyValue,
    report_only_value: CrossOriginOpenerPolicyValue,
    reporting_endpoint: Option<Url>,
    report_only_reporting_endpoint: Option<Url>,
}

/// Chromium keeps a separate virtual browsing-context-group identity for
/// report-only COOP. It changes without severing WindowProxy relationships and
/// allows later access reporting to distinguish a deployment-only split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CrossOriginOpenerPolicyVirtualGroupId(u64);

impl CrossOriginOpenerPolicyVirtualGroupId {
    fn allocate() -> Self {
        Self(NEXT_VIRTUAL_BROWSING_CONTEXT_GROUP_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Current or prospective top-level Document state used by COOP navigation
/// admission and by the committed Document's next navigation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TopLevelDocumentCrossOriginOpenerPolicy {
    policy: ResponseCrossOriginOpenerPolicy,
    serialized_origin: String,
    document_url: Url,
    document_referrer: String,
    is_initial_empty_document: bool,
    virtual_browsing_context_group: CrossOriginOpenerPolicyVirtualGroupId,
}

impl TopLevelDocumentCrossOriginOpenerPolicy {
    #[cfg(test)]
    pub(crate) fn new(
        value: CrossOriginOpenerPolicyValue,
        serialized_origin: String,
        is_initial_empty_document: bool,
    ) -> Self {
        let document_url = Url::parse(&serialized_origin)
            .unwrap_or_else(|_| Url::parse("about:blank").expect("valid about:blank URL"));
        Self {
            policy: ResponseCrossOriginOpenerPolicy {
                value,
                ..Default::default()
            },
            serialized_origin,
            document_url,
            document_referrer: String::new(),
            is_initial_empty_document,
            virtual_browsing_context_group: CrossOriginOpenerPolicyVirtualGroupId::allocate(),
        }
    }

    fn from_response_with_identity(
        final_url: &Url,
        headers: &[(String, String)],
        document_referrer: String,
        virtual_browsing_context_group: CrossOriginOpenerPolicyVirtualGroupId,
    ) -> Self {
        Self {
            policy: response_cross_origin_opener_policy_from_headers(final_url, headers),
            serialized_origin: moli_url::origin_ascii_serialization(final_url),
            document_url: final_url.clone(),
            document_referrer,
            is_initial_empty_document: false,
            virtual_browsing_context_group,
        }
    }

    #[cfg(test)]
    pub(crate) const fn value(&self) -> CrossOriginOpenerPolicyValue {
        self.policy.value
    }

    pub(crate) fn document_referrer(&self) -> &str {
        &self.document_referrer
    }

    pub(crate) fn inherited_for_document(
        &self,
        document_url: Url,
        serialized_origin: String,
        document_referrer: String,
        is_initial_empty_document: bool,
    ) -> Self {
        Self {
            policy: self.policy.clone(),
            serialized_origin,
            document_url,
            document_referrer,
            is_initial_empty_document,
            virtual_browsing_context_group: self.virtual_browsing_context_group,
        }
    }
}

/// COOP state installed while a new realm is still private to its Page owner.
#[derive(Clone, Debug)]
pub(crate) enum CrossOriginOpenerPolicyCommit {
    Response(ResponseCrossOriginOpenerPolicy),
    Navigation {
        state: TopLevelDocumentCrossOriginOpenerPolicy,
        reports: Vec<Request>,
    },
    Inherited(TopLevelDocumentCrossOriginOpenerPolicy),
}

impl Default for CrossOriginOpenerPolicyCommit {
    fn default() -> Self {
        Self::Response(ResponseCrossOriginOpenerPolicy::default())
    }
}

impl CrossOriginOpenerPolicyCommit {
    pub(crate) fn from_response(final_url: &Url, headers: &[(String, String)]) -> Self {
        Self::Response(response_cross_origin_opener_policy_from_headers(
            final_url, headers,
        ))
    }

    pub(crate) fn resolve_for_document(
        &self,
        document_url: &Url,
        serialized_origin: String,
        document_referrer: String,
        is_initial_empty_document: bool,
    ) -> (TopLevelDocumentCrossOriginOpenerPolicy, Vec<Request>) {
        match self {
            Self::Response(policy) => (
                TopLevelDocumentCrossOriginOpenerPolicy {
                    policy: policy.clone(),
                    serialized_origin,
                    document_url: document_url.clone(),
                    document_referrer,
                    is_initial_empty_document,
                    virtual_browsing_context_group: CrossOriginOpenerPolicyVirtualGroupId::allocate(
                    ),
                },
                Vec::new(),
            ),
            Self::Navigation { state, reports } => (state.clone(), reports.clone()),
            Self::Inherited(state) => (
                state.inherited_for_document(
                    document_url.clone(),
                    serialized_origin,
                    document_referrer,
                    is_initial_empty_document,
                ),
                Vec::new(),
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CrossOriginOpenerPolicyNavigationResult {
    pub(crate) browsing_context_group_swap: bool,
    pub(crate) commit: CrossOriginOpenerPolicyCommit,
    #[cfg(test)]
    pub(crate) virtual_group_switch_count: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CrossOriginEmbedderPolicy {
    #[default]
    None,
    RequireCorp,
    Credentialless,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DocumentIsolationPolicy {
    #[default]
    None,
    IsolateAndRequireCorp,
    IsolateAndCredentialless,
}

pub(crate) fn response_headers_enable_cross_origin_isolation(
    final_url: &Url,
    headers: &[(String, String)],
) -> bool {
    if !moli_url::is_potentially_trustworthy_url(final_url) {
        return false;
    }
    if document_isolation_policy_from_headers(headers).enables_cross_origin_isolation() {
        return true;
    }
    let coop = response_header_policy_value(headers, "cross-origin-opener-policy");
    matches!(coop.as_deref(), Some("same-origin"))
        && cross_origin_embedder_policy_from_headers(headers).enables_cross_origin_isolation()
}

#[cfg(test)]
pub(crate) fn cross_origin_opener_policy_value_from_headers(
    final_url: &Url,
    headers: &[(String, String)],
) -> CrossOriginOpenerPolicyValue {
    response_cross_origin_opener_policy_from_headers(final_url, headers).value
}

pub(crate) fn response_cross_origin_opener_policy_from_headers(
    final_url: &Url,
    headers: &[(String, String)],
) -> ResponseCrossOriginOpenerPolicy {
    if !moli_url::is_potentially_trustworthy_url(final_url) {
        return ResponseCrossOriginOpenerPolicy::default();
    }
    let (mut value, reporting_group) =
        parsed_cross_origin_opener_policy_header(headers, "cross-origin-opener-policy");
    let (mut report_only_value, report_only_reporting_group) =
        parsed_cross_origin_opener_policy_header(headers, "cross-origin-opener-policy-report-only");
    let enforced_coep = cross_origin_embedder_policy_from_headers(headers);
    let report_only_coep = cross_origin_embedder_policy_from_header_name(
        headers,
        "cross-origin-embedder-policy-report-only",
    );
    if value == CrossOriginOpenerPolicyValue::SameOrigin
        && enforced_coep.enables_cross_origin_isolation()
    {
        value = CrossOriginOpenerPolicyValue::SameOriginPlusCoep;
    }
    if report_only_value == CrossOriginOpenerPolicyValue::SameOrigin
        && (enforced_coep.enables_cross_origin_isolation()
            || report_only_coep.enables_cross_origin_isolation())
    {
        report_only_value = CrossOriginOpenerPolicyValue::SameOriginPlusCoep;
    }
    let reporting_endpoints = if final_url.scheme() == "https" {
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            headers, final_url,
        )
    } else {
        Default::default()
    };
    ResponseCrossOriginOpenerPolicy {
        value,
        report_only_value,
        reporting_endpoint: reporting_group
            .as_deref()
            .and_then(|group| reporting_endpoints.endpoint_for_group(group))
            .and_then(|endpoint| Url::parse(endpoint).ok()),
        report_only_reporting_endpoint: report_only_reporting_group
            .as_deref()
            .and_then(|group| reporting_endpoints.endpoint_for_group(group))
            .and_then(|endpoint| Url::parse(endpoint).ok()),
    }
}

pub(crate) fn response_enforces_cross_origin_opener_policy(
    final_url: &Url,
    headers: &[(String, String)],
) -> bool {
    response_cross_origin_opener_policy_from_headers(final_url, headers).value
        != CrossOriginOpenerPolicyValue::UnsafeNone
}

/// Chromium's enforced COOP browsing-instance swap matrix.
///
/// Opaque serialized origins never compare same-origin here. A future
/// group-safe opaque-origin nonce can make that identity explicit without
/// weakening this fail-closed behavior.
#[cfg(test)]
pub(crate) fn should_swap_browsing_context_group_for_cross_origin_opener_policy(
    current: &TopLevelDocumentCrossOriginOpenerPolicy,
    destination: &TopLevelDocumentCrossOriginOpenerPolicy,
) -> bool {
    should_swap_browsing_context_group_for_cross_origin_opener_policy_values(
        current,
        current.policy.value,
        destination,
        destination.policy.value,
    )
}

fn should_swap_browsing_context_group_for_cross_origin_opener_policy_values(
    current: &TopLevelDocumentCrossOriginOpenerPolicy,
    current_value: CrossOriginOpenerPolicyValue,
    destination: &TopLevelDocumentCrossOriginOpenerPolicy,
    destination_value: CrossOriginOpenerPolicyValue,
) -> bool {
    use CrossOriginOpenerPolicyValue as Coop;

    let same_origin = current.serialized_origin != "null"
        && current.serialized_origin == destination.serialized_origin;
    match current_value {
        Coop::UnsafeNone => !matches!(destination_value, Coop::UnsafeNone),
        Coop::SameOriginAllowPopups => match destination_value {
            Coop::UnsafeNone => !current.is_initial_empty_document,
            Coop::SameOriginAllowPopups => !same_origin,
            Coop::SameOrigin | Coop::SameOriginPlusCoep | Coop::NoopenerAllowPopups => true,
        },
        Coop::NoopenerAllowPopups => match destination_value {
            Coop::UnsafeNone => false,
            Coop::NoopenerAllowPopups => current.is_initial_empty_document || !same_origin,
            Coop::SameOriginAllowPopups | Coop::SameOrigin | Coop::SameOriginPlusCoep => true,
        },
        Coop::SameOrigin | Coop::SameOriginPlusCoep => {
            current_value != destination_value || !same_origin
        }
    }
}

/// Evaluates every redirect response and the terminal response as one
/// navigation-owned COOP transaction.
///
/// Chromium deliberately ORs enforced mismatches across the chain. It also
/// advances the report-only virtual group after a virtual mismatch and after
/// every later response once a real mismatch has occurred. Re-deriving only
/// from the terminal headers loses both invariants.
pub(crate) fn evaluate_cross_origin_opener_policy_navigation(
    current: &TopLevelDocumentCrossOriginOpenerPolicy,
    redirect_chain: &[NavigationRedirect],
    final_url: &Url,
    final_headers: &[(String, String)],
    document_referrer: &str,
    navigation_initiator_url: Option<&Url>,
    has_other_window_in_browsing_context_group: bool,
    response_block: Option<crate::runtime::RendererMainDocumentResponseBlock>,
) -> CrossOriginOpenerPolicyNavigationResult {
    let initial_navigation_source = navigation_initiator_url.is_some_and(|initiator| {
        let origin = moli_url::origin_ascii_serialization(initiator);
        origin != "null" && origin == current.serialized_origin
    });
    let mut status = CrossOriginOpenerPolicyNavigationStatus {
        current: current.clone(),
        navigation_from_initial_empty_document: current.is_initial_empty_document,
        browsing_context_group_swap: false,
        #[cfg(test)]
        virtual_group_switch_count: 0,
        reports: Vec::new(),
        has_other_window_in_browsing_context_group,
        is_navigation_source: initial_navigation_source,
        document_referrer: document_referrer.to_owned(),
    };
    for redirect in redirect_chain {
        status.enforce_response(&redirect.from_url, &redirect.headers);
    }
    if response_block.is_some() {
        status.force_blocked_response(final_url, final_headers);
    } else {
        status.enforce_response(final_url, final_headers);
    }
    status.current.is_initial_empty_document = false;
    CrossOriginOpenerPolicyNavigationResult {
        browsing_context_group_swap: status.browsing_context_group_swap,
        #[cfg(test)]
        virtual_group_switch_count: status.virtual_group_switch_count,
        commit: CrossOriginOpenerPolicyCommit::Navigation {
            state: status.current,
            reports: status.reports,
        },
    }
}

struct CrossOriginOpenerPolicyNavigationStatus {
    current: TopLevelDocumentCrossOriginOpenerPolicy,
    navigation_from_initial_empty_document: bool,
    browsing_context_group_swap: bool,
    #[cfg(test)]
    virtual_group_switch_count: usize,
    reports: Vec<Request>,
    has_other_window_in_browsing_context_group: bool,
    is_navigation_source: bool,
    document_referrer: String,
}

impl CrossOriginOpenerPolicyNavigationStatus {
    fn force_blocked_response(
        &mut self,
        response_url: &Url,
        response_headers: &[(String, String)],
    ) {
        self.browsing_context_group_swap = true;
        #[cfg(test)]
        {
            self.virtual_group_switch_count += 1;
        }
        self.current = TopLevelDocumentCrossOriginOpenerPolicy::from_response_with_identity(
            response_url,
            response_headers,
            self.document_referrer.clone(),
            CrossOriginOpenerPolicyVirtualGroupId::allocate(),
        );
    }

    fn enforce_response(&mut self, response_url: &Url, response_headers: &[(String, String)]) {
        let mut response = TopLevelDocumentCrossOriginOpenerPolicy::from_response_with_identity(
            response_url,
            response_headers,
            self.document_referrer.clone(),
            self.current.virtual_browsing_context_group,
        );
        let cross_origin_policy_swap =
            should_swap_browsing_context_group_for_cross_origin_opener_policy_values(
                &self.current,
                self.current.policy.value,
                &response,
                response.policy.value,
            );
        self.browsing_context_group_swap |= cross_origin_policy_swap;
        let report_only_policy_swap =
            should_swap_browsing_context_group_for_cross_origin_opener_policy_values(
                &self.current,
                self.current.policy.report_only_value,
                &response,
                response.policy.report_only_value,
            );
        let navigating_to_report_only_policy_swap =
            should_swap_browsing_context_group_for_cross_origin_opener_policy_values(
                &self.current,
                self.current.policy.value,
                &response,
                response.policy.report_only_value,
            );
        let navigating_from_report_only_policy_swap =
            should_swap_browsing_context_group_for_cross_origin_opener_policy_values(
                &self.current,
                self.current.policy.report_only_value,
                &response,
                response.policy.value,
            );
        let virtual_browsing_context_group_swap = report_only_policy_swap
            && (navigating_to_report_only_policy_swap || navigating_from_report_only_policy_swap);

        if self.has_other_window_in_browsing_context_group && cross_origin_policy_swap {
            append_navigation_report_pair(
                &mut self.reports,
                &self.current,
                &response,
                self.is_navigation_source,
                false,
            );
        }
        if self.has_other_window_in_browsing_context_group && virtual_browsing_context_group_swap {
            append_navigation_report_pair(
                &mut self.reports,
                &self.current,
                &response,
                self.is_navigation_source,
                true,
            );
        }
        if self.browsing_context_group_swap || virtual_browsing_context_group_swap {
            response.virtual_browsing_context_group =
                CrossOriginOpenerPolicyVirtualGroupId::allocate();
            #[cfg(test)]
            {
                self.virtual_group_switch_count += 1;
            }
        }
        response.is_initial_empty_document = false;
        self.current = response;
        self.is_navigation_source = true;
        // Chromium retains this fact for every response in the same
        // navigation, including redirects.
        self.current.is_initial_empty_document = self.navigation_from_initial_empty_document;
    }
}

fn append_navigation_report_pair(
    reports: &mut Vec<Request>,
    previous: &TopLevelDocumentCrossOriginOpenerPolicy,
    response: &TopLevelDocumentCrossOriginOpenerPolicy,
    previous_is_navigation_source: bool,
    report_only: bool,
) {
    let same_origin = previous.serialized_origin != "null"
        && previous.serialized_origin == response.serialized_origin;
    let (response_endpoint, response_effective_policy) = if report_only {
        (
            response.policy.report_only_reporting_endpoint.as_ref(),
            response.policy.report_only_value,
        )
    } else {
        (
            response.policy.reporting_endpoint.as_ref(),
            response.policy.value,
        )
    };
    if let Some(endpoint) = response_endpoint {
        let mut body = Map::new();
        body.insert(
            "type".to_owned(),
            Value::String("navigation-to-response".to_owned()),
        );
        body.insert(
            "previousResponseURL".to_owned(),
            Value::String(if same_origin {
                sanitized_reporting_url(&previous.document_url)
            } else {
                String::new()
            }),
        );
        body.insert(
            "referrer".to_owned(),
            Value::String(sanitized_reporting_url_from_string(
                &response.document_referrer,
            )),
        );
        append_navigation_report_request(
            reports,
            endpoint,
            &response.document_url,
            body,
            report_only,
            response_effective_policy,
        );
    }

    let (previous_endpoint, previous_effective_policy) = if report_only {
        (
            previous.policy.report_only_reporting_endpoint.as_ref(),
            previous.policy.report_only_value,
        )
    } else {
        (
            previous.policy.reporting_endpoint.as_ref(),
            previous.policy.value,
        )
    };
    if let Some(endpoint) = previous_endpoint {
        let mut body = Map::new();
        body.insert(
            "type".to_owned(),
            Value::String("navigation-from-response".to_owned()),
        );
        body.insert(
            "nextResponseURL".to_owned(),
            Value::String(if previous_is_navigation_source || same_origin {
                sanitized_reporting_url(&response.document_url)
            } else {
                String::new()
            }),
        );
        append_navigation_report_request(
            reports,
            endpoint,
            &previous.document_url,
            body,
            report_only,
            previous_effective_policy,
        );
    }
}

fn append_navigation_report_request(
    reports: &mut Vec<Request>,
    endpoint: &Url,
    context_url: &Url,
    mut body: Map<String, Value>,
    report_only: bool,
    effective_policy: CrossOriginOpenerPolicyValue,
) {
    body.insert(
        "disposition".to_owned(),
        Value::String(if report_only { "reporting" } else { "enforce" }.to_owned()),
    );
    body.insert(
        "effectivePolicy".to_owned(),
        Value::String(effective_policy.label().to_owned()),
    );
    let body = json!([{
        "age": 0,
        "type": "coop",
        "url": sanitized_reporting_url(context_url),
        "body": body,
    }])
    .to_string();
    let Ok(request) = Request::new_bytes(
        "POST",
        endpoint.as_str(),
        Some(body.into_bytes()),
        vec![(
            "Content-Type".to_owned(),
            "application/reports+json".to_owned(),
        )],
    ) else {
        return;
    };
    reports.push(
        request
            .with_initiator_url(context_url)
            .with_resource_type(RequestResourceType::CspReport)
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_redirect_mode(RequestRedirectMode::Error),
    );
}

fn sanitized_reporting_url_from_string(value: &str) -> String {
    Url::parse(value)
        .ok()
        .map_or_else(String::new, |url| sanitized_reporting_url(&url))
}

fn sanitized_reporting_url(url: &Url) -> String {
    if !matches!(url.scheme(), "http" | "https") {
        return String::new();
    }
    let mut sanitized = url.clone();
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    sanitized.set_fragment(None);
    sanitized.to_string()
}

pub(crate) fn send_cross_origin_opener_policy_reports(
    loader: &crate::network::ResourceRequestClient,
    reports: Vec<Request>,
) {
    for request in reports {
        if let Err(error) = loader.fetch_text_callback(request, |result| {
            if let Err(error) = result {
                tracing::debug!(message = error.to_string(), "COOP report delivery failed");
            }
        }) {
            tracing::debug!(
                message = error.to_string(),
                "COOP report request submission failed"
            );
        }
    }
}

pub(crate) fn cross_origin_embedder_policy_from_headers(
    headers: &[(String, String)],
) -> CrossOriginEmbedderPolicy {
    cross_origin_embedder_policy_from_header_name(headers, "cross-origin-embedder-policy")
}

fn cross_origin_embedder_policy_from_header_name(
    headers: &[(String, String)],
    name: &str,
) -> CrossOriginEmbedderPolicy {
    match response_header_policy_value(headers, name).as_deref() {
        Some("require-corp") => CrossOriginEmbedderPolicy::RequireCorp,
        Some("credentialless") => CrossOriginEmbedderPolicy::Credentialless,
        _ => CrossOriginEmbedderPolicy::None,
    }
}

pub(crate) fn document_isolation_policy_from_headers(
    headers: &[(String, String)],
) -> DocumentIsolationPolicy {
    match response_header_policy_value(headers, "document-isolation-policy").as_deref() {
        Some("isolate-and-require-corp") => DocumentIsolationPolicy::IsolateAndRequireCorp,
        Some("isolate-and-credentialless") => DocumentIsolationPolicy::IsolateAndCredentialless,
        _ => DocumentIsolationPolicy::None,
    }
}

impl CrossOriginEmbedderPolicy {
    pub(crate) fn enables_cross_origin_isolation(self) -> bool {
        matches!(self, Self::RequireCorp | Self::Credentialless)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RequireCorp => "require-corp",
            Self::Credentialless => "credentialless",
        }
    }
}

impl CrossOriginOpenerPolicyValue {
    fn label(self) -> &'static str {
        match self {
            Self::UnsafeNone => "unsafe-none",
            Self::SameOriginAllowPopups => "same-origin-allow-popups",
            Self::SameOrigin => "same-origin",
            Self::SameOriginPlusCoep => "same-origin-plus-coep",
            Self::NoopenerAllowPopups => "noopener-allow-popups",
        }
    }
}

impl DocumentIsolationPolicy {
    pub(crate) fn enables_cross_origin_isolation(self) -> bool {
        matches!(
            self,
            Self::IsolateAndRequireCorp | Self::IsolateAndCredentialless
        )
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::IsolateAndRequireCorp => "isolate-and-require-corp",
            Self::IsolateAndCredentialless => "isolate-and-credentialless",
        }
    }
}

fn response_header_policy_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .rev()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
}

fn parsed_cross_origin_opener_policy_header(
    headers: &[(String, String)],
    name: &str,
) -> (CrossOriginOpenerPolicyValue, Option<String>) {
    let Some((_, raw_value)) = headers
        .iter()
        .rev()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
    else {
        return Default::default();
    };
    let mut members = raw_value.split(';');
    let value = match members.next().unwrap_or_default().trim() {
        "same-origin-allow-popups" => CrossOriginOpenerPolicyValue::SameOriginAllowPopups,
        "same-origin" => CrossOriginOpenerPolicyValue::SameOrigin,
        "noopener-allow-popups" => CrossOriginOpenerPolicyValue::NoopenerAllowPopups,
        "unsafe-none" => CrossOriginOpenerPolicyValue::UnsafeNone,
        _ => return Default::default(),
    };
    let reporting_group = members.find_map(|parameter| {
        let (name, raw_value) = parameter.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("report-to") {
            return None;
        }
        let value = raw_value.trim();
        (value.len() >= 2 && value.starts_with('"') && value.ends_with('"'))
            .then(|| value[1..value.len() - 1].to_owned())
    });
    (value, reporting_group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redirect(
        from_url: &str,
        to_url: &str,
        headers: Vec<(String, String)>,
    ) -> NavigationRedirect {
        NavigationRedirect {
            from_url: Url::parse(from_url).expect("valid redirect source URL"),
            to_url: Url::parse(to_url).expect("valid redirect destination URL"),
            status: 302,
            headers,
            network_extra_info_available: false,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }
    }

    #[test]
    fn coop_coep_headers_enable_cross_origin_isolation_for_trustworthy_urls() {
        let url = Url::parse("https://example.test/").expect("valid url");
        let headers = vec![
            (
                "Cross-Origin-Embedder-Policy".to_owned(),
                "require-corp".to_owned(),
            ),
            (
                "Cross-Origin-Opener-Policy".to_owned(),
                "same-origin".to_owned(),
            ),
        ];
        assert!(response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn cross_origin_isolation_requires_both_headers() {
        let url = Url::parse("https://example.test/").expect("valid url");
        let headers = vec![(
            "Cross-Origin-Opener-Policy".to_owned(),
            "same-origin".to_owned(),
        )];
        assert!(!response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn document_isolation_policy_enables_cross_origin_isolation_for_trustworthy_urls() {
        let url = Url::parse("https://example.test/").expect("valid url");
        let headers = vec![(
            "Document-Isolation-Policy".to_owned(),
            "isolate-and-require-corp".to_owned(),
        )];
        assert!(response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn document_isolation_policy_cross_origin_isolation_requires_trustworthy_url() {
        let url = Url::parse("http://example.test/").expect("valid url");
        let headers = vec![(
            "Document-Isolation-Policy".to_owned(),
            "isolate-and-credentialless".to_owned(),
        )];
        assert!(!response_headers_enable_cross_origin_isolation(
            &url, &headers
        ));
    }

    #[test]
    fn parses_cross_origin_embedder_policy_header_values() {
        assert_eq!(
            cross_origin_embedder_policy_from_headers(&[(
                "Cross-Origin-Embedder-Policy".to_owned(),
                "require-corp; report-to=\"endpoint\"".to_owned()
            )]),
            CrossOriginEmbedderPolicy::RequireCorp
        );
        assert_eq!(
            cross_origin_embedder_policy_from_headers(&[(
                "Cross-Origin-Embedder-Policy".to_owned(),
                "credentialless".to_owned()
            )]),
            CrossOriginEmbedderPolicy::Credentialless
        );
        assert_eq!(
            cross_origin_embedder_policy_from_headers(&[(
                "Cross-Origin-Embedder-Policy".to_owned(),
                "invalid".to_owned()
            )]),
            CrossOriginEmbedderPolicy::None
        );
    }

    #[test]
    fn parses_enforced_cross_origin_opener_policy_and_augments_same_origin_with_coep() {
        let trustworthy = Url::parse("https://example.test/").expect("trustworthy URL");
        assert_eq!(
            cross_origin_opener_policy_value_from_headers(
                &trustworthy,
                &[(
                    "Cross-Origin-Opener-Policy".to_owned(),
                    "same-origin-allow-popups; report-to=endpoint".to_owned(),
                )],
            ),
            CrossOriginOpenerPolicyValue::SameOriginAllowPopups
        );
        assert_eq!(
            cross_origin_opener_policy_value_from_headers(
                &trustworthy,
                &[
                    (
                        "Cross-Origin-Opener-Policy".to_owned(),
                        "same-origin".to_owned(),
                    ),
                    (
                        "Cross-Origin-Embedder-Policy".to_owned(),
                        "require-corp".to_owned(),
                    ),
                ],
            ),
            CrossOriginOpenerPolicyValue::SameOriginPlusCoep
        );
        let untrustworthy = Url::parse("http://example.test/").expect("untrustworthy URL");
        assert_eq!(
            cross_origin_opener_policy_value_from_headers(
                &untrustworthy,
                &[(
                    "Cross-Origin-Opener-Policy".to_owned(),
                    "same-origin".to_owned(),
                )],
            ),
            CrossOriginOpenerPolicyValue::UnsafeNone
        );
    }

    #[test]
    fn parses_report_only_coop_and_resolves_secure_reporting_endpoints() {
        let url = Url::parse("https://example.test/document").expect("valid URL");
        let policy = response_cross_origin_opener_policy_from_headers(
            &url,
            &[
                (
                    "Cross-Origin-Opener-Policy".to_owned(),
                    "same-origin; report-to=\"enforced\"".to_owned(),
                ),
                (
                    "Cross-Origin-Opener-Policy-Report-Only".to_owned(),
                    "same-origin; report-to=\"deployment\"".to_owned(),
                ),
                (
                    "Cross-Origin-Embedder-Policy-Report-Only".to_owned(),
                    "require-corp".to_owned(),
                ),
                (
                    "Reporting-Endpoints".to_owned(),
                    "enforced=\"/coop-enforced\", deployment=\"/coop-deployment\"".to_owned(),
                ),
            ],
        );

        assert_eq!(policy.value, CrossOriginOpenerPolicyValue::SameOrigin);
        assert_eq!(
            policy.report_only_value,
            CrossOriginOpenerPolicyValue::SameOriginPlusCoep
        );
        assert_eq!(
            policy.reporting_endpoint.as_ref().map(Url::as_str),
            Some("https://example.test/coop-enforced")
        );
        assert_eq!(
            policy
                .report_only_reporting_endpoint
                .as_ref()
                .map(Url::as_str),
            Some("https://example.test/coop-deployment")
        );
    }

    #[test]
    fn redirect_coop_mismatch_remains_authoritative_when_final_response_matches_source() {
        let current = TopLevelDocumentCrossOriginOpenerPolicy::new(
            CrossOriginOpenerPolicyValue::UnsafeNone,
            "https://a.test".to_owned(),
            false,
        );
        let result = evaluate_cross_origin_opener_policy_navigation(
            &current,
            &[redirect(
                "https://a.test/start",
                "https://a.test/final",
                vec![(
                    "Cross-Origin-Opener-Policy".to_owned(),
                    "same-origin".to_owned(),
                )],
            )],
            &Url::parse("https://a.test/final").expect("valid final URL"),
            &[],
            "https://a.test/source",
            Some(&Url::parse("https://a.test/source").expect("valid initiator URL")),
            true,
            None,
        );

        assert!(result.browsing_context_group_swap);
        assert_eq!(result.virtual_group_switch_count, 2);
        let CrossOriginOpenerPolicyCommit::Navigation { state, .. } = result.commit else {
            panic!("navigation evaluator must produce an exact commit");
        };
        assert_eq!(state.value(), CrossOriginOpenerPolicyValue::UnsafeNone);
        assert!(!state.is_initial_empty_document);
    }

    #[test]
    fn blocked_response_forces_real_and_virtual_group_switch_without_enforcing_error_coop() {
        let current = TopLevelDocumentCrossOriginOpenerPolicy::new(
            CrossOriginOpenerPolicyValue::UnsafeNone,
            "https://a.test".to_owned(),
            false,
        );
        let error_url = Url::parse("chrome-error://chromewebdata/").expect("valid error URL");
        let result = evaluate_cross_origin_opener_policy_navigation(
            &current,
            &[],
            &error_url,
            &[("content-type".to_owned(), "text/html".to_owned())],
            "https://a.test/source",
            Some(&Url::parse("https://a.test/source").expect("valid initiator URL")),
            true,
            Some(
                crate::runtime::RendererMainDocumentResponseBlock::CrossOriginOpenerPolicySandboxedNavigation,
            ),
        );

        assert!(result.browsing_context_group_swap);
        assert_eq!(result.virtual_group_switch_count, 1);
        let CrossOriginOpenerPolicyCommit::Navigation { state, reports } = result.commit else {
            panic!("blocked navigation must produce an exact error-Document commit");
        };
        assert_eq!(state.value(), CrossOriginOpenerPolicyValue::UnsafeNone);
        assert!(reports.is_empty());
    }

    #[test]
    fn report_only_swap_changes_only_virtual_group_and_builds_reporting_api_request() {
        let current_url = Url::parse("https://a.test/current").expect("valid current URL");
        let current = TopLevelDocumentCrossOriginOpenerPolicy::from_response_with_identity(
            &current_url,
            &[],
            String::new(),
            CrossOriginOpenerPolicyVirtualGroupId::allocate(),
        );
        let destination_url =
            Url::parse("https://a.test/destination#secret").expect("valid destination URL");
        let result = evaluate_cross_origin_opener_policy_navigation(
            &current,
            &[],
            &destination_url,
            &[
                (
                    "Cross-Origin-Opener-Policy-Report-Only".to_owned(),
                    "same-origin; report-to=\"coop\"".to_owned(),
                ),
                (
                    "Reporting-Endpoints".to_owned(),
                    "coop=\"/reports\"".to_owned(),
                ),
            ],
            "https://user:password@a.test/referrer#fragment",
            Some(&current_url),
            true,
            None,
        );

        assert!(!result.browsing_context_group_swap);
        assert_eq!(result.virtual_group_switch_count, 1);
        let CrossOriginOpenerPolicyCommit::Navigation { state, reports } = result.commit else {
            panic!("navigation evaluator must produce an exact commit");
        };
        assert_ne!(
            state.virtual_browsing_context_group,
            current.virtual_browsing_context_group
        );
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].url.as_str(), "https://a.test/reports");
        assert_eq!(reports[0].request_mode, RequestMode::NoCors);
        assert_eq!(
            reports[0].credentials_mode,
            RequestCredentialsMode::SameOrigin
        );
        assert_eq!(reports[0].redirect_mode, RequestRedirectMode::Error);
        let body: Value =
            serde_json::from_slice(reports[0].body.as_deref().expect("COOP report body"))
                .expect("valid COOP report JSON");
        assert_eq!(body[0]["type"], "coop");
        assert_eq!(body[0]["url"], "https://a.test/destination");
        assert_eq!(body[0]["body"]["type"], "navigation-to-response");
        assert_eq!(body[0]["body"]["disposition"], "reporting");
        assert_eq!(body[0]["body"]["effectivePolicy"], "same-origin");
        assert_eq!(
            body[0]["body"]["previousResponseURL"],
            "https://a.test/current"
        );
        assert_eq!(body[0]["body"]["referrer"], "https://a.test/referrer");
    }

    #[test]
    fn coop_group_swap_matrix_matches_chromium_for_committed_and_initial_empty_documents() {
        use CrossOriginOpenerPolicyValue as Coop;

        struct Case {
            from: Coop,
            to: Coop,
            same_origin: bool,
            cross_origin: bool,
            initial_empty_cross_origin: bool,
        }
        let cases = [
            Case {
                from: Coop::UnsafeNone,
                to: Coop::UnsafeNone,
                same_origin: false,
                cross_origin: false,
                initial_empty_cross_origin: false,
            },
            Case {
                from: Coop::UnsafeNone,
                to: Coop::SameOriginAllowPopups,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::UnsafeNone,
                to: Coop::SameOrigin,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::UnsafeNone,
                to: Coop::SameOriginPlusCoep,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::SameOriginAllowPopups,
                to: Coop::UnsafeNone,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: false,
            },
            Case {
                from: Coop::SameOriginAllowPopups,
                to: Coop::SameOriginAllowPopups,
                same_origin: false,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::SameOriginAllowPopups,
                to: Coop::SameOrigin,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::SameOrigin,
                to: Coop::UnsafeNone,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::SameOrigin,
                to: Coop::SameOrigin,
                same_origin: false,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::SameOrigin,
                to: Coop::SameOriginPlusCoep,
                same_origin: true,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::SameOriginPlusCoep,
                to: Coop::SameOriginPlusCoep,
                same_origin: false,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
            Case {
                from: Coop::NoopenerAllowPopups,
                to: Coop::UnsafeNone,
                same_origin: false,
                cross_origin: false,
                initial_empty_cross_origin: false,
            },
            Case {
                from: Coop::NoopenerAllowPopups,
                to: Coop::NoopenerAllowPopups,
                same_origin: false,
                cross_origin: true,
                initial_empty_cross_origin: true,
            },
        ];
        for case in cases {
            let current_same = TopLevelDocumentCrossOriginOpenerPolicy::new(
                case.from,
                "https://a.test".to_owned(),
                false,
            );
            let destination_same = TopLevelDocumentCrossOriginOpenerPolicy::new(
                case.to,
                "https://a.test".to_owned(),
                false,
            );
            assert_eq!(
                should_swap_browsing_context_group_for_cross_origin_opener_policy(
                    &current_same,
                    &destination_same,
                ),
                case.same_origin,
                "same-origin case {:?} -> {:?}",
                case.from,
                case.to,
            );

            let destination_cross = TopLevelDocumentCrossOriginOpenerPolicy::new(
                case.to,
                "https://b.test".to_owned(),
                false,
            );
            assert_eq!(
                should_swap_browsing_context_group_for_cross_origin_opener_policy(
                    &current_same,
                    &destination_cross,
                ),
                case.cross_origin,
                "cross-origin case {:?} -> {:?}",
                case.from,
                case.to,
            );

            let current_initial = TopLevelDocumentCrossOriginOpenerPolicy::new(
                case.from,
                "https://a.test".to_owned(),
                true,
            );
            assert_eq!(
                should_swap_browsing_context_group_for_cross_origin_opener_policy(
                    &current_initial,
                    &destination_cross,
                ),
                case.initial_empty_cross_origin,
                "initial-empty case {:?} -> {:?}",
                case.from,
                case.to,
            );
        }
    }

    #[test]
    fn parses_document_isolation_policy_header_values() {
        assert_eq!(
            document_isolation_policy_from_headers(&[(
                "Document-Isolation-Policy".to_owned(),
                "isolate-and-require-corp; report-to=\"endpoint\"".to_owned()
            )]),
            DocumentIsolationPolicy::IsolateAndRequireCorp
        );
        assert_eq!(
            document_isolation_policy_from_headers(&[(
                "Document-Isolation-Policy".to_owned(),
                "isolate-and-credentialless".to_owned()
            )]),
            DocumentIsolationPolicy::IsolateAndCredentialless
        );
        assert_eq!(
            document_isolation_policy_from_headers(&[(
                "Document-Isolation-Policy".to_owned(),
                "invalid".to_owned()
            )]),
            DocumentIsolationPolicy::None
        );
    }
}
