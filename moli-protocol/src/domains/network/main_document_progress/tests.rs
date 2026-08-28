use crate::devtools_runtime::{AutomationEvent, DevToolsNetworkResourceType};
use moli_cookie_jar::{
    StoredCookieQueryReport, StoredCookieSetRejectionReason, StoredCookieSetStatus,
};
use moli_fetch::{
    NegotiatedHttpVersion, NetworkExchangeObservation, NetworkObservationJournal,
    NetworkRequestObservation, NetworkResponseObservation, RedirectInfo,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::unbounded_channel;
use url::Url;

use super::gate::{
    MainDocumentFailedNavigationProgressSource, MainDocumentProgressDrain,
    MainDocumentProgressEventBatch, MainDocumentProgressOutputBoundary,
    MainDocumentProgressOutputTarget, MainDocumentProgressPhase, MainDocumentProgressSource,
    MainDocumentProgressSourceKind,
};
use super::*;

fn completed_events() -> CompletedMainDocumentNetworkEvents {
    CompletedMainDocumentNetworkEvents::new(
        "GET".to_owned(),
        vec![("Accept".to_owned(), "text/html".to_owned())],
        None,
        200,
        vec![("Content-Type".to_owned(), "text/html".to_owned())],
        Vec::new(),
        Vec::new(),
        false,
        false,
    )
}

fn observation_journal(
    exchanges: Vec<(Vec<(String, String)>, u16, Vec<(String, String)>)>,
) -> NetworkObservationJournal {
    NetworkObservationJournal::from_exchanges(
        exchanges
            .into_iter()
            .map(|(request_headers, status, response_headers)| {
                NetworkExchangeObservation::new(
                    NetworkRequestObservation::new(request_headers),
                    Some(NetworkResponseObservation::new(status, response_headers)),
                )
            })
            .collect(),
    )
}

fn completed_progress_context() -> CompletedMainDocumentProgressContext {
    CompletedMainDocumentProgressContext::new(
        vec![Some("SID-1".to_owned())],
        Some("REQ-1".to_owned()),
        false,
        Url::parse("http://example.test/start").unwrap(),
        "GET".to_owned(),
        None,
        vec![("Accept".to_owned(), "text/html".to_owned())],
        "LOADER-1".to_owned(),
        "FRAME-1".to_owned(),
        12.5,
    )
}

fn live_progress_source() -> (
    MainDocumentLiveNetworkProgressSource,
    tokio::sync::mpsc::UnboundedReceiver<crate::conn::BackgroundProtocolEvent>,
) {
    let (sender, receiver) = unbounded_channel();
    (
        MainDocumentLiveNetworkProgressSource {
            sender: Some(sender),
            progress_queue: super::gate::MainDocumentProgressQueueHandle::from_source(
                MainDocumentProgressSource::streaming(),
            ),
            session_ids: vec![Some("SID-1".to_owned())],
            request_id: "REQ-1".to_owned(),
            loader_id: "LOADER-1".to_owned(),
            frame_id: "FRAME-1".to_owned(),
            timestamp: 12.5,
            initial_request_headers: vec![("Accept".to_owned(), "text/html".to_owned())],
            initial_request_cookie_report: None,
        },
        receiver,
    )
}

#[test]
fn failed_initial_transport_emits_request_extra_info_from_observed_headers() {
    let (live_source, mut receiver) = live_progress_source();
    let source = MainDocumentBodyProgressSource::from_live_source(
        Some(live_source),
        MainDocumentResponseVisibility::Immediate,
    );
    let journal =
        NetworkObservationJournal::from_request_observation(NetworkRequestObservation::new(vec![
            ("Host".to_owned(), "example.test".to_owned()),
            ("Accept-Encoding".to_owned(), "gzip".to_owned()),
        ]));

    source.emit_failed_initial_request_extra_info(&journal);

    let event = receiver
        .try_recv()
        .expect("failed request ExtraInfo should be emitted")
        .into_protocol_message();
    assert_eq!(event["method"], json!("Network.requestWillBeSentExtraInfo"));
    assert_eq!(event["params"]["requestId"], json!("REQ-1"));
    assert_eq!(event["params"]["headers"]["Host"], json!("example.test"));
    assert_eq!(event["params"]["headers"]["Accept-Encoding"], json!("gzip"));
    assert_eq!(event["params"]["associatedCookies"], json!([]));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn failed_redirect_transport_emits_completed_hop_and_final_request_extra_info() {
    let (live_source, mut receiver) = live_progress_source();
    let source = MainDocumentBodyProgressSource::from_live_source(
        Some(live_source),
        MainDocumentResponseVisibility::Immediate,
    );
    let final_url = Url::parse("http://example.test/reset").unwrap();
    let redirect = RedirectInfo {
        from_url: Url::parse("http://example.test/start").unwrap(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("Location".to_owned(), final_url.to_string())],
        network_extra_info_available: true,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: true,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };
    let journal = NetworkObservationJournal::from_exchanges(vec![
        NetworkExchangeObservation::new(
            NetworkRequestObservation::new(vec![(
                "X-Request-Hop".to_owned(),
                "initial".to_owned(),
            )]),
            Some(NetworkResponseObservation::new(
                302,
                vec![("X-Response-Hop".to_owned(), "redirect".to_owned())],
            )),
        ),
        NetworkExchangeObservation::new(
            NetworkRequestObservation::new(vec![("X-Request-Hop".to_owned(), "final".to_owned())]),
            None,
        ),
    ]);

    source.emit_failed_request_progress(
        "GET",
        None,
        &[("Accept".to_owned(), "text/html".to_owned())],
        &[redirect],
        &journal,
    );

    let messages = std::iter::from_fn(|| receiver.try_recv().ok())
        .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .map(|message| message["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "Network.requestWillBeSentExtraInfo",
            "Network.requestWillBeSent",
            "Network.responseReceivedExtraInfo",
            "Network.requestWillBeSentExtraInfo",
        ]
    );
    assert_eq!(
        messages[0]["params"]["headers"]["X-Request-Hop"],
        json!("initial")
    );
    assert_eq!(messages[1]["params"]["request"]["url"], json!(final_url));
    assert_eq!(
        messages[1]["params"]["redirectResponse"]["status"],
        json!(302)
    );
    assert_eq!(messages[1]["params"]["redirectHasExtraInfo"], json!(true));
    assert_eq!(messages[2]["params"]["statusCode"], json!(302));
    assert_eq!(
        messages[2]["params"]["headers"]["X-Response-Hop"],
        json!("redirect")
    );
    assert_eq!(
        messages[3]["params"]["headers"]["X-Request-Hop"],
        json!("final")
    );
}

fn protocol_messages_from_background_events(
    events: Vec<crate::conn::BackgroundProtocolEvent>,
) -> Vec<Value> {
    events
        .into_iter()
        .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
        .collect()
}

fn drain_into_background_events(
    drain: &mut MainDocumentProgressDrain,
) -> Vec<crate::conn::BackgroundProtocolEvent> {
    let mut events = Vec::new();
    {
        let mut output = MainDocumentProgressOutputTarget::background_events(&mut events);
        drain.drain_into_output_target(&mut output);
    }
    events
}

fn drain_into_protocol_messages(drain: &mut MainDocumentProgressDrain) -> Vec<Value> {
    protocol_messages_from_background_events(drain_into_background_events(drain))
}

fn drain_gate_until_response_metadata_visible_into_protocol_messages(
    gate: &mut MainDocumentProgressGate,
) -> Vec<Value> {
    let mut events = Vec::new();
    MainDocumentProgressBackgroundEventBarrier::drain_until_response_metadata_visible(
        &mut events,
        gate,
    );
    protocol_messages_from_background_events(events)
}

fn drain_gate_until_body_finished_visible_into_protocol_messages(
    gate: &mut MainDocumentProgressGate,
) -> Vec<Value> {
    let mut events = Vec::new();
    MainDocumentProgressBackgroundEventBarrier::drain_until_body_finished_visible(
        &mut events,
        gate,
    );
    protocol_messages_from_background_events(events)
}

fn response_progress_batch_for_output_target() -> MainDocumentProgressEventBatch {
    MainDocumentProgressEventBatch::from_events(vec![
        MainDocumentNavigationProgressEvent::ResponseReceived {
            target: MainDocumentProgressEventTarget {
                session_ids: vec![None, Some("SID-2".to_owned())],
                request_id: "REQ-2".to_owned(),
                loader_id: "LOADER-2".to_owned(),
                frame_id: "FRAME-2".to_owned(),
                timestamp: 21.0,
            },
            final_url: Url::parse("http://example.test/target").unwrap(),
            status: 200,
            headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
            cookie_set_reports: Vec::new(),
            extra_info_status: 200,
            extra_info_headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
            network_extra_info_available: false,
            emit_extra_info: false,
            encoded_data_length: 29,
            from_cache: false,
            negotiated_http_version: None,
            has_extra_info: false,
        },
    ])
}

#[test]
fn pending_document_progress_transfer_keeps_body_unmaterialized() {
    let transfer = CompletedDocumentProgressTransfer::new_pending_body(
        MainDocumentBodyNetworkProgress::StreamingBody,
    );

    let (body, progress) = transfer.into_parts();

    assert!(matches!(body, CompletedDocumentProgressBody::Pending));
    assert!(matches!(
        progress,
        MainDocumentBodyNetworkProgress::StreamingBody
    ));
}

#[test]
fn progress_output_target_serializes_background_targets_consistently() {
    let (sender, mut receiver) = unbounded_channel();
    let mut background = MainDocumentProgressOutputTarget::background_sender(&sender);
    background.emit_batch(response_progress_batch_for_output_target());
    drop(sender);

    let mut background_events = Vec::new();
    MainDocumentProgressOutputTarget::background_events(&mut background_events)
        .emit_batch(response_progress_batch_for_output_target());

    let mut background_parts = Vec::new();
    while let Ok(value) = receiver.try_recv() {
        background_parts.push(value.into_parts());
    }
    let background_event_parts = background_events
        .into_iter()
        .map(|event| event.into_parts())
        .collect::<Vec<_>>();
    let background_out = background_parts
        .iter()
        .map(|(message, _)| message.clone())
        .collect::<Vec<_>>();
    let background_event_out = background_event_parts
        .iter()
        .map(|(message, _)| message.clone())
        .collect::<Vec<_>>();

    assert_eq!(background_event_out, background_out);
    assert_eq!(background_out.len(), 2);
    assert!(matches!(
        background_parts[0].1,
        Some(AutomationEvent::NetworkResponseStarted(_))
    ));
    assert!(matches!(
        background_parts[1].1,
        Some(AutomationEvent::NetworkResponseStarted(_))
    ));
    assert!(matches!(
        background_event_parts[0].1,
        Some(AutomationEvent::NetworkResponseStarted(_))
    ));
    assert!(matches!(
        background_event_parts[1].1,
        Some(AutomationEvent::NetworkResponseStarted(_))
    ));
    assert_eq!(
        background_out[0]["method"],
        json!("Network.responseReceived")
    );
    assert_eq!(
        background_out[0]["params"]["response"]["encodedDataLength"],
        json!(29)
    );
    assert_eq!(background_out[1]["sessionId"], json!("SID-2"));
}

#[test]
fn progress_output_target_background_raw_events_stay_sidecar_free_without_classifier_bridge() {
    let report = StoredCookieQueryReport::default();

    let (sender, mut receiver) = unbounded_channel();
    let mut background = MainDocumentProgressOutputTarget::background_sender(&sender);
    super::emit::emit_request_will_be_sent_extra_info(
        &mut background,
        Some("SID-extra"),
        "REQ-extra",
        &[("Accept".to_owned(), "text/html".to_owned())],
        &report,
        1.25,
    );
    drop(sender);

    let mut background_events = Vec::new();
    let mut background_event_output =
        MainDocumentProgressOutputTarget::background_events(&mut background_events);
    super::emit::emit_request_will_be_sent_extra_info(
        &mut background_event_output,
        Some("SID-extra"),
        "REQ-extra",
        &[("Accept".to_owned(), "text/html".to_owned())],
        &report,
        1.25,
    );

    let event = receiver
        .try_recv()
        .expect("background extraInfo event should be sent");
    let (message, automation_event) = event.into_parts();
    let (background_event_message, background_event_sidecar) = background_events
        .pop()
        .expect("background event target should collect extraInfo event")
        .into_parts();

    assert_eq!(background_event_message, message);
    assert_eq!(
        message["method"],
        json!("Network.requestWillBeSentExtraInfo")
    );
    assert_eq!(message["sessionId"], json!("SID-extra"));
    assert!(
        automation_event.is_none(),
        "extraInfo raw protocol events should stay sidecar-free without a classifier bridge"
    );
    assert!(
        background_event_sidecar.is_none(),
        "background event target should keep the sender target sidecar semantics"
    );
}

#[test]
fn live_progress_source_serializes_through_progress_emissions() {
    let (source, mut receiver) = live_progress_source();
    let final_url = Url::parse("http://example.test/final").unwrap();
    let cookie_set_reports = vec![moli_cookie_jar::StoredCookieSetReport {
        status: StoredCookieSetStatus::Rejected(StoredCookieSetRejectionReason::Parse),
        rejection_reasons: vec![StoredCookieSetRejectionReason::Parse],
        warning_reasons: Vec::new(),
        effective_same_site: None,
    }];
    let redirect = RedirectInfo {
        from_url: Url::parse("http://example.test/start").unwrap(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("Location".to_owned(), final_url.to_string())],
        network_extra_info_available: true,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: true,
        request_cookie_report: None,
        cookie_set_reports,
        from_cache: false,
        negotiated_http_version: None,
    };
    let journal = observation_journal(vec![
        (
            vec![("Host".to_owned(), "example.test".to_owned())],
            302,
            vec![("Location".to_owned(), final_url.to_string())],
        ),
        (
            vec![("Host".to_owned(), "example.test".to_owned())],
            200,
            vec![("Content-Type".to_owned(), "text/html".to_owned())],
        ),
    ]);

    source.emit_redirect_requests(
        "GET",
        None,
        &[("Accept".to_owned(), "text/html".to_owned())],
        None,
        &[redirect],
        &journal,
        true,
    );
    source.emit_response_received(
        &final_url,
        200,
        &[("Content-Type".to_owned(), "text/html".to_owned())],
        &[],
        &journal,
        1,
        true,
        false,
        None,
    );
    source.emit_body_finished(17);

    let initial_request_extra = receiver
        .try_recv()
        .expect("initial request extra info event");
    let (initial_request_extra, initial_request_extra_sidecar) = initial_request_extra.into_parts();
    assert!(initial_request_extra_sidecar.is_none());
    assert_eq!(
        initial_request_extra["method"],
        json!("Network.requestWillBeSentExtraInfo")
    );

    let redirect_request = receiver.try_recv().expect("redirect request event");
    let (redirect_request, redirect_request_automation_event) = redirect_request.into_parts();
    assert!(matches!(
        redirect_request_automation_event,
        Some(AutomationEvent::NetworkBeforeRequestSent(_))
    ));
    assert_eq!(
        redirect_request["method"],
        json!("Network.requestWillBeSent")
    );
    assert_eq!(redirect_request["params"]["requestId"], json!("REQ-1"));
    assert_eq!(
        redirect_request["params"]["redirectResponse"]["status"],
        json!(302)
    );
    assert_eq!(
        redirect_request["params"]["redirectHasExtraInfo"],
        json!(true)
    );

    let redirect_extra_info = receiver.try_recv().expect("redirect extra info event");
    let (redirect_extra_info, redirect_extra_info_automation_event) =
        redirect_extra_info.into_parts();
    assert!(redirect_extra_info_automation_event.is_none());
    assert_eq!(
        redirect_extra_info["method"],
        json!("Network.responseReceivedExtraInfo")
    );
    assert_eq!(redirect_extra_info["params"]["statusCode"], json!(302));

    let final_request_extra = receiver.try_recv().expect("final request extra info event");
    let (final_request_extra, final_request_extra_sidecar) = final_request_extra.into_parts();
    assert!(final_request_extra_sidecar.is_none());
    assert_eq!(
        final_request_extra["method"],
        json!("Network.requestWillBeSentExtraInfo")
    );

    let response_extra = receiver
        .try_recv()
        .expect("final response extra info event");
    let (response_extra, response_extra_sidecar) = response_extra.into_parts();
    assert!(response_extra_sidecar.is_none());
    assert_eq!(
        response_extra["method"],
        json!("Network.responseReceivedExtraInfo")
    );
    assert_eq!(response_extra["params"]["statusCode"], json!(200));

    let response = receiver.try_recv().expect("response event");
    let (response, response_automation_event) = response.into_parts();
    assert!(matches!(
        response_automation_event,
        Some(AutomationEvent::NetworkResponseStarted(_))
    ));
    assert_eq!(response["method"], json!("Network.responseReceived"));
    assert_eq!(response["params"]["hasExtraInfo"], json!(true));
    assert_eq!(
        response["params"]["response"]["url"],
        json!(final_url.as_str())
    );

    let data = receiver.try_recv().expect("data received event");
    let (data, data_automation_event) = data.into_parts();
    assert!(data_automation_event.is_none());
    assert_eq!(data["method"], json!("Network.dataReceived"));
    assert_eq!(data["params"]["requestId"], json!("REQ-1"));
    assert_eq!(data["params"]["dataLength"], json!(17));
    assert_eq!(data["params"]["encodedDataLength"], json!(17));

    let finished = receiver.try_recv().expect("loading finished event");
    let (finished, finished_automation_event) = finished.into_parts();
    assert!(matches!(
        finished_automation_event,
        Some(AutomationEvent::NetworkResponseCompleted(event))
            if event.request_id.as_str() == "REQ-1"
                && event.encoded_data_length == Some(17)
    ));
    assert_eq!(finished["method"], json!("Network.loadingFinished"));
    assert_eq!(finished["params"]["encodedDataLength"], json!(17));
}

#[test]
fn completed_body_http_response_emits_correlated_empty_cookie_extra_info() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let mut events = completed_events();
    events.network_extra_info_available = true;
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let network_methods = out
        .iter()
        .filter_map(|message| message["method"].as_str())
        .filter(|method| {
            matches!(
                *method,
                "Network.requestWillBeSent"
                    | "Network.requestWillBeSentExtraInfo"
                    | "Network.responseReceivedExtraInfo"
                    | "Network.responseReceived"
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        network_methods,
        [
            "Network.requestWillBeSent",
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
            "Network.responseReceived",
        ]
    );
    let request_extra = out
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .expect("request extra info");
    assert_eq!(request_extra["params"]["associatedCookies"], json!([]));
    let response = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .expect("response event");
    assert_eq!(response["params"]["hasExtraInfo"], json!(true));
}

#[test]
fn completed_body_extra_info_uses_transport_observed_headers() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let mut events = completed_events();
    events.network_extra_info_available = true;
    events = events.with_network_observation_journal(observation_journal(vec![(
        vec![
            ("Host".to_owned(), "example.test".to_owned()),
            ("Accept-Encoding".to_owned(), "gzip, deflate".to_owned()),
        ],
        200,
        vec![("X-Raw-Response".to_owned(), "observed".to_owned())],
    )]));
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let request_extra = out
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .expect("request extra info");
    assert_eq!(
        request_extra["params"]["headers"]["Host"],
        json!("example.test")
    );
    assert_eq!(
        request_extra["params"]["headers"]["Accept-Encoding"],
        json!("gzip, deflate")
    );
    let response_extra = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceivedExtraInfo"))
        .expect("response extra info");
    assert_eq!(
        response_extra["params"]["headers"]["X-Raw-Response"],
        json!("observed")
    );
}

#[test]
fn completed_body_auth_retry_uses_initial_request_and_final_response_observations() {
    let final_url = Url::parse("http://example.test/auth").unwrap();
    let mut events = completed_events();
    events.network_extra_info_available = true;
    events = events.with_network_observation_journal(observation_journal(vec![
        (
            vec![
                ("Host".to_owned(), "example.test".to_owned()),
                ("X-Auth-Attempt".to_owned(), "initial".to_owned()),
            ],
            407,
            vec![(
                "Proxy-Authenticate".to_owned(),
                "Basic realm=\"proxy\"".to_owned(),
            )],
        ),
        (
            vec![
                ("Host".to_owned(), "example.test".to_owned()),
                ("Proxy-Authorization".to_owned(), "Basic secret".to_owned()),
                ("X-Auth-Attempt".to_owned(), "credential".to_owned()),
            ],
            200,
            vec![("X-Final-Response".to_owned(), "yes".to_owned())],
        ),
    ]));
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let request_extra = out
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .expect("request extra info");
    assert_eq!(
        request_extra["params"]["headers"]["X-Auth-Attempt"],
        json!("initial")
    );
    assert!(
        request_extra["params"]["headers"]
            .get("Proxy-Authorization")
            .is_none()
    );
    let response_extra = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceivedExtraInfo"))
        .expect("response extra info");
    assert_eq!(
        response_extra["params"]["headers"]["X-Final-Response"],
        json!("yes")
    );
    assert_eq!(response_extra["params"]["statusCode"], json!(200));
}

#[test]
fn completed_body_revalidation_keeps_raw_304_extra_info() {
    let final_url = Url::parse("http://example.test/revalidated").unwrap();
    let mut events = completed_events();
    events.network_extra_info_available = true;
    events.response_status = 200;
    events.response_headers = vec![("X-Merged-Response".to_owned(), "cached".to_owned())];
    events = events.with_network_observation_journal(observation_journal(vec![(
        vec![("If-None-Match".to_owned(), "\"v1\"".to_owned())],
        304,
        vec![("ETag".to_owned(), "\"v1\"".to_owned())],
    )]));
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let response_extra = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceivedExtraInfo"))
        .expect("response extra info");
    assert_eq!(response_extra["params"]["statusCode"], json!(304));
    assert_eq!(response_extra["params"]["headers"]["ETag"], json!("\"v1\""));

    let response = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .expect("response event");
    assert_eq!(response["params"]["response"]["status"], json!(200));
    assert_eq!(
        response["params"]["response"]["headers"]["X-Merged-Response"],
        json!("cached")
    );
}

#[test]
fn completed_body_http_service_worker_response_does_not_infer_extra_info_from_url() {
    let final_url = Url::parse("http://example.test/sw-controlled").unwrap();
    let events = completed_events();
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);

    assert!(
        !out.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Network.requestWillBeSentExtraInfo" | "Network.responseReceivedExtraInfo")
            )
        }),
        "an HTTP URL alone must not imply network-service ExtraInfo"
    );
    let response = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .expect("response event");
    assert_eq!(response["params"]["hasExtraInfo"], json!(false));
}

