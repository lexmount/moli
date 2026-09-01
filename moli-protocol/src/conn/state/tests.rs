use crate::devtools_runtime::DevToolsNetworkResourceType;
use moli_cookie_jar::new_shared_browser_cookie_store;
use moli_core::page::{SameDocumentHistoryUpdate, SubresourceResourceType};

use super::super::fetch_support::{
    FetchAuthChallenge, PendingFetchAuthNavigation, PendingFetchNavigation,
    PendingSubresourceFetchOwnerKind,
};
use super::browser_context::BrowserContext;
use super::devtools_session::DevToolsSessionState;
use super::fetch::{
    FetchInterceptionPattern, FetchRequestStage, FetchResourceTypeFilter, TargetFetchOwner,
    TargetFetchState,
};
use super::navigation::{PageNavigationHistoryEntry, TargetNavigationHistoryState};
use super::navigation_outcome::{NavigationDispatchState, NavigationResultProjection};
use super::page_slot::{DocumentStartScript, IsolatedWorldDefinition};
use super::runtime_slot::TargetRuntimeSlot;
use super::session::TargetPageSessionState;
use super::{
    PageTargetHost,
    parking::{ParkedTargetOwnerState, TargetOwnerState},
};

use serde_json::json;
use std::collections::HashMap;
use url::Url;

fn test_navigation_dispatch_state(fetch_request_id: &str) -> NavigationDispatchState {
    NavigationDispatchState {
        navigate_id: Some(1),
        navigate_session_id: Some("SID-1".to_owned()),
        result_projection: NavigationResultProjection::Cdp(
            json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
        ),
        frame_id: "TID-1".to_owned(),
        session_id: Some("SID-1".to_owned()),
        request_id: Some(format!("NETWORK-{fetch_request_id}")),
        loader_id: "LID-0000000001".to_owned(),
        request_announced: false,
        requested_url: Url::parse("https://example.test/").unwrap(),
        request_method: "GET".to_owned(),
        request_body: None,
        request_body_bytes: None,
        request_headers: Vec::new(),
        request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
        timestamp: 0.0,
        source_document_security: Default::default(),
    }
}

#[test]
fn background_target_owns_page_session_state_atomically() {
    let mut target = PageTargetHost::with_url("TID-A".to_owned(), None, "about:blank".to_owned());

    let mut state = PageTargetHost::empty("TID-state-test".to_owned());
    state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary] = DevToolsSessionState {
        page_session_state: TargetPageSessionState {
            log_enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };
    *target.state_mut() = state;
    assert!(
        target.state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled
    );

    target.state_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .log_enabled = false;
    assert!(
        !target.state().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled
    );
}

#[test]
fn page_domain_subscription_generation_tracks_distinct_enable_lifetimes() {
    let mut state = TargetPageSessionState::default();
    assert_eq!(state.page_domain_subscription_generation(), None);

    state.enable_page_domain(17);
    let first = state
        .page_domain_subscription_generation()
        .expect("enabled Page domain should expose a subscription generation");
    state.enable_page_domain(18);
    assert_eq!(
        state.page_domain_subscription_generation(),
        Some(first),
        "repeated Page.enable must preserve active download observers"
    );

    state.disable_page_domain();
    assert!(!state.page_domain_subscription_is_current(first));
    assert_eq!(
        state,
        TargetPageSessionState::default(),
        "internal subscription generations must not keep default parked state alive"
    );

    state.enable_page_domain(19);
    let second = state
        .page_domain_subscription_generation()
        .expect("re-enabled Page domain should expose a subscription generation");
    assert_ne!(
        second, first,
        "Page.disable followed by Page.enable must not resume old download observers"
    );
}

#[test]
fn input_event_ignore_state_survives_document_context_replacement() {
    let mut state = TargetPageSessionState {
        input_events_ignored: true,
        ..Default::default()
    };

    state.clear_loaded_document_context_state();

    assert!(
        state.input_events_ignored,
        "Chromium retains each Inspector session's input ignore handle across renderer navigation"
    );
}

#[test]
fn target_fetch_owner_projects_config_without_exposing_slots() {
    let mut owner = TargetFetchOwner::default();
    owner.configure(
        Some("FETCH-SID".to_owned()),
        true,
        vec![
            FetchInterceptionPattern {
                url_pattern: "*://example.test/api".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
                request_stage: FetchRequestStage::Response,
            },
            FetchInterceptionPattern {
                url_pattern: "*://example.test/ws".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::WebSocket),
                request_stage: FetchRequestStage::Request,
            },
        ],
    );

    assert!(owner.is_enabled());
    assert!(owner.handle_auth_requests());
    assert_eq!(
        owner.event_session_id(Some("NETWORK-SID")),
        Some("FETCH-SID")
    );
    let snapshot = owner.subresource_interception_snapshot();
    assert_eq!(
        owner.subresource_interception_config(),
        (true, None),
        "Fetch + WebSocket filters require broad subresource interception"
    );
    assert_eq!(
        snapshot.event_session_id(None),
        Some("FETCH-SID"),
        "subresource snapshot preserves the event session id"
    );
    assert_eq!(
        snapshot.matching_request_stage(
            DevToolsNetworkResourceType::Fetch,
            &Url::parse("https://example.test/api").unwrap(),
        ),
        Some(FetchRequestStage::Response)
    );
    assert_eq!(
        snapshot.matching_request_stage(
            DevToolsNetworkResourceType::WebSocket,
            &Url::parse("wss://example.test/ws").unwrap(),
        ),
        Some(FetchRequestStage::Request)
    );
    assert_eq!(
        owner.matching_document_request_stage(&Url::parse("https://example.test/api").unwrap()),
        None
    );

    owner.configure(
        None,
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "*://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    assert_eq!(
        owner.event_session_id(Some("NETWORK-SID")),
        Some("FETCH-SID"),
        "no-session Fetch.enable no longer replaces the session-owned Fetch config"
    );
    assert_eq!(
        owner.subresource_interception_config(),
        (true, None),
        "aggregate subresource config still includes the FETCH-SID WebSocket pattern"
    );
    let root_config = owner.config_snapshot_for_session(None);
    assert_eq!(root_config.session_id(), None);
    assert_eq!(
        root_config.subresource_interception_config(),
        (true, Some(SubresourceResourceType::Fetch))
    );

    owner.configure(
        Some("FETCH-SID".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "*://example.test/doc".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Document),
            request_stage: FetchRequestStage::Response,
        }],
    );
    assert_eq!(
        owner.matching_document_request_stage(&Url::parse("https://example.test/doc").unwrap()),
        Some(FetchRequestStage::Response)
    );
    assert_eq!(
        owner.matching_document_request_stage(&Url::parse("https://example.test/other").unwrap()),
        None
    );

    owner.reset_config();
    assert!(!owner.is_enabled());
    assert_eq!(owner.subresource_interception_config(), (false, None));
    assert_eq!(
        owner.matching_document_request_stage(&Url::parse("https://example.test/doc").unwrap()),
        None
    );
    assert_eq!(
        owner
            .subresource_interception_snapshot()
            .matching_request_stage(
                DevToolsNetworkResourceType::Document,
                &Url::parse("https://example.test/doc").unwrap(),
            ),
        None
    );
}

