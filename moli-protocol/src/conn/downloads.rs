use moli_core::browser::{
    DownloadAccessError, DownloadBody, DownloadObservation, DownloadPolicy, DownloadSnapshot,
    DownloadState, WebContentsHandle,
};
use moli_core::page::RendererPendingDownloadActivation;
use moli_fetch::Request;
use url::Url;

use super::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
    CompletedDownloadBodyArtifact, NavigationDispatchState, output::BackgroundEventSender,
};

#[cfg(test)]
#[path = "downloads/lifecycle_tests.rs"]
mod lifecycle_tests;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedDownloadActivation {
    web_contents: WebContentsHandle,
    frame_id: String,
    request_headers: Vec<(String, String)>,
    initiator_url: Option<Url>,
    policy: DownloadPolicy,
    event_route: DownloadEventRoute,
    activation: RendererPendingDownloadActivation,
}

#[derive(Debug, Eq, PartialEq)]
struct DownloadEventRoute {
    browser_observers: Vec<BrowserDownloadObserver>,
    automation_events_enabled: bool,
    page_observers: Vec<PageDownloadObserver>,
}

#[derive(Debug, Eq, PartialEq)]
struct BrowserDownloadObserver {
    session_id: Option<String>,
    subscription_generation: u64,
}

#[derive(Debug, Eq, PartialEq)]
struct PageDownloadObserver {
    session_id: Option<String>,
    subscription_generation: u64,
}

impl DownloadEventRoute {
    fn has_observers(&self) -> bool {
        self.automation_events_enabled
            || !self.browser_observers.is_empty()
            || !self.page_observers.is_empty()
    }
}

impl CdpConnection {
    pub(crate) fn prepare_download_activation_for_owner(
        &self,
        owner: &CommandOwnerScope,
        activation: RendererPendingDownloadActivation,
    ) -> Option<PreparedDownloadActivation> {
        let web_contents = self.browser_web_contents_for_owner(owner).ok()?;
        self.prepare_download_activation(owner, web_contents, activation)
    }

    pub(crate) fn prepare_download_activation(
        &self,
        owner: &CommandOwnerScope,
        web_contents: WebContentsHandle,
        activation: RendererPendingDownloadActivation,
    ) -> Option<PreparedDownloadActivation> {
        let context = self.browser_context_by_browser_id(web_contents.context())?;
        let frame_id = context
            .download_frame_id_for_web_contents(web_contents)?
            .to_owned();
        let request_headers = context.effective_extra_headers_for_target(&frame_id);
        let initiator_url = context.target_document_url(&frame_id).cloned();
        let (policy, automation_events_enabled) =
            self.download_configuration_for_browser_context(web_contents.context())?;
        let event_route = self.download_event_route(owner, automation_events_enabled);
        Some(PreparedDownloadActivation {
            web_contents,
            frame_id,
            request_headers,
            initiator_url,
            policy,
            event_route,
            activation,
        })
    }