#[test]
fn completed_body_http_redirect_emits_correlated_no_cookie_extra_info() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let mut events = completed_events();
    events.network_extra_info_available = true;
    events.redirect_chain = vec![moli_core::page::NavigationRedirect {
        from_url: Url::parse("http://example.test/start").unwrap(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("location".to_owned(), final_url.to_string())],
        network_extra_info_available: true,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: true,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }];
    events = events.with_network_observation_journal(observation_journal(vec![
        (
            vec![("X-Request-Hop".to_owned(), "initial".to_owned())],
            302,
            vec![("X-Response-Hop".to_owned(), "redirect".to_owned())],
        ),
        (
            vec![("X-Request-Hop".to_owned(), "final".to_owned())],
            200,
            vec![("X-Response-Hop".to_owned(), "final".to_owned())],
        ),
    ]));
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let network_methods = out
        .iter()
        .filter_map(|message| message["method"].as_str())
        .filter(|method| {
            matches!(
                *method,
                "Network.requestWillBeSent"
                    | "Network.requestWillBeSentExtraInfo"
                    | "Network.responseReceivedExtraInfo"
                    | "Network.responseReceived"
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        network_methods,
        [
            "Network.requestWillBeSent",
            "Network.requestWillBeSentExtraInfo",
            "Network.requestWillBeSent",
            "Network.responseReceivedExtraInfo",
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
            "Network.responseReceived",
        ]
    );
    let redirected_request = out
        .iter()
        .filter(|message| message["method"] == json!("Network.requestWillBeSent"))
        .nth(1)
        .expect("redirected request");
    assert_eq!(
        redirected_request["params"]["redirectHasExtraInfo"],
        json!(true)
    );
    let request_extras = out
        .iter()
        .filter(|message| message["method"] == json!("Network.requestWillBeSentExtraInfo"))
        .collect::<Vec<_>>();
    assert_eq!(
        request_extras
            .iter()
            .map(|message| message["params"]["headers"]["X-Request-Hop"].clone())
            .collect::<Vec<_>>(),
        [json!("initial"), json!("final")]
    );
    let response_extras = out
        .iter()
        .filter(|message| message["method"] == json!("Network.responseReceivedExtraInfo"))
        .collect::<Vec<_>>();
    assert_eq!(
        response_extras
            .iter()
            .map(|message| message["params"]["headers"]["X-Response-Hop"].clone())
            .collect::<Vec<_>>(),
        [json!("redirect"), json!("final")]
    );
}

#[test]
fn completed_body_redirect_without_transport_extra_info_keeps_flag_false() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let mut events = completed_events();
    events.redirect_chain = vec![moli_core::page::NavigationRedirect {
        from_url: Url::parse("http://example.test/start").unwrap(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("location".to_owned(), final_url.to_string())],
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    }];
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);

    let redirect_request = out
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["redirectResponse"].is_object()
        })
        .expect("redirect requestWillBeSent");
    assert_eq!(
        redirect_request["params"]["redirectHasExtraInfo"],
        json!(false),
        "redirectHasExtraInfo must stay false without an observable HTTP response"
    );

    assert!(
        !out.iter().any(|message| {
            message["method"] == json!("Network.responseReceivedExtraInfo")
                && message["params"]["statusCode"] == json!(302)
        }),
        "a redirect without transport ExtraInfo must not enqueue responseReceivedExtraInfo"
    );
}