#[test]
fn target_fetch_owner_aggregates_network_intercepts_by_id() {
    let mut owner = TargetFetchOwner::default();
    owner.add_network_intercept(
        "intercept-request".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/request".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.add_network_intercept(
        "intercept-response".to_owned(),
        Some("BIDI-SID".to_owned()),
        true,
        vec!["https://example.test/response".to_owned()],
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/response".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Xhr),
            request_stage: FetchRequestStage::Response,
        }],
    );

    assert!(owner.is_enabled());
    assert!(owner.handle_auth_requests());
    assert_eq!(owner.event_session_id(None), Some("BIDI-SID"));
    let config = owner.config_snapshot();
    assert_eq!(config.patterns().len(), 2);
    assert_eq!(
        config.patterns()[0].request_stage,
        FetchRequestStage::Request
    );
    assert_eq!(
        config.patterns()[1].request_stage,
        FetchRequestStage::Response
    );

    assert!(owner.remove_network_intercept("intercept-request"));
    let config = owner.config_snapshot();
    assert!(owner.is_enabled());
    assert_eq!(config.patterns().len(), 1);
    assert_eq!(
        config.patterns()[0].request_stage,
        FetchRequestStage::Response
    );
    assert!(owner.handle_auth_requests());

    assert!(!owner.remove_network_intercept("missing-intercept"));
    assert!(owner.remove_network_intercept("intercept-response"));
    assert!(!owner.is_enabled());
    assert_eq!(owner.subresource_interception_config(), (false, None));
}

#[test]
fn target_fetch_owner_keeps_fetch_enable_configs_per_session() {
    let mut owner = TargetFetchOwner::default();
    owner.configure(
        Some("SID-primary".to_owned()),
        true,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/primary".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.configure(
        Some("SID-aux".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/aux".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Xhr),
            request_stage: FetchRequestStage::Response,
        }],
    );

    let aggregate = owner.config_snapshot();
    assert!(aggregate.is_enabled());
    assert_eq!(aggregate.patterns().len(), 2);
    assert!(aggregate.handle_auth_requests());
    assert_eq!(
        owner.subresource_interception_config(),
        (true, Some(SubresourceResourceType::Fetch))
    );

    let primary = owner.config_snapshot_for_session(Some("SID-primary"));
    assert!(primary.is_enabled());
    assert!(primary.handle_auth_requests());
    assert_eq!(primary.patterns().len(), 1);
    assert_eq!(
        primary.patterns()[0].url_pattern,
        "https://example.test/primary"
    );
    assert_eq!(
        primary.subresource_interception_config(),
        (true, Some(SubresourceResourceType::Fetch))
    );

    let aux = owner.config_snapshot_for_session(Some("SID-aux"));
    assert!(aux.is_enabled());
    assert!(!aux.handle_auth_requests());
    assert_eq!(aux.patterns().len(), 1);
    assert_eq!(aux.patterns()[0].url_pattern, "https://example.test/aux");
    assert_eq!(
        aux.subresource_interception_config(),
        (true, Some(SubresourceResourceType::Xhr))
    );

    assert!(owner.remove_fetch_session(Some("SID-primary")));
    let aggregate = owner.config_snapshot();
    assert!(aggregate.is_enabled());
    assert!(!aggregate.handle_auth_requests());
    assert_eq!(aggregate.patterns().len(), 1);
    assert_eq!(
        aggregate.patterns()[0].url_pattern,
        "https://example.test/aux"
    );
    assert!(
        !owner
            .config_snapshot_for_session(Some("SID-primary"))
            .is_enabled()
    );
    assert!(
        owner
            .config_snapshot_for_session(Some("SID-aux"))
            .is_enabled()
    );
}

#[test]
fn target_fetch_owner_appends_network_intercept_after_fetch_request_stage_sessions() {
    let mut owner = TargetFetchOwner::default();
    owner.configure(
        Some("SID-z".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.configure(
        Some("SID-a".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.add_network_intercept(
        "intercept-bidi".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );

    let api_url = Url::parse("https://example.test/api").unwrap();
    let pause_sessions = owner
        .config_snapshot()
        .subresource_interception_snapshot()
        .matching_request_stage_pause_sessions(
            Some("SID-z"),
            DevToolsNetworkResourceType::Fetch,
            &api_url,
        )
        .into_iter()
        .map(|session| (session.session_id, session.owner_kind))
        .collect::<Vec<_>>();

    assert_eq!(
        pause_sessions,
        vec![
            (
                Some("SID-z".to_owned()),
                PendingSubresourceFetchOwnerKind::Fetch
            ),
            (
                Some("SID-a".to_owned()),
                PendingSubresourceFetchOwnerKind::Fetch
            ),
            (
                Some("BIDI-SID".to_owned()),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi
            ),
        ]
    );
}

#[test]
fn target_fetch_owner_appends_network_intercept_after_fetch_response_stage_sessions() {
    let mut owner = TargetFetchOwner::default();
    owner.configure(
        Some("SID-z".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Response,
        }],
    );
    owner.configure(
        Some("SID-a".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Response,
        }],
    );
    owner.add_network_intercept(
        "intercept-bidi".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Response,
        }],
    );

    let api_url = Url::parse("https://example.test/api").unwrap();
    let pause_sessions = owner
        .config_snapshot()
        .subresource_interception_snapshot()
        .matching_response_stage_pause_sessions(
            Some("SID-z"),
            DevToolsNetworkResourceType::Fetch,
            &api_url,
        )
        .into_iter()
        .map(|session| (session.session_id, session.owner_kind))
        .collect::<Vec<_>>();

    assert_eq!(
        pause_sessions,
        vec![
            (
                Some("SID-z".to_owned()),
                PendingSubresourceFetchOwnerKind::Fetch
            ),
            (
                Some("SID-a".to_owned()),
                PendingSubresourceFetchOwnerKind::Fetch
            ),
            (
                Some("BIDI-SID".to_owned()),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi
            ),
        ]
    );
}