    pub(crate) async fn handle_prepared_download_activation_background_events_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        activation: PreparedDownloadActivation,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        self.handle_prepared_download_activation_async(out, activation, true, command_context)
            .await
    }

    pub(crate) async fn handle_prepared_download_activation_inline_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        activation: PreparedDownloadActivation,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        self.handle_prepared_download_activation_async(out, activation, false, command_context)
            .await
    }

    async fn handle_prepared_download_activation_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        prepared: PreparedDownloadActivation,
        allow_background_events: bool,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        let PreparedDownloadActivation {
            web_contents,
            frame_id,
            request_headers,
            initiator_url,
            policy,
            event_route,
            activation,
        } = prepared;
        let Some(context) = self.browser_context_by_browser_id(web_contents.context()) else {
            return Ok(());
        };
        if context
            .download_frame_id_for_web_contents(web_contents)
            .is_none()
        {
            return Ok(());
        }
        let observation = if policy.behavior.is_canceled_without_download() {
            if !event_route.has_observers() {
                return Ok(());
            }
            let (url, headers) = activation
                .response
                .as_ref()
                .map(|response| (response.final_url.as_str(), response.headers.as_slice()))
                .unwrap_or((activation.url.as_str(), &[]));
            Some(DownloadObservation::denied(
                url,
                headers,
                activation.suggested_filename.as_deref(),
            )?)
        } else {
            if policy.download_path.is_none() {
                return Ok(());
            }
            if let Some(response) = activation.response {
                let url = Url::parse(&response.final_url)
                    .or_else(|_| Url::parse(&activation.url))
                    .map_err(|error| format!("invalid download url: {error}"))?;
                self.browser_context_by_browser_id_mut(web_contents.context())
                    .expect("exact download Context was resolved without yielding")
                    .start_download_response(
                        web_contents,
                        &policy,
                        url,
                        response.headers,
                        DownloadBody::Buffered(response.body),
                    )?
            } else {
                let mut request = Request::get(&activation.url)
                    .map_err(|error| format!("invalid download url: {error}"))?;
                request.request_headers = request_headers;
                request = request
                    .with_top_level_navigation_cookie_context()
                    .with_page_network_policy();
                if let Some(initiator_url) = &initiator_url {
                    request = request.with_initiator_url(initiator_url);
                }
                let fetch_defaults = self.document_fetch_defaults();
                self.browser_context_by_browser_id_mut(web_contents.context())
                    .expect("exact download Context was resolved without yielding")
                    .start_download_request(
                        web_contents,
                        fetch_defaults,
                        &policy,
                        request,
                        activation.suggested_filename,
                    )?
            }
        };
        if let Some(observation) = observation {
            self.observe_download(
                DownloadProjection {
                    frame_id,
                    event_route,
                    observation,
                    started: false,
                },
                out,
                allow_background_events,
                command_context,
            )
            .await;
        }
        Ok(())
    }

    pub(crate) async fn handle_navigation_download_response_async(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
        state: &NavigationDispatchState,
        final_url: Url,
        body_artifact: CompletedDownloadBodyArtifact,
        command_context: &mut CommandDispatchContext,
    ) -> Result<(), String> {
        let web_contents = state.web_contents;
        let Some(context) = self.browser_context_by_browser_id(web_contents.context()) else {
            return Ok(());
        };
        if context
            .download_frame_id_for_web_contents(web_contents)
            .is_none()
        {
            return Ok(());
        }
        let Some((policy, automation_events_enabled)) =
            self.download_configuration_for_browser_context(web_contents.context())
        else {
            return Ok(());
        };
        let event_route = self.download_event_route(&state.owner, automation_events_enabled);
        let (body, headers) = body_artifact.into_parts();
        let observation = self
            .browser_context_by_browser_id_mut(web_contents.context())
            .expect("exact navigation download Context was resolved without yielding")
            .start_download_response(web_contents, &policy, final_url, headers, body)?;
        if let Some(observation) = observation {
            self.observe_download(
                DownloadProjection {
                    frame_id: state.frame_id.clone(),
                    event_route,
                    observation,
                    started: false,
                },
                out,
                true,
                command_context,
            )
            .await;
        }
        Ok(())
    }

    async fn observe_download(
        &self,
        mut projection: DownloadProjection,
        out: &mut Vec<BackgroundProtocolEvent>,
        allow_background_events: bool,
        command_context: &mut CommandDispatchContext,
    ) {
        let initial = projection.observation.snapshot();
        let terminal = initial.state != DownloadState::Active;
        let events = projection.project(initial);
        if allow_background_events && let Some(sender) = self.background_event_sender() {
            let response_flush = command_context.response_flush().receiver();
            if response_flush.is_some() {
                command_context.extend_post_response_events(events);
            } else {
                send_background_download_events(&sender, events);
            }
            if !terminal {
                tokio::spawn(async move {
                    // This gates observation only. The Browser task was admitted
                    // before this wait and outlives an abandoned frontend response.
                    if !wait_for_command_response_flush(response_flush).await {
                        return;
                    }
                    while let Some(snapshot) = projection.observation.next_update().await {
                        let terminal = snapshot.state != DownloadState::Active;
                        send_background_download_events(&sender, projection.project(snapshot));
                        if terminal {
                            break;
                        }
                    }
                });
            }
        } else {
            out.extend(events);
            if !terminal {
                while let Some(snapshot) = projection.observation.next_update().await {
                    let terminal = snapshot.state != DownloadState::Active;
                    out.extend(projection.project(snapshot));
                    if terminal {
                        break;
                    }
                }
            }
        }
    }

    pub(crate) fn cancel_download(&self, guid: &str) -> Result<(), String> {
        self.browser_contexts()
            .find_map(|context| context.cancel_download(guid))
            .ok_or_else(|| "No download item found for the given GUID".to_owned())?
            .map_err(download_access_error)
    }

    pub(crate) fn start_open_download_as_stream(
        &self,
        guid: &str,
    ) -> Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, String> {
        self.browser_contexts()
            .find_map(|context| context.read_download_artifact(guid))
            .ok_or_else(|| "No download item found for the given GUID".to_owned())?
            .map_err(download_access_error)
    }

    pub(crate) fn finish_open_download_as_stream(&mut self, bytes: Vec<u8>) -> String {
        self.open_global_io_stream(bytes)
    }

    fn download_event_route(
        &self,
        owner: &CommandOwnerScope,
        automation_events_enabled: bool,
    ) -> DownloadEventRoute {
        let page_observers = self
            .page_event_session_ids_for_owner(owner)
            .into_iter()
            .filter_map(|event_session_id| {
                self.page_domain_subscription_generation_for_session_owner(
                    event_session_id.as_deref(),
                )
                .map(|subscription_generation| PageDownloadObserver {
                    session_id: event_session_id,
                    subscription_generation,
                })
            })
            .collect();
        DownloadEventRoute {
            browser_observers: self
                .download_subscriptions
                .browser_event_observers()
                .into_iter()
                .map(
                    |(session_id, subscription_generation)| BrowserDownloadObserver {
                        session_id,
                        subscription_generation,
                    },
                )
                .collect(),
            automation_events_enabled: automation_events_enabled
                || self.download_subscriptions.webdriver_bidi_events_enabled,
            page_observers,
        }
    }
}