#[test]
fn critical_client_hint_restart_keeps_discarded_response_extra_info_separate_from_internal_307() {
    let navigation_url = Url::parse("http://example.test/start").unwrap();
    let journal = observation_journal(vec![
        (
            vec![("Sec-CH-UA".to_owned(), "\"Chromium\";v=\"145\"".to_owned())],
            403,
            vec![
                ("Accept-CH".to_owned(), "Sec-CH-UA-Arch".to_owned()),
                ("Critical-CH".to_owned(), "Sec-CH-UA-Arch".to_owned()),
            ],
        ),
        (
            vec![
                ("Sec-CH-UA".to_owned(), "\"Chromium\";v=\"145\"".to_owned()),
                ("Sec-CH-UA-Arch".to_owned(), "\"x86\"".to_owned()),
            ],
            200,
            vec![("Content-Type".to_owned(), "text/html".to_owned())],
        ),
    ]);
    let mut events = completed_events()
        .with_negotiated_http_version(Some(NegotiatedHttpVersion::Http10))
        .with_network_observation_journal(journal);
    events.network_extra_info_available = true;
    events.redirect_chain = vec![moli_core::page::NavigationRedirect {
        from_url: navigation_url.clone(),
        to_url: navigation_url.clone(),
        status: 307,
        headers: vec![("Location".to_owned(), navigation_url.to_string())],
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: Some(NegotiatedHttpVersion::Http11),
    }];

    let batches = completed_progress_context().event_batches(&events, &navigation_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));
    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let relevant = out
        .iter()
        .filter(|message| {
            matches!(
                message["method"].as_str(),
                Some(
                    "Network.requestWillBeSent"
                        | "Network.requestWillBeSentExtraInfo"
                        | "Network.responseReceivedExtraInfo"
                        | "Network.responseReceived"
                )
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        relevant
            .iter()
            .map(|message| message["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "Network.requestWillBeSent",
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
            "Network.requestWillBeSent",
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
            "Network.responseReceived",
        ]
    );
    assert_eq!(relevant[2]["params"]["statusCode"], json!(403));
    assert_eq!(
        relevant[3]["params"]["redirectResponse"]["status"],
        json!(307)
    );
    assert_eq!(
        relevant[3]["params"]["redirectResponse"]["statusText"],
        json!("Internal Redirect")
    );
    assert_eq!(
        relevant[3]["params"]["redirectResponse"]["protocol"],
        json!("http/1.1")
    );
    assert_eq!(relevant[3]["params"]["redirectHasExtraInfo"], json!(false));
    assert_eq!(
        relevant[4]["params"]["headers"]["Sec-CH-UA-Arch"],
        json!("\"x86\"")
    );
    assert_eq!(relevant[5]["params"]["statusCode"], json!(200));
    assert_eq!(relevant[6]["params"]["hasExtraInfo"], json!(true));
}

#[test]
fn completed_body_uses_negotiated_protocol_for_redirect_and_final_response() {
    let final_url = Url::parse("https://example.test/final").unwrap();
    let mut events =
        completed_events().with_negotiated_http_version(Some(NegotiatedHttpVersion::Http2));
    events.redirect_chain = vec![moli_core::page::NavigationRedirect {
        from_url: Url::parse("https://example.test/start").unwrap(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("location".to_owned(), final_url.to_string())],
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: Some(NegotiatedHttpVersion::Http10),
    }];
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);
    let redirect = out
        .iter()
        .find(|message| message["params"]["redirectResponse"].is_object())
        .expect("redirect request");
    assert_eq!(
        redirect["params"]["redirectResponse"]["protocol"],
        json!("http/1.0")
    );
    let response = out
        .iter()
        .find(|message| message["method"] == json!("Network.responseReceived"))
        .expect("final response");
    assert_eq!(response["params"]["response"]["protocol"], json!("h2"));
}

#[test]
fn cached_completed_body_emits_served_from_cache_before_response() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let mut events = completed_events();
    events.response_from_cache = true;
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    let request = drain_into_protocol_messages(&mut drain);
    assert_eq!(request.len(), 1);
    assert_eq!(request[0]["method"], json!("Network.requestWillBeSent"));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::ResponseMetadataVisible);
    let mut response_events = drain_into_background_events(&mut drain);
    assert_eq!(response_events.len(), 2);

    let (cached, cached_sidecar) = response_events.remove(0).into_parts();
    assert_eq!(
        cached,
        json!({
            "method": "Network.requestServedFromCache",
            "params": { "requestId": "REQ-1" },
            "sessionId": "SID-1"
        })
    );
    assert!(cached_sidecar.is_none());

    let (response, response_sidecar) = response_events.remove(0).into_parts();
    assert_eq!(response["method"], json!("Network.responseReceived"));
    assert_eq!(response["params"]["response"]["fromDiskCache"], json!(true));
    assert!(matches!(
        response_sidecar,
        Some(AutomationEvent::NetworkResponseStarted(event))
            if event.request_id.as_str() == "REQ-1" && event.from_cache
    ));
}