#[test]
fn target_fetch_owner_appends_network_intercept_after_fetch_auth_required_sessions() {
    let mut owner = TargetFetchOwner::default();
    owner.configure(
        Some("SID-z".to_owned()),
        true,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.configure(
        Some("SID-a".to_owned()),
        true,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/other".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.add_network_intercept(
        "intercept-bidi".to_owned(),
        Some("BIDI-SID".to_owned()),
        true,
        vec!["https://example.test/protected*".to_owned()],
        Vec::new(),
    );

    let api_url = Url::parse("https://example.test/protected/basic").unwrap();
    let pause_sessions = owner
        .config_snapshot()
        .subresource_interception_snapshot()
        .matching_auth_required_pause_sessions(Some("SID-z"), &api_url)
        .into_iter()
        .map(|session| (session.session_id, session.owner_kind))
        .collect::<Vec<_>>();

    assert_eq!(
        pause_sessions,
        vec![
            (
                Some("SID-z".to_owned()),
                PendingSubresourceFetchOwnerKind::Fetch
            ),
            (
                Some("SID-a".to_owned()),
                PendingSubresourceFetchOwnerKind::Fetch
            ),
            (
                Some("BIDI-SID".to_owned()),
                PendingSubresourceFetchOwnerKind::NetworkOrBidi
            ),
        ]
    );
}

#[test]
fn target_fetch_owner_network_request_stage_survives_earlier_fetch_response_pattern() {
    let mut owner = TargetFetchOwner::default();
    owner.configure(
        Some("SID-fetch".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Response,
        }],
    );
    owner.add_network_intercept(
        "intercept-bidi".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );

    let api_url = Url::parse("https://example.test/api").unwrap();
    let pause_sessions = owner
        .config_snapshot()
        .subresource_interception_snapshot()
        .matching_request_stage_pause_sessions(
            Some("SID-fetch"),
            DevToolsNetworkResourceType::Fetch,
            &api_url,
        );

    assert_eq!(pause_sessions.len(), 1);
    assert_eq!(pause_sessions[0].session_id.as_deref(), Some("BIDI-SID"));
    assert_eq!(
        pause_sessions[0].owner_kind,
        PendingSubresourceFetchOwnerKind::NetworkOrBidi
    );
}

#[test]
fn target_fetch_owner_keeps_dormant_network_intercept_removable() {
    let mut owner = TargetFetchOwner::default();
    owner.add_network_intercept(
        "intercept-dormant".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        Vec::new(),
    );

    assert!(!owner.is_enabled());
    assert_eq!(owner.subresource_interception_config(), (false, None));
    assert_eq!(owner.config_snapshot().patterns(), &[]);
    assert!(
        owner
            .config_snapshot()
            .matching_network_intercepts(
                FetchRequestStage::Request,
                DevToolsNetworkResourceType::Fetch,
                &Url::parse("https://example.test/api").unwrap(),
            )
            .is_empty()
    );
    assert!(owner.remove_network_intercept("intercept-dormant"));
    assert!(!owner.is_enabled());
}

#[test]
fn target_fetch_owner_reports_matching_network_intercept_ids_by_stage() {
    let mut owner = TargetFetchOwner::default();
    owner.add_network_intercept(
        "intercept-global".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: None,
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.add_network_intercept(
        "intercept-api".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    owner.add_network_intercept(
        "intercept-response".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/api".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Response,
        }],
    );

    let api_url = Url::parse("https://example.test/api").unwrap();
    let config = owner.config_snapshot();
    let request_intercepts = config.matching_network_intercepts(
        FetchRequestStage::Request,
        DevToolsNetworkResourceType::Fetch,
        &api_url,
    );
    assert_eq!(
        request_intercepts
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-api", "intercept-global"]
    );
    assert_eq!(
        config
            .matching_network_intercepts(
                FetchRequestStage::Response,
                DevToolsNetworkResourceType::Fetch,
                &api_url,
            )
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-response"]
    );
    assert_eq!(
        config
            .matching_network_intercepts(
                FetchRequestStage::Response,
                DevToolsNetworkResourceType::Xhr,
                &api_url,
            )
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-response"]
    );
    assert_eq!(
        config
            .matching_network_intercepts(
                FetchRequestStage::Request,
                DevToolsNetworkResourceType::Xhr,
                &api_url,
            )
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-api", "intercept-global"]
    );

    assert!(owner.remove_network_intercept("intercept-api"));
    assert_eq!(
        owner
            .config_snapshot()
            .subresource_interception_snapshot()
            .matching_network_intercepts(
                FetchRequestStage::Request,
                DevToolsNetworkResourceType::Fetch,
                &api_url,
            )
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-global"]
    );
}