fn download_access_error(error: DownloadAccessError) -> String {
    match error {
        DownloadAccessError::AlreadyTerminal => "Download item is no longer active",
        DownloadAccessError::InProgress => "Download item is not completed yet",
        DownloadAccessError::NoArtifact => "Download item has no readable artifact",
    }
    .to_owned()
}

struct DownloadProjection {
    frame_id: String,
    event_route: DownloadEventRoute,
    observation: DownloadObservation,
    started: bool,
}

impl DownloadProjection {
    fn project(&mut self, snapshot: DownloadSnapshot) -> Vec<BackgroundProtocolEvent> {
        let Some(metadata) = snapshot.metadata else {
            return Vec::new();
        };
        let guid = self.observation.guid();
        let mut events = Vec::new();
        if !self.started {
            events.extend(download_will_begin_events(
                &self.event_route,
                &self.frame_id,
                guid,
                &metadata.url,
                &metadata.suggested_filename,
            ));
            if snapshot.state != DownloadState::Canceled {
                events.extend(download_progress_events(
                    &self.event_route,
                    guid,
                    "inProgress",
                    0,
                    0,
                    None,
                ));
            }
            self.started = true;
        }
        match snapshot.state {
            DownloadState::Active if snapshot.received_bytes > 0 => {
                events.extend(download_progress_events(
                    &self.event_route,
                    guid,
                    "inProgress",
                    snapshot.received_bytes,
                    snapshot.total_bytes.unwrap_or(0),
                    None,
                ));
            }
            DownloadState::Active => {}
            DownloadState::Completed { artifact_path } => {
                events.extend(download_progress_events(
                    &self.event_route,
                    guid,
                    "completed",
                    snapshot.received_bytes,
                    snapshot.received_bytes,
                    Some(&artifact_path.to_string_lossy()),
                ));
            }
            DownloadState::Canceled => {
                events.extend(download_progress_events(
                    &self.event_route,
                    guid,
                    "canceled",
                    snapshot.received_bytes,
                    snapshot.received_bytes,
                    None,
                ));
            }
        }
        events
    }
}

pub(crate) fn response_headers_indicate_download(headers: &[(String, String)]) -> bool {
    moli_web_mime::response_headers_indicate_attachment_download(headers)
}

fn send_background_download_events(
    sender: &BackgroundEventSender,
    events: Vec<BackgroundProtocolEvent>,
) {
    for event in events {
        let _ = sender.send(event);
    }
}
async fn wait_for_command_response_flush(
    mut receiver: Option<tokio::sync::watch::Receiver<bool>>,
) -> bool {
    let Some(receiver) = receiver.as_mut() else {
        return true;
    };
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            return false;
        }
    }
    true
}