#[test]
fn cached_completed_body_redirect_emits_cache_event_before_next_request() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let mut events = completed_events();
    events.redirect_chain = vec![moli_core::page::NavigationRedirect {
        from_url: Url::parse("http://example.test/start").unwrap(),
        to_url: final_url.clone(),
        status: 302,
        headers: vec![("location".to_owned(), final_url.to_string())],
        network_extra_info_available: false,
        request_extra_info: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: true,
        negotiated_http_version: None,
    }];
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    let out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(out[1]["method"], json!("Network.requestServedFromCache"));
    assert_eq!(out[1]["params"]["requestId"], json!("REQ-1"));
    assert_eq!(out[2]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(
        out[2]["params"]["redirectResponse"]["fromDiskCache"],
        json!(true)
    );
}

#[test]
fn live_ready_progress_emissions_reuse_streaming_output_queue() {
    let mut drain = MainDocumentProgressDrain::new();

    drain.append_ready_emission(
        MainDocumentProgressPhase::ResponseReceived,
        super::gate::MainDocumentProgressEmission::new(
            MainDocumentProgressPhase::ResponseReceived,
            response_progress_batch_for_output_target(),
        ),
    );
    let out = drain_into_protocol_messages(&mut drain);

    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.responseReceived"));
    assert_eq!(out[0]["params"]["response"]["encodedDataLength"], json!(29));
    assert_eq!(out[1]["sessionId"], json!("SID-2"));

    drain.append_ready_emission(
        MainDocumentProgressPhase::BodyFinished,
        super::gate::MainDocumentProgressEmission::new(
            MainDocumentProgressPhase::BodyFinished,
            MainDocumentProgressEventBatch::from_events(vec![
                MainDocumentNavigationProgressEvent::LoadingFinished {
                    target: MainDocumentProgressEventTarget {
                        session_ids: vec![Some("SID-2".to_owned())],
                        request_id: "REQ-2".to_owned(),
                        loader_id: "LOADER-2".to_owned(),
                        frame_id: "FRAME-2".to_owned(),
                        timestamp: 21.0,
                    },
                    encoded_data_length: 29,
                },
            ]),
        ),
    );
    let out = drain_into_protocol_messages(&mut drain);

    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.dataReceived"));
    assert_eq!(out[0]["params"]["dataLength"], json!(29));
    assert_eq!(out[0]["params"]["encodedDataLength"], json!(29));
    assert_eq!(out[1]["method"], json!("Network.loadingFinished"));
    assert_eq!(out[1]["params"]["encodedDataLength"], json!(29));
    assert_eq!(
        out[1]["params"],
        json!({
            "requestId": "REQ-2",
            "timestamp": 21.0,
            "encodedDataLength": 29,
        })
    );
}