#[test]
fn target_fetch_owner_reports_matching_auth_required_network_intercepts() {
    let mut owner = TargetFetchOwner::default();
    owner.add_network_intercept(
        "intercept-auth".to_owned(),
        Some("BIDI-SID".to_owned()),
        true,
        vec!["https://example.test/protected*".to_owned()],
        Vec::new(),
    );
    owner.add_network_intercept(
        "intercept-auth-all".to_owned(),
        Some("BIDI-SID".to_owned()),
        true,
        vec!["*".to_owned()],
        Vec::new(),
    );

    let matching_url = Url::parse("https://example.test/protected/basic").unwrap();
    let other_url = Url::parse("https://other.test/protected/basic").unwrap();
    let config = owner.config_snapshot();
    assert_eq!(owner.subresource_interception_config(), (true, None));
    assert!(config.matches_auth_required(&matching_url));
    assert!(config.matches_auth_required(&other_url));
    assert_eq!(
        config.matching_request_stage(DevToolsNetworkResourceType::Fetch, &matching_url),
        None
    );
    assert_eq!(
        config
            .matching_auth_required_network_intercepts(&matching_url)
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-auth", "intercept-auth-all"]
    );
    assert_eq!(
        config
            .matching_auth_required_network_intercepts(&other_url)
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-auth-all"]
    );

    owner.add_network_intercept(
        "intercept-request".to_owned(),
        Some("BIDI-SID".to_owned()),
        false,
        Vec::new(),
        vec![FetchInterceptionPattern {
            url_pattern: "https://example.test/protected".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
            request_stage: FetchRequestStage::Request,
        }],
    );
    assert!(owner.remove_network_intercept("intercept-auth-all"));
    let config = owner.config_snapshot();
    assert!(config.matches_auth_required(&matching_url));
    assert!(!config.matches_auth_required(&other_url));
    assert!(owner.remove_network_intercept("intercept-auth"));
    assert!(owner.is_enabled());
    assert!(!owner.config_snapshot().matches_auth_required(&matching_url));
}

#[test]
fn pending_navigation_rejects_generic_request_action_without_consuming_id() {
    let mut state = TargetFetchState::default();
    state.register_pending_fetch_navigation_request(PendingFetchNavigation {
        fetch_request_id: "FETCH-NAV".to_owned(),
        interception_session_id: Some("SID-fetch".to_owned()),
        document_navigation_token: None,
        navigation: test_navigation_dispatch_state("FETCH-NAV"),
        request_cookie_report: None,
        intercept_response: false,
        response_stage_url_match_policy: crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
        auth_required_blocked_intercepts: Vec::new(),
    });

    assert_eq!(
        state.consume_pending_request_action("FETCH-NAV"),
        Err("RequestNotFound")
    );
    assert!(
        state.has_pending_fetch_request_id_for_test("FETCH-NAV"),
        "mismatched action must not consume the request id"
    );
    assert!(
        state.has_pending_fetch_navigation_for_test("FETCH-NAV"),
        "mismatched action must leave the navigation pending"
    );
    assert!(
        state
            .take_pending_fetch_navigation_for_action_session("FETCH-NAV", Some("SID-other"))
            .is_none(),
        "a different session must not claim the navigation pause"
    );

    let pending = state
        .take_pending_fetch_navigation_for_action_session("FETCH-NAV", Some("SID-fetch"))
        .expect("pending navigation should still be actionable");
    assert_eq!(pending.fetch_request_id, "FETCH-NAV");
}

#[test]
fn pending_auth_navigation_rejects_generic_request_action_without_consuming_id() {
    let mut state = TargetFetchState::default();
    state.register_pending_fetch_auth_navigation(
        "FETCH-AUTH".to_owned(),
        PendingFetchAuthNavigation {
            owner_session_id: None,
            action_session_id: None,
            interception_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            fetch_request_id: "FETCH-AUTH".to_owned(),
            response_stage_request_id: "FETCH-AUTH".to_owned(),
            document_navigation_token: None,
            navigation: test_navigation_dispatch_state("FETCH-AUTH"),
            challenge: FetchAuthChallenge {
                origin: "https://example.test".to_owned(),
                source: "Server".to_owned(),
                scheme: "basic".to_owned(),
                realm: "test-area".to_owned(),
            },
            request_cookie_report: None,
            auth_response: PendingFetchAuthNavigation::test_auth_response(
                Url::parse("https://example.test/").unwrap(),
            ),
            intercept_response: false,
            response_stage_url_match_policy:
                crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
            auth_stage_chain: None,
        },
    );

    assert_eq!(
        state.consume_pending_request_action("FETCH-AUTH"),
        Err("RequestNotFound")
    );
    assert!(
        state.has_pending_fetch_request_id_for_test("FETCH-AUTH"),
        "mismatched action must not consume the request id"
    );
    assert!(
        state.has_pending_fetch_auth_navigation_for_test("FETCH-AUTH"),
        "mismatched action must leave the auth navigation pending"
    );

    let pending = state
        .take_pending_fetch_auth_navigation("FETCH-AUTH")
        .expect("pending auth navigation should still be actionable");
    assert_eq!(pending.fetch_request_id, "FETCH-AUTH");
}

#[test]
fn unscoped_fetch_owned_auth_navigation_allows_routed_action_session() {
    let mut state = TargetFetchState::default();
    state.register_pending_fetch_auth_navigation(
        "FETCH-AUTH".to_owned(),
        PendingFetchAuthNavigation {
            owner_session_id: None,
            action_session_id: None,
            interception_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            fetch_request_id: "FETCH-AUTH".to_owned(),
            response_stage_request_id: "FETCH-AUTH".to_owned(),
            document_navigation_token: None,
            navigation: test_navigation_dispatch_state("FETCH-AUTH"),
            challenge: FetchAuthChallenge {
                origin: "https://example.test".to_owned(),
                source: "Server".to_owned(),
                scheme: "basic".to_owned(),
                realm: "test-area".to_owned(),
            },
            request_cookie_report: None,
            auth_response: PendingFetchAuthNavigation::test_auth_response(
                Url::parse("https://example.test/").unwrap(),
            ),
            intercept_response: false,
            response_stage_url_match_policy:
                crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
            auth_stage_chain: None,
        },
    );

    let pending = state
        .take_pending_fetch_auth_navigation_for_action_session("FETCH-AUTH", Some("bidi-session-1"))
        .expect("request-id-routed BiDi action should release unscoped Fetch auth navigation");
    assert_eq!(pending.fetch_request_id, "FETCH-AUTH");
    assert!(!state.has_pending_fetch_request_id_for_test("FETCH-AUTH"));

    state.register_pending_fetch_auth_navigation(
        "NETWORK-AUTH".to_owned(),
        PendingFetchAuthNavigation {
            owner_session_id: None,
            action_session_id: None,
            interception_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            fetch_request_id: "NETWORK-AUTH".to_owned(),
            response_stage_request_id: "NETWORK-AUTH".to_owned(),
            document_navigation_token: None,
            navigation: test_navigation_dispatch_state("NETWORK-AUTH"),
            challenge: FetchAuthChallenge {
                origin: "https://example.test".to_owned(),
                source: "Server".to_owned(),
                scheme: "basic".to_owned(),
                realm: "test-area".to_owned(),
            },
            request_cookie_report: None,
            auth_response: PendingFetchAuthNavigation::test_auth_response(
                Url::parse("https://example.test/").unwrap(),
            ),
            intercept_response: false,
            response_stage_url_match_policy:
                crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
            auth_stage_chain: None,
        },
    );

    assert!(
        state
            .take_pending_fetch_auth_navigation_for_action_session(
                "NETWORK-AUTH",
                Some("bidi-session-1")
            )
            .is_none(),
        "Network/BiDi-owned auth navigation should still require an explicit action owner"
    );
    assert!(state.has_pending_fetch_request_id_for_test("NETWORK-AUTH"));
}