fn download_will_begin_events(
    event_route: &DownloadEventRoute,
    frame_id: &str,
    guid: &str,
    url: &str,
    suggested_filename: &str,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = event_route
        .page_observers
        .iter()
        .map(|observer| {
            BackgroundProtocolEvent::page_download_will_begin(
                observer.session_id.as_deref(),
                observer.subscription_generation,
                frame_id,
                guid,
                url,
                suggested_filename,
            )
        })
        .collect::<Vec<_>>();
    for observer in &event_route.browser_observers {
        events.push(download_will_begin_event(
            observer.session_id.as_deref(),
            Some(observer.subscription_generation),
            frame_id,
            guid,
            url,
            suggested_filename,
        ));
    }
    if event_route.automation_events_enabled {
        events.push(BackgroundProtocolEvent::automation_download_will_begin(
            frame_id,
            guid,
            url,
            suggested_filename,
        ));
    }
    events
}

fn download_progress_events(
    event_route: &DownloadEventRoute,
    guid: &str,
    state: &str,
    received_bytes: u64,
    total_bytes: u64,
    file_path: Option<&str>,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = event_route
        .page_observers
        .iter()
        .map(|observer| {
            BackgroundProtocolEvent::page_download_progress(
                observer.session_id.as_deref(),
                observer.subscription_generation,
                guid,
                state,
                received_bytes,
                total_bytes,
            )
        })
        .collect::<Vec<_>>();
    for observer in &event_route.browser_observers {
        events.push(download_progress_event(
            observer.session_id.as_deref(),
            Some(observer.subscription_generation),
            guid,
            state,
            received_bytes,
            total_bytes,
            file_path,
        ));
    }
    if event_route.automation_events_enabled {
        events.push(BackgroundProtocolEvent::automation_download_progress(
            guid,
            state,
            received_bytes,
            total_bytes,
            file_path,
        ));
    }
    events
}

fn download_will_begin_event(
    session_id: Option<&str>,
    subscription_generation: Option<u64>,
    frame_id: &str,
    guid: &str,
    url: &str,
    suggested_filename: &str,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::browser_download_will_begin(
        session_id,
        subscription_generation,
        frame_id,
        guid,
        url,
        suggested_filename,
    )
}

fn download_progress_event(
    session_id: Option<&str>,
    subscription_generation: Option<u64>,
    guid: &str,
    state: &str,
    received_bytes: u64,
    total_bytes: u64,
    file_path: Option<&str>,
) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::browser_download_progress(
        session_id,
        subscription_generation,
        guid,
        state,
        received_bytes,
        total_bytes,
        file_path,
    )
}

#[cfg(test)]
mod tests {
    use moli_core::browser::DownloadBehavior;
    use moli_core::page::RendererPendingDownloadActivation;

    use crate::{
        conn::{BackgroundProtocolEvent, BrowserContext, CdpConnection, CommandOwnerScope},
        devtools_runtime::AutomationEvent,
    };

    use super::{
        BrowserDownloadObserver, DownloadEventRoute, PageDownloadObserver, download_progress_event,
        download_progress_events, download_will_begin_event, download_will_begin_events,
        response_headers_indicate_download,
    };

    #[test]
    fn download_behavior_parses_cdp_tokens_with_existing_case_sensitivity() {
        assert_eq!(
            crate::conn::parse_download_behavior("default"),
            Some(DownloadBehavior::Default)
        );
        assert_eq!(
            crate::conn::parse_download_behavior("deny"),
            Some(DownloadBehavior::Deny)
        );
        assert_eq!(
            crate::conn::parse_download_behavior("allow"),
            Some(DownloadBehavior::Allow)
        );
        assert_eq!(
            crate::conn::parse_download_behavior("allowAndName"),
            Some(DownloadBehavior::AllowAndName)
        );
        assert_eq!(crate::conn::parse_download_behavior("allowandname"), None);
        assert_eq!(crate::conn::parse_download_behavior("unknown"), None);
    }