#[test]
fn progress_output_queue_buffers_body_finished_until_response_received() {
    let mut drain = MainDocumentProgressDrain::new();

    drain.append_ready_emission(
        MainDocumentProgressPhase::BodyFinished,
        super::gate::MainDocumentProgressEmission::new(
            MainDocumentProgressPhase::BodyFinished,
            MainDocumentProgressEventBatch::from_events(vec![
                MainDocumentNavigationProgressEvent::LoadingFinished {
                    target: MainDocumentProgressEventTarget {
                        session_ids: vec![Some("SID-2".to_owned())],
                        request_id: "REQ-2".to_owned(),
                        loader_id: "LOADER-2".to_owned(),
                        frame_id: "FRAME-2".to_owned(),
                        timestamp: 21.0,
                    },
                    encoded_data_length: 29,
                },
            ]),
        ),
    );

    let out = drain_into_protocol_messages(&mut drain);
    assert!(
        out.is_empty(),
        "main document terminal Network event must wait for responseReceived"
    );

    drain.append_ready_emission(
        MainDocumentProgressPhase::ResponseReceived,
        super::gate::MainDocumentProgressEmission::new(
            MainDocumentProgressPhase::ResponseReceived,
            response_progress_batch_for_output_target(),
        ),
    );

    let out = drain_into_background_events(&mut drain);
    let parts = out
        .into_iter()
        .map(crate::conn::BackgroundProtocolEvent::into_parts)
        .collect::<Vec<_>>();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0].0["method"], json!("Network.responseReceived"));
    assert_eq!(parts[1].0["method"], json!("Network.responseReceived"));
    assert_eq!(parts[2].0["method"], json!("Network.dataReceived"));
    assert_eq!(parts[2].0["params"]["requestId"], json!("REQ-2"));
    assert!(parts[2].1.is_none());
    assert_eq!(parts[3].0["method"], json!("Network.loadingFinished"));
    assert_eq!(parts[3].0["params"]["requestId"], json!("REQ-2"));
    assert!(
        matches!(
            parts[3].1.as_ref(),
            Some(AutomationEvent::NetworkResponseCompleted(event))
                if event.request_id.as_str() == "REQ-2"
                    && event.encoded_data_length == Some(29)
        ),
        "buffered terminal event must retain its typed automation sidecar"
    );

    let out = drain_into_protocol_messages(&mut drain);
    assert!(
        out.is_empty(),
        "buffered terminal event must only be released once"
    );
}