#[test]
fn active_target_state_groups_runtime_fetch_and_owner_state() {
    let mut context =
        BrowserContext::new_with_page_for_test("BID-active-owner", "TID-active-owner");

    assert!(!context.active_page_state().runtime_slot.has_loaded_page());
    assert!(!context.active_page_state().fetch_owner.is_enabled());
    assert!(context.active_page_state().owner_state.is_default());

    context
        .active_page_state_mut()
        .runtime_slot
        .enable_primary_network_events();
    context.active_page_state_mut().fetch_owner.configure(
        Some("FETCH-SID".to_owned()),
        false,
        vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: Some(FetchResourceTypeFilter::Document),
            request_stage: FetchRequestStage::Response,
        }],
    );
    context
        .active_page_state_mut()
        .owner_state
        .next_document_start_script_id = 7;

    assert!(
        context
            .active_page_state()
            .runtime_slot
            .primary_network_events_enabled()
    );
    assert_eq!(
        context
            .active_page_state()
            .fetch_owner
            .matching_document_request_stage(&Url::parse("https://example.test/").unwrap()),
        Some(FetchRequestStage::Response)
    );
    assert_eq!(
        context
            .active_page_state()
            .owner_state
            .next_document_start_script_id,
        7
    );
}

#[test]
#[should_panic(
    expected = "replace_loaded_page(None) is not a valid production transition; use clear_loaded_page_with_reason"
)]
fn replace_loaded_page_rejects_implicit_no_page_transition() {
    let mut slot = TargetRuntimeSlot::default();
    let _ = slot.replace_loaded_page(None);
}

#[test]
fn background_target_owns_fetch_state() {
    let mut target = PageTargetHost::with_url("TID-A".to_owned(), None, "about:blank".to_owned());
    assert!(target.fetch_owner.pending_state().is_empty());

    target
        .fetch_owner
        .pending_state_mut()
        .insert_pending_fetch_request_id_for_test("FETCH-1".to_owned());
    assert!(!target.fetch_owner.pending_state().is_empty());
    assert!(
        target
            .fetch_owner
            .pending_state()
            .has_pending_fetch_request_id_for_test("FETCH-1")
    );
}

#[test]
fn navigation_history_seed_entry_preserves_pending_update() {
    let mut history = TargetNavigationHistoryState::default();
    history.mark_replace_current();

    let seed_id = history.allocate_entry_id();
    history.seed_entry(PageNavigationHistoryEntry {
        id: seed_id,
        url: "https://example.test/seed".to_owned(),
        user_typed_url: "https://example.test/seed".to_owned(),
        title: "seed".to_owned(),
        transition_type: "typed".to_owned(),
        document_sequence_number: None,
    });

    let reloaded_id = history.allocate_entry_id();
    history.record_loaded_entry(PageNavigationHistoryEntry {
        id: reloaded_id,
        url: "https://example.test/reloaded".to_owned(),
        user_typed_url: "https://example.test/reloaded".to_owned(),
        title: "reloaded".to_owned(),
        transition_type: "typed".to_owned(),
        document_sequence_number: None,
    });

    let (current_index, entries) = history.snapshot();
    assert_eq!(current_index, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "https://example.test/reloaded");
    assert_eq!(entries[0].user_typed_url, "https://example.test/seed");
    assert_eq!(entries[0].transition_type, "reload");
}

#[test]
fn initial_empty_document_seeds_browser_navigation_history_metadata() {
    let mut owner = TargetOwnerState::default();

    owner.begin_initial_empty_document(
        "TID-initial".to_owned(),
        "about:blank".to_owned(),
        None,
        None,
    );

    let (current_index, entries) = owner.navigation_history_snapshot(None);
    assert_eq!(current_index, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "about:blank");
    assert_eq!(entries[0].user_typed_url, "about:blank");
    assert_eq!(entries[0].transition_type, "auto_toplevel");
}

#[test]
fn direct_target_initial_url_replaces_empty_document_history_entry() {
    let mut owner = TargetOwnerState::default();

    owner.begin_initial_empty_document(
        "TID-direct".to_owned(),
        "about:blank".to_owned(),
        None,
        None,
    );
    owner.mark_next_navigation_history_replace_initial_empty_document();
    owner.record_loaded_page_navigation_history((
        "https://example.test/direct".to_owned(),
        "direct".to_owned(),
    ));

    let (current_index, entries) = owner.navigation_history_snapshot(None);
    assert_eq!(current_index, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "https://example.test/direct");
    assert_eq!(entries[0].user_typed_url, "https://example.test/direct");
    assert_eq!(entries[0].title, "direct");
    assert_eq!(entries[0].transition_type, "auto_toplevel");
}

#[test]
fn navigation_history_prune_rejects_only_pending_existing_entry_traversal() {
    let mut history = TargetNavigationHistoryState::default();
    let initial_id = history.allocate_entry_id();
    history.seed_entry(PageNavigationHistoryEntry {
        id: initial_id,
        url: "https://example.test/initial".to_owned(),
        user_typed_url: "https://example.test/initial".to_owned(),
        title: "initial".to_owned(),
        transition_type: "typed".to_owned(),
        document_sequence_number: None,
    });
    assert!(history.record_same_document_update(
        "https://example.test/pushed".to_owned(),
        "pushed".to_owned(),
        SameDocumentHistoryUpdate::Push,
    ));
    let pushed_id = history.snapshot().1[1].id;

    history.mark_replace_current();
    assert!(
        history.can_prune_all_but_current(),
        "a new pending reload/replace entry must survive pruning"
    );
    assert!(history.prune_all_but_current());
    let (current_index, entries) = history.snapshot();
    assert_eq!(current_index, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, pushed_id);

    history.mark_traverse_to_entry(pushed_id);
    assert!(
        !history.can_prune_all_but_current(),
        "pending traversal to an existing history index cannot be pruned"
    );
    assert!(!history.prune_all_but_current());
}