    #[test]
    fn background_download_uses_its_target_and_context_header_layers() {
        let mut connection = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-download".to_owned());
        browser_context.set_active_target_id("TID-active");
        browser_context.set_default_extra_headers(vec![(
            "X-Context-Default".to_owned(),
            "default".to_owned(),
        )]);
        browser_context.global_extra_headers =
            vec![("X-Context-Global".to_owned(), "global".to_owned())];

        assert!(browser_context.register_page_target_url_fixture(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.test/".to_owned(),
        ));
        browser_context.set_base_extra_headers_for_target(
            "TID-background",
            vec![("X-Target".to_owned(), "target".to_owned())],
        );
        connection.install_browser_context_fixture_for_test(browser_context);

        let owner = connection
            .prepare_download_activation_for_owner(
                &CommandOwnerScope::for_session("SID-background"),
                RendererPendingDownloadActivation {
                    url: "https://background.test/report.txt".to_owned(),
                    suggested_filename: None,
                    response: None,
                },
            )
            .expect("background session must resolve its Page target");
        assert_eq!(owner.frame_id, "TID-background");
        assert_eq!(
            owner.request_headers,
            [
                ("X-Context-Global".to_owned(), "global".to_owned()),
                ("X-Context-Default".to_owned(), "default".to_owned()),
                ("X-Target".to_owned(), "target".to_owned()),
            ]
        );
    }