#[test]
fn completed_body_progress_queue_drains_network_events_by_milestone() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let events = completed_events();
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    let mut out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(out[0]["params"]["requestId"], json!("REQ-1"));
    assert_eq!(out[0]["params"]["loaderId"], json!("LOADER-1"));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::ResponseMetadataVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.responseReceived"));
    assert_eq!(
        out[0]["params"]["response"]["url"],
        json!(final_url.as_str())
    );

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.dataReceived"));
    assert_eq!(out[0]["params"]["dataLength"], json!(17));
    assert_eq!(out[1]["method"], json!("Network.loadingFinished"));
    assert_eq!(out[1]["params"]["encodedDataLength"], json!(17));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert!(
        out.is_empty(),
        "repeating the same milestone must not duplicate Network events"
    );
}

#[test]
fn completed_body_progress_queue_can_release_all_materialized_events_at_body_finished() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let events = completed_events();
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let out = drain_into_protocol_messages(&mut drain);

    assert_eq!(out.len(), 4);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(out[1]["method"], json!("Network.responseReceived"));
    assert_eq!(out[2]["method"], json!("Network.dataReceived"));
    assert_eq!(out[2]["params"]["dataLength"], json!(17));
    assert_eq!(out[3]["method"], json!("Network.loadingFinished"));
    assert_eq!(out[3]["params"]["encodedDataLength"], json!(17));
}