#[test]
fn navigation_history_traversal_reuses_same_document_entries() {
    let mut history = TargetNavigationHistoryState::default();
    let initial_id = history.allocate_entry_id();
    history.seed_entry(PageNavigationHistoryEntry {
        id: initial_id,
        url: "https://example.test/page".to_owned(),
        user_typed_url: "https://example.test/page".to_owned(),
        title: "page".to_owned(),
        transition_type: "typed".to_owned(),
        document_sequence_number: None,
    });
    assert!(history.record_same_document_update(
        "https://example.test/page?state=pushed".to_owned(),
        "page".to_owned(),
        SameDocumentHistoryUpdate::Push,
    ));

    let (_, entries) = history.snapshot();
    let pushed_id = entries[1].id;
    assert_eq!(
        entries[0].document_sequence_number, entries[1].document_sequence_number,
        "pushState entries must retain the current document sequence"
    );
    assert_eq!(entries[1].user_typed_url, "https://example.test/page");
    assert_eq!(entries[1].transition_type, "link");

    assert!(history.record_same_document_update(
        "https://example.test/page".to_owned(),
        "page".to_owned(),
        SameDocumentHistoryUpdate::Traverse { delta: -1 },
    ));
    let (current_index, entries) = history.snapshot();
    assert_eq!(current_index, 0);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, initial_id);
    assert_eq!(entries[1].id, pushed_id);

    assert!(history.record_same_document_update(
        "https://example.test/page?state=pushed".to_owned(),
        "page".to_owned(),
        SameDocumentHistoryUpdate::Traverse { delta: 1 },
    ));
    let (current_index, entries) = history.snapshot();
    assert_eq!(current_index, 1);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].id, pushed_id);
}

#[test]
fn background_target_keeps_owner_state_independent_from_network_artifacts() {
    let mut target = PageTargetHost::with_url("TID-A".to_owned(), None, "about:blank".to_owned());

    let mut owner_state = ParkedTargetOwnerState {
        next_document_start_script_id: 9,
        isolated_worlds: vec![IsolatedWorldDefinition {
            name: "utility".to_owned(),
            grant_universal_access: false,
        }],
        ..Default::default()
    };
    owner_state
        .runtime_observable_state
        .mark_emitted_console_counts(HashMap::from([(42, 3)]));
    owner_state
        .runtime_observable_state
        .mark_emitted_exception_entries(2);
    owner_state.log_storage_state.clear_at(2, 3);
    owner_state
        .console_output_state
        .advance_console_domain_to_current(3, 1);
    target.owner_state = owner_state;

    assert!(
        target
            .runtime_slot
            .collected_network_data_artifacts()
            .is_empty()
    );
    let restored = std::mem::take(&mut target.owner_state);
    assert_eq!(
        restored
            .runtime_observable_state
            .emitted_console_entries_for_context(42, None),
        3
    );
    assert_eq!(
        restored
            .runtime_observable_state
            .emitted_exception_entries(),
        2
    );
    assert_eq!(restored.log_storage_state.lifecycle_start(), 2);
    assert_eq!(restored.log_storage_state.network_start(), 3);
    assert!(
        !restored
            .console_output_state
            .has_unemitted_console_domain(3, 1)
    );
    assert!(
        restored
            .console_output_state
            .has_unemitted_console_domain(3, 2)
    );
    assert_eq!(restored.next_document_start_script_id, 9);
    assert_eq!(restored.isolated_worlds.len(), 1);
    assert!(target.owner_state.is_default());
}

#[test]
fn committed_document_navigation_state_clears_document_local_runtime_state() {
    let mut owner_state = TargetOwnerState::default();
    owner_state.document_start_scripts.push((
        "1".to_owned(),
        DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source: "globalThis.fromPreload = true;".to_owned(),
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        },
    ));
    owner_state.insert_attached_child_frame_id("FRAME-child".to_owned());
    owner_state
        .runtime_observable_state
        .mark_emitted_exception_entries(3);
    owner_state
        .console_output_state
        .advance_console_domain_to_current(7, 2);
    owner_state.clear_committed_document_navigation_state();

    assert_eq!(
        owner_state
            .runtime_observable_state
            .emitted_exception_entries(),
        0,
        "new document navigation must reset Runtime observable cursors"
    );
    assert!(
        owner_state
            .console_output_state
            .has_unemitted_console_domain(7, 2),
        "new document navigation must reset Console domain cursors"
    );
    assert_eq!(
        owner_state.document_start_scripts.len(),
        1,
        "pre-document scripts are target-owner state, not document-local cursor state"
    );
}

#[test]
fn devtools_session_runtime_context_clear_resets_child_default_emission_cursor() {
    let mut state = DevToolsSessionState::default();
    state.mark_child_default_execution_context_id_emitted(99);
    state.clear_runtime_remote_object_tracking();

    assert!(
        !state.has_emitted_child_default_execution_context_id(99),
        "Runtime context clear must forget session-local child-default replay cursors"
    );
}