    #[test]
    fn browser_download_will_begin_event_is_protocol_only() {
        let event = download_will_begin_event(
            None,
            None,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Browser.downloadWillBegin");
        assert_eq!(message["params"]["frameId"], "FRAME-download");
        assert_eq!(message["params"]["guid"], "GUID-download");
        assert_eq!(message["params"]["url"], "https://example.test/report.txt");
        assert_eq!(message["params"]["suggestedFilename"], "report.txt");
        assert_eq!(automation_event, None);
    }

    #[test]
    fn browser_download_progress_event_is_protocol_only() {
        let event = download_progress_event(
            None,
            None,
            "GUID-download",
            "completed",
            512,
            512,
            Some("/tmp/report.txt"),
        );

        let (message, automation_event) = event.into_parts();

        assert_eq!(message["method"], "Browser.downloadProgress");
        assert_eq!(message["params"]["guid"], "GUID-download");
        assert_eq!(message["params"]["state"], "completed");
        assert_eq!(message["params"]["receivedBytes"], 512);
        assert_eq!(message["params"]["totalBytes"], 512);
        assert_eq!(message["params"]["filePath"], "/tmp/report.txt");
        assert_eq!(automation_event, None);
    }

    #[test]
    fn automation_download_events_are_sidecar_only() {
        let will_begin = BackgroundProtocolEvent::automation_download_will_begin(
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        assert!(!will_begin.has_protocol_wire_message());
        assert_eq!(
            will_begin.download_will_begin_frame_id(),
            Some("FRAME-download")
        );
        let (_, automation_event) = will_begin.into_parts();
        let Some(AutomationEvent::BrowserDownloadWillBegin(event)) = automation_event else {
            panic!("expected typed Browser.downloadWillBegin automation event");
        };
        assert_eq!(event.frame_id.as_str(), "FRAME-download");
        assert_eq!(event.guid, "GUID-download");
        assert_eq!(event.url, "https://example.test/report.txt");
        assert_eq!(event.suggested_filename, "report.txt");

        let progress = BackgroundProtocolEvent::automation_download_progress(
            "GUID-download",
            "completed",
            512,
            512,
            Some("/tmp/report.txt"),
        );
        assert!(!progress.has_protocol_wire_message());
        let (_, automation_event) = progress.into_parts();
        let Some(AutomationEvent::BrowserDownloadProgress(event)) = automation_event else {
            panic!("expected typed Browser.downloadProgress automation event");
        };
        assert_eq!(event.guid, "GUID-download");
        assert_eq!(event.state, "completed");
        assert_eq!(event.received_bytes, 512);
        assert_eq!(event.total_bytes, 512);
        assert_eq!(event.file_path.as_deref(), Some("/tmp/report.txt"));
    }

    #[test]
    fn download_events_fan_out_to_page_and_browser_before_automation() {
        let route = DownloadEventRoute {
            browser_observers: vec![
                BrowserDownloadObserver {
                    session_id: Some("SID-browser-a".to_owned()),
                    subscription_generation: 8,
                },
                BrowserDownloadObserver {
                    session_id: Some("SID-browser-b".to_owned()),
                    subscription_generation: 9,
                },
            ],
            automation_events_enabled: true,
            page_observers: vec![PageDownloadObserver {
                session_id: Some("SID-page".to_owned()),
                subscription_generation: 7,
            }],
        };

        let events = download_will_begin_events(
            &route,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].0["method"], "Page.downloadWillBegin");
        assert_eq!(parts[0].0["sessionId"], "SID-page");
        assert_eq!(parts[0].0["params"]["frameId"], "FRAME-download");
        assert_eq!(parts[0].1, None);
        assert_eq!(parts[1].0["method"], "Browser.downloadWillBegin");
        assert_eq!(parts[1].0["sessionId"], "SID-browser-a");
        assert_eq!(parts[1].1, None);
        assert_eq!(parts[2].0["method"], "Browser.downloadWillBegin");
        assert_eq!(parts[2].0["sessionId"], "SID-browser-b");
        assert_eq!(parts[2].1, None);
        assert_eq!(parts[3].0["method"], "Moli.automationOnly");
        assert!(matches!(
            parts[3].1,
            Some(AutomationEvent::BrowserDownloadWillBegin(_))
        ));
    }

    #[test]
    fn stale_browser_observer_does_not_suppress_automation_download_event() {
        let mut conn = CdpConnection::new();
        conn.set_browser_download_events_enabled_for_session(Some("SID-browser"), true);
        let generation = conn.download_subscriptions.browser_event_observers()[0].1;
        let route = DownloadEventRoute {
            browser_observers: vec![BrowserDownloadObserver {
                session_id: Some("SID-browser".to_owned()),
                subscription_generation: generation,
            }],
            automation_events_enabled: true,
            page_observers: Vec::new(),
        };
        let events = download_will_begin_events(
            &route,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        assert_eq!(events.len(), 2);
        assert!(events[0].route_is_current(&conn));
        assert!(events[1].route_is_current(&conn));

        conn.set_browser_download_events_enabled_for_session(Some("SID-browser"), false);

        assert!(!events[0].route_is_current(&conn));
        assert!(
            events[1].route_is_current(&conn),
            "a stale CDP observer must not suppress internal WebDriver/BiDi delivery"
        );
        assert_eq!(
            events[1].download_will_begin_frame_id(),
            Some("FRAME-download")
        );
    }

    #[test]
    fn page_download_progress_omits_browser_only_file_path() {
        let route = DownloadEventRoute {
            browser_observers: vec![BrowserDownloadObserver {
                session_id: Some("SID-browser".to_owned()),
                subscription_generation: 12,
            }],
            automation_events_enabled: false,
            page_observers: vec![PageDownloadObserver {
                session_id: Some("SID-page".to_owned()),
                subscription_generation: 11,
            }],
        };

        let events = download_progress_events(
            &route,
            "GUID-download",
            "completed",
            512,
            512,
            Some("/tmp/report.txt"),
        );
        let parts = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>();

        assert_eq!(parts[0].0["method"], "Page.downloadProgress");
        assert!(
            parts[0].0["params"].get("filePath").is_none(),
            "Page.downloadProgress does not expose Browser.downloadProgress.filePath"
        );
        assert_eq!(parts[0].1, None);
        assert_eq!(parts[1].0["method"], "Browser.downloadProgress");
        assert_eq!(parts[1].0["sessionId"], "SID-browser");
        assert_eq!(parts[1].0["params"]["filePath"], "/tmp/report.txt");
        assert_eq!(parts[1].1, None);
    }

    #[test]
    fn page_download_events_do_not_depend_on_browser_events_enabled() {
        let route = DownloadEventRoute {
            browser_observers: Vec::new(),
            automation_events_enabled: false,
            page_observers: vec![PageDownloadObserver {
                session_id: Some("SID-page".to_owned()),
                subscription_generation: 13,
            }],
        };

        let events = download_will_begin_events(
            &route,
            "FRAME-download",
            "GUID-download",
            "https://example.test/report.txt",
            "report.txt",
        );
        assert_eq!(events.len(), 1);
        let message = events
            .into_iter()
            .next()
            .expect("Page observer should receive the download event")
            .into_protocol_message();
        assert_eq!(message["method"], "Page.downloadWillBegin");
        assert_eq!(message["sessionId"], "SID-page");
    }

    #[test]
    fn response_headers_indicate_download_uses_web_mime_attachment_helper() {
        assert!(response_headers_indicate_download(&[(
            "Content-Disposition".to_owned(),
            "attachment; filename=\"report.txt\"".to_owned(),
        )]));
        assert!(!response_headers_indicate_download(&[(
            "Content-Disposition".to_owned(),
            "inline; filename=\"report.txt\"".to_owned(),
        )]));
    }
}