#[test]
fn completed_body_progress_drain_keeps_mark_ready_separate_from_output_drain() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let events = completed_events();
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    let mut out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::ResponseMetadataVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.responseReceived"));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.dataReceived"));
    assert_eq!(out[0]["params"]["dataLength"], json!(17));
    assert_eq!(out[1]["method"], json!("Network.loadingFinished"));
    assert_eq!(out[1]["params"]["encodedDataLength"], json!(17));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::ResponseMetadataVisible);
    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert!(
        out.is_empty(),
        "draining a second time must not replay queued progress"
    );
}

#[test]
fn progress_output_barrier_drains_source_generated_progress_before_cdp_output() {
    let final_url = Url::parse("http://example.test/final").unwrap();
    let events = completed_events();
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut gate = MainDocumentProgressGate::new(MainDocumentProgressDrain::from_source(
        MainDocumentProgressSource::completed_body(batches),
    ));

    let mut out = drain_gate_until_response_metadata_visible_into_protocol_messages(&mut gate);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(out[1]["method"], json!("Network.responseReceived"));

    out = drain_gate_until_body_finished_visible_into_protocol_messages(&mut gate);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.dataReceived"));
    assert_eq!(out[0]["params"]["dataLength"], json!(17));
    assert_eq!(out[1]["method"], json!("Network.loadingFinished"));
    assert_eq!(out[1]["params"]["encodedDataLength"], json!(17));

    out = drain_gate_until_body_finished_visible_into_protocol_messages(&mut gate);
    assert!(
        out.is_empty(),
        "body-finished milestones must be idempotent after the body is released"
    );

    let events = completed_events();
    let batches = completed_progress_context().event_batches(&events, &final_url, 17);
    let mut gate = MainDocumentProgressGate::new(MainDocumentProgressDrain::from_source(
        MainDocumentProgressSource::completed_body(batches),
    ));

    let mut out = {
        let mut events = Vec::new();
        let mut barrier =
            MainDocumentProgressBackgroundEventBarrier::background_events(&mut events, &mut gate);
        barrier.drain_progress();
        protocol_messages_from_background_events(events)
    };
    crate::domains::command_output::CommandOutputPlan::result(json!({"ok": true})).emit_into(
        &mut out,
        Some(99),
        Some("SID-1"),
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(out[1]["id"], json!(99));

    let out = drain_gate_until_body_finished_visible_into_protocol_messages(&mut gate);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0]["method"], json!("Network.responseReceived"));
    assert_eq!(out[1]["method"], json!("Network.dataReceived"));
    assert_eq!(out[2]["method"], json!("Network.loadingFinished"));
}