#[test]
fn background_target_mutates_owner_state_in_place() {
    let mut target = PageTargetHost::with_url("TID-A".to_owned(), None, "about:blank".to_owned());

    let identifier = {
        let owner_state = &mut target.owner_state;
        owner_state.next_document_start_script_id =
            owner_state.next_document_start_script_id.wrapping_add(1);
        let identifier = owner_state.next_document_start_script_id.to_string();
        owner_state.document_start_scripts.push((
            identifier.clone(),
            DocumentStartScript {
                registry_key: None,
                devtools_session: None,
                source: "globalThis.fromPreload = true;".to_owned(),
                world_name: Some("utility".to_owned()),
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
        ));
        owner_state
            .console_output_state
            .advance_console_domain_to_current(7, 3);
        identifier
    };

    assert_eq!(identifier, "1");
    let parked_state = &target.owner_state;
    assert_eq!(parked_state.next_document_start_script_id, 1);
    assert_eq!(parked_state.document_start_scripts.len(), 1);
    assert!(
        !parked_state
            .console_output_state
            .has_unemitted_console_domain(7, 3)
    );

    {
        let owner_state = &mut target.owner_state;
        owner_state.next_document_start_script_id = 0;
        owner_state.document_start_scripts.clear();
        owner_state.clear_observable_output_state();
    }

    assert!(
        target.owner_state.is_default(),
        "resetting owner state must leave the host's embedded state at its default"
    );
}

fn stored_cookie(name: &str, value: &str) -> moli_cookie_jar::StoredCookie {
    moli_cookie_jar::StoredCookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "example.com".to_owned(),
        host_only: false,
        path: "/".to_owned(),
        secure: false,
        http_only: false,
        expires: None,
        same_site: moli_cookie_jar::StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        source_scheme: moli_cookie_jar::StoredCookieSourceScheme::NonSecure,
        source_port: -1,
        creation_index: 0,
        last_access_index: 0,
    }
}

#[test]
fn seed_initial_cookies_keeps_store_available_after_lock_holder_panic() {
    let cookie_store = new_shared_browser_cookie_store();
    let panicking_store = cookie_store.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _guard = panicking_store.lock();
        panic!("panic while holding cookie store lock");
    }));

    super::browser_context::seed_initial_cookies(
        &cookie_store,
        vec![stored_cookie("sid", "seeded")],
    );

    let cookies = cookie_store.lock().cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "sid");
}

#[test]
fn browser_context_clears_origin_site_data_through_partition_owner() {
    use super::browser_context::SiteDataClearOptions;

    let mut context = BrowserContext::new("CTX-site-data".to_owned());
    let origin = Url::parse("https://app.example.com/page").unwrap();
    let storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(&origin, None)
        .serialized_storage_key();
    let sibling_storage_key = moli_storage_key::MoliStorageKey::first_party_from_url(
        &Url::parse("https://cdn.example.com/page").unwrap(),
        None,
    )
    .serialized_storage_key();

    context.store_response_cookie_headers_for_test(
        &origin,
        &[("set-cookie".to_owned(), "host=1; Path=/; Secure".to_owned())],
    );
    context.store_response_cookie_headers_for_test(
        &Url::parse("https://cdn.example.com/page").unwrap(),
        &[(
            "set-cookie".to_owned(),
            "sibling=1; Path=/; Secure".to_owned(),
        )],
    );
    assert_eq!(context.snapshot_cookies().len(), 2);
    {
        let mut store = context.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "1"));
        assert!(store.set_item(&sibling_storage_key, "local", "2"));
    }

    context
        .clear_site_data_for_origin(
            &origin,
            SiteDataClearOptions {
                cookies: true,
                local_storage: true,
                indexed_db: false,
                storage_buckets: false,
                http_cache: false,
            },
        )
        .expect("site data clear should succeed");

    let cookies = context.snapshot_cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "sibling");
    let mut store = context.web_storage_store_for_test().lock();
    assert_eq!(store.get_item(&storage_key, "local"), None);
    assert_eq!(
        store.get_item(&sibling_storage_key, "local"),
        Some("2".to_owned())
    );
}

#[test]
fn navigation_request_identity_rejects_stale_tokens_without_ordering() {
    let mut context = BrowserContext::new("CTX-nav".to_owned());
    context.set_active_target_id("TID-nav");
    context.attach_active_session("SID-nav");

    let first = context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should create first navigation token");
    assert!(context.accepts_pending_document_navigation_event(&first));

    let second = context
        .start_document_navigation_for_active_target("LOADER-2".to_owned())
        .expect("active target should create second navigation token");
    assert!(context.accepts_pending_document_navigation_event(&second));
    assert!(
        !context.accepts_pending_document_navigation_event(&first),
        "a new navigation request identity must make previous events stale"
    );
    assert_ne!(second.request_id, first.request_id);
    assert_eq!(second.target_id, "TID-nav");
    assert_eq!(second.loader_id, "LOADER-2");
}

#[test]
fn clearing_document_navigation_rejects_late_events() {
    let mut context = BrowserContext::new("CTX-nav".to_owned());
    context.set_active_target_id("TID-nav");
    let token = context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should create navigation token");

    context.clear_document_navigation_state_for_active_target();

    assert!(
        !context.accepts_pending_document_navigation_event(&token),
        "closed or reset targets must reject late navigation events"
    );
}

#[test]
fn active_target_initial_empty_document_record_tracks_navigation_lifecycle() {
    let mut context = BrowserContext::new("CTX-initial-active".to_owned());
    context.set_active_target_id("TID-initial-active");
    context.set_target_url("about:blank#active".to_owned());
    context.begin_active_target_initial_empty_document("about:blank#active".to_owned());

    assert_eq!(
        context
            .active_page_state()
            .runtime_slot
            .moli_memory_diagnostics()["loadedPageAbsenceReason"],
        json!("initial-document-page-build-pending"),
        "active initial document target should make the missing Page reason explicit"
    );
    assert_eq!(
        context.moli_memory_diagnostics()["isolateScope"]["pendingDocumentPageBuildCount"],
        json!(1),
        "active initial document target without a Page should count as pending page build"
    );

    let initial = context
        .active_page_state()
        .owner_state
        .initial_empty_document_state()
        .expect("active target should record initial empty document");
    assert_eq!(initial.target_id(), "TID-initial-active");
    assert_eq!(initial.initial_url(), "about:blank#active");
    assert!(initial.is_on_initial_empty_document());
    assert!(!initial.materialized());
    assert!(!initial.pending_cross_document_navigation());

    context.mark_target_initial_empty_document_materialized("TID-initial-active");
    assert!(
        context
            .active_page_state()
            .owner_state
            .initial_empty_document_state()
            .expect("initial empty document state")
            .materialized()
    );

    let token = context
        .start_document_navigation_for_active_target("LOADER-initial-active".to_owned())
        .expect("active target should start document navigation");
    let pending = context
        .active_page_state()
        .owner_state
        .initial_empty_document_state()
        .expect("initial empty document state");
    assert!(pending.is_on_initial_empty_document());
    assert!(pending.pending_cross_document_navigation());

    context.clear_pending_document_navigation_for_target_if_loader_matches(
        Some("TID-initial-active"),
        "LOADER-initial-active",
    );
    let cleared = context
        .active_page_state()
        .owner_state
        .initial_empty_document_state()
        .expect("initial empty document state");
    assert!(cleared.is_on_initial_empty_document());
    assert!(!cleared.pending_cross_document_navigation());

    let committed = context
        .start_document_navigation_for_active_target("LOADER-initial-active-2".to_owned())
        .expect("active target should restart document navigation");
    context.commit_document_navigation_if_matches(&committed);
    let exited = context
        .active_page_state()
        .owner_state
        .initial_empty_document_state()
        .expect("initial empty document state");
    assert!(exited.exited());
    assert!(!exited.pending_cross_document_navigation());
    assert!(!exited.is_on_initial_empty_document());
    assert!(
        !context.accepts_pending_document_navigation_event(&token),
        "cleared navigation must stay rejected"
    );
}

#[test]
fn materialized_active_initial_empty_document_requires_loaded_page() {
    let mut context = BrowserContext::new("CTX-initial-active-invariant".to_owned());
    context.set_active_target_id("TID-initial-active-invariant");
    context.set_target_url("about:blank#active".to_owned());
    context.begin_active_target_initial_empty_document("about:blank#active".to_owned());

    assert_eq!(
        context.assert_target_materialized_initial_empty_document_has_page(
            "TID-initial-active-invariant"
        ),
        Ok(()),
        "pending initial document Page build is allowed before materialization"
    );

    context.mark_target_initial_empty_document_materialized("TID-initial-active-invariant");

    assert_eq!(
        context
            .assert_target_materialized_initial_empty_document_has_page(
                "TID-initial-active-invariant"
            )
            .expect_err("materialized active initial document must have a Page"),
        "TargetInitialEmptyDocumentMissingPage: target TID-initial-active-invariant has materialized current initial empty document without loaded Page"
    );
}

#[test]
fn background_target_initial_empty_document_record_tracks_navigation_lifecycle() {
    let mut context = BrowserContext::new("CTX-initial-bg".to_owned());
    context.stage_background_target(
        "TID-initial-bg".to_owned(),
        Some("SID-initial-bg".to_owned()),
        "about:blank#background".to_owned(),
        None,
        None,
    );

    assert_eq!(
        context
            .background_target_at(0)
            .unwrap()
            .runtime_slot()
            .moli_memory_diagnostics()["loadedPageAbsenceReason"],
        json!("initial-document-page-build-pending"),
        "background initial document target should make the missing Page reason explicit"
    );
    assert_eq!(
        context.moli_memory_diagnostics()["isolateScope"]["pendingDocumentPageBuildCount"],
        json!(1),
        "background initial document target without a Page should count as pending page build"
    );

    let initial = context
        .parked_target_owner_state("TID-initial-bg")
        .and_then(TargetOwnerState::initial_empty_document_state)
        .expect("background target should record initial empty document");
    assert_eq!(initial.target_id(), "TID-initial-bg");
    assert_eq!(initial.initial_url(), "about:blank#background");
    assert!(initial.is_on_initial_empty_document());
    assert!(!initial.materialized());

    context.mark_target_initial_empty_document_materialized("TID-initial-bg");
    assert!(
        context
            .parked_target_owner_state("TID-initial-bg")
            .and_then(TargetOwnerState::initial_empty_document_state)
            .expect("initial empty document state")
            .materialized()
    );

    let token = context
        .start_document_navigation_for_target("TID-initial-bg", "LOADER-initial-bg".to_owned())
        .expect("background target should start document navigation");
    assert!(
        context
            .parked_target_owner_state("TID-initial-bg")
            .and_then(TargetOwnerState::initial_empty_document_state)
            .expect("initial empty document state")
            .pending_cross_document_navigation()
    );

    context.commit_document_navigation_if_matches(&token);
    let exited = context
        .parked_target_owner_state("TID-initial-bg")
        .and_then(TargetOwnerState::initial_empty_document_state)
        .expect("initial empty document state");
    assert!(exited.exited());
    assert!(!exited.pending_cross_document_navigation());
    assert!(!exited.is_on_initial_empty_document());
}

#[test]
fn materialized_background_initial_empty_document_requires_loaded_page() {
    let mut context = BrowserContext::new("CTX-initial-bg-invariant".to_owned());
    context.stage_background_target(
        "TID-initial-bg-invariant".to_owned(),
        Some("SID-initial-bg-invariant".to_owned()),
        "about:blank#background".to_owned(),
        None,
        None,
    );

    assert_eq!(
        context
            .assert_target_materialized_initial_empty_document_has_page("TID-initial-bg-invariant"),
        Ok(()),
        "pending initial document Page build is allowed before materialization"
    );

    context.mark_target_initial_empty_document_materialized("TID-initial-bg-invariant");

    assert_eq!(
        context
            .assert_target_materialized_initial_empty_document_has_page("TID-initial-bg-invariant")
            .expect_err("materialized background initial document must have a Page"),
        "TargetInitialEmptyDocumentMissingPage: target TID-initial-bg-invariant has materialized current initial empty document without loaded Page"
    );
}

#[test]
fn committed_document_navigation_accepts_matching_late_body_only_until_next_navigation() {
    let mut context = BrowserContext::new("CTX-nav".to_owned());
    context.set_active_target_id("TID-nav");

    let first = context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should create first navigation token");
    assert!(
        context.accepts_document_body_completion_event(&first),
        "pending body completion should be accepted for the matching token"
    );
    context.commit_document_navigation_if_matches(&first);
    assert!(
        context.accepts_document_body_completion_event(&first),
        "late body completion should be accepted after the matching document commit"
    );

    let second = context
        .start_document_navigation_for_active_target("LOADER-2".to_owned())
        .expect("active target should create second navigation token");
    assert!(
        !context.accepts_document_body_completion_event(&first),
        "starting a newer navigation must make the previous body completion stale"
    );
    assert!(context.accepts_document_body_completion_event(&second));
}