#[test]
fn completed_no_commit_response_progress_orders_response_before_canceled_failure() {
    let final_url = Url::parse("http://example.test/no-content").unwrap();
    let events = CompletedMainDocumentNetworkEvents::new(
        "GET".to_owned(),
        vec![("Accept".to_owned(), "text/html".to_owned())],
        None,
        204,
        vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        Vec::new(),
        Vec::new(),
        false,
        false,
    );
    let batches = completed_progress_context().no_commit_response_event_batches(
        &events,
        &final_url,
        "net::ERR_ABORTED",
    );
    let mut drain =
        MainDocumentProgressDrain::from_source(MainDocumentProgressSource::completed_body(batches));

    let mut out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::ResponseMetadataVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.responseReceived"));
    assert_eq!(out[0]["params"]["response"]["status"], json!(204));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.loadingFailed"));
    assert_eq!(out[0]["params"]["errorText"], json!("net::ERR_ABORTED"));
    assert_eq!(out[0]["params"]["canceled"], json!(true));

    let batches = completed_progress_context().no_commit_response_event_batches(
        &events,
        &final_url,
        "net::ERR_ABORTED",
    );
    let mut gate = MainDocumentProgressGate::new(MainDocumentProgressDrain::from_source(
        MainDocumentProgressSource::completed_body(batches),
    ));
    gate.make_body_finished_visible();
    let mut terminal = Vec::new();
    MainDocumentProgressBackgroundEventBarrier::background_events(&mut terminal, &mut gate)
        .drain_progress();
    let terminal = protocol_messages_from_background_events(terminal);
    assert_eq!(terminal.len(), 3);
    assert_eq!(terminal[0]["method"], json!("Network.requestWillBeSent"));
    assert_eq!(terminal[1]["method"], json!("Network.responseReceived"));
    assert_eq!(terminal[2]["method"], json!("Network.loadingFailed"));
}

#[test]
fn failed_navigation_progress_drain_prequeues_loading_failed() {
    let target = MainDocumentProgressEventTarget {
        session_ids: vec![Some("SID-1".to_owned())],
        request_id: "REQ-1".to_owned(),
        loader_id: "LOADER-1".to_owned(),
        frame_id: "FRAME-1".to_owned(),
        timestamp: 12.5,
    };
    let source = MainDocumentProgressSource::new(MainDocumentProgressSourceKind::FailedNavigation(
        Box::new(MainDocumentFailedNavigationProgressSource {
            failure_event: Some(MainDocumentNavigationProgressEvent::LoadingFailed {
                target,
                error_text: "net::ERR_FAILED".to_owned(),
            }),
        }),
    ));
    let mut drain = MainDocumentProgressDrain::from_source(source);

    let mut events = drain_into_background_events(&mut drain);
    assert_eq!(events.len(), 1);
    let (failed, failed_sidecar) = events.remove(0).into_parts();
    assert!(matches!(
        failed_sidecar,
        Some(AutomationEvent::NetworkFetchError(event))
            if event.request_id.as_str() == "REQ-1"
                && event.resource_type == Some(DevToolsNetworkResourceType::Document)
                && event.error_text.as_deref() == Some("net::ERR_FAILED")
    ));
    assert_eq!(failed["method"], json!("Network.loadingFailed"));
    assert_eq!(
        failed["params"],
        json!({
            "requestId": "REQ-1",
            "timestamp": 12.5,
            "type": "Document",
            "errorText": "net::ERR_FAILED",
            "canceled": false,
        })
    );

    let out = drain_into_protocol_messages(&mut drain);
    assert!(
        out.is_empty(),
        "failed navigation progress must not replay loadingFailed"
    );
}

#[test]
fn error_page_progress_releases_failed_before_finished_at_separate_boundaries() {
    let target = MainDocumentProgressEventTarget {
        session_ids: vec![Some("SID-1".to_owned())],
        request_id: "REQ-1".to_owned(),
        loader_id: "LOADER-1".to_owned(),
        frame_id: "FRAME-1".to_owned(),
        timestamp: 12.5,
    };
    let source = MainDocumentProgressSource::error_page(
        Some(MainDocumentNavigationProgressEvent::LoadingFailed {
            target: target.clone(),
            error_text: "net::ERR_CONNECTION_REFUSED".to_owned(),
        }),
        Some(MainDocumentNavigationProgressEvent::LoadingFinished {
            target,
            encoded_data_length: 0,
        }),
    );
    let mut drain = MainDocumentProgressDrain::from_source(source);

    assert!(drain_into_protocol_messages(&mut drain).is_empty());

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::ResponseMetadataVisible);
    let failed = drain_into_protocol_messages(&mut drain);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["method"], json!("Network.loadingFailed"));
    assert_eq!(failed[0]["params"]["requestId"], json!("REQ-1"));
    assert_eq!(failed[0]["params"]["canceled"], json!(false));

    drain.mark_output_visible_until(MainDocumentProgressOutputBoundary::BodyFinishedVisible);
    let finished = drain_into_protocol_messages(&mut drain);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["method"], json!("Network.loadingFinished"));
    assert_eq!(finished[0]["params"]["requestId"], json!("REQ-1"));
    assert_eq!(finished[0]["params"]["encodedDataLength"], json!(0));

    assert!(drain_into_protocol_messages(&mut drain).is_empty());
}

#[test]
fn aborted_navigation_progress_marks_loading_failed_canceled() {
    let target = MainDocumentProgressEventTarget {
        session_ids: vec![Some("SID-1".to_owned())],
        request_id: "REQ-1".to_owned(),
        loader_id: "LOADER-1".to_owned(),
        frame_id: "FRAME-1".to_owned(),
        timestamp: 12.5,
    };
    let source = MainDocumentProgressSource::new(MainDocumentProgressSourceKind::FailedNavigation(
        Box::new(MainDocumentFailedNavigationProgressSource {
            failure_event: Some(MainDocumentNavigationProgressEvent::LoadingFailed {
                target,
                error_text: "net::ERR_ABORTED".to_owned(),
            }),
        }),
    ));
    let mut drain = MainDocumentProgressDrain::from_source(source);

    let out = drain_into_protocol_messages(&mut drain);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["method"], json!("Network.loadingFailed"));
    assert_eq!(out[0]["params"]["errorText"], json!("net::ERR_ABORTED"));
    assert_eq!(out[0]["params"]["canceled"], json!(true));
}
