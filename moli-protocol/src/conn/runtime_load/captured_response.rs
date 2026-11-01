use anyhow::Result;

use super::*;

/// A captured response's parser inputs and exact renderer reservation. Taking
/// this value ends protocol state access; preparation owns all of its waits.
pub(crate) struct PreparedCapturedNavigationResponse {
    engine: NavigationEngine,
    page_reservation: Option<RendererDocumentPreparationTarget>,
    load_inputs: TargetNavigationLoadInputs,
    requested_url: Url,
    request_method: String,
    request_headers: moli_fetch::RequestHeaders,
    head: ResponseHead,
    body: CapturedBody,
    network_observation_journal: NetworkObservationJournal,
    body_progress_source: MainDocumentBodyProgressSource,
}

pub(crate) struct PreparedNavigationLoadError {
    engine: NavigationEngine,
    page_reservation: RendererDocumentPreparationTarget,
    load_inputs: TargetNavigationLoadInputs,
    unreachable_url: Url,
    request_method: String,
    request_headers: moli_fetch::RequestHeaders,
    error_text: String,
}

impl PreparedNavigationLoadError {
    pub(crate) async fn run(mut self) -> Result<NavigationLoadOutcome> {
        prepare_network_error_page_navigation_with_engine_async(
            &mut self.engine,
            self.page_reservation,
            &self.load_inputs,
            self.unreachable_url,
            self.request_method,
            self.request_headers,
            self.error_text,
            RendererReplyBoundary::Stage,
        )
        .await
        .context("failed to prepare network error document")
    }
}

impl PreparedCapturedNavigationResponse {
    pub(crate) async fn run(mut self) -> Result<NavigationLoadOutcome> {
        if super::super::downloads::response_headers_indicate_download(&self.head.headers) {
            complete_auxiliary_download_response(&self.load_inputs, self.head.clone());
            let body = self
                .body
                .process_in_worker(|body| body.materialize_bytes())
                .await
                .context("failed to materialize captured download body")?;
            return Ok(NavigationLoadOutcome::download(
                CdpConnection::build_download_from_raw_response(
                    self.request_method,
                    self.request_headers,
                    RawResponse::from_head_and_body(self.head, body),
                    self.network_observation_journal,
                ),
            ));
        }
        if response_status_has_no_content(self.head.status) {
            return Ok(no_content_navigation_from_response(
                self.head,
                self.request_method,
                self.request_headers,
                self.network_observation_journal,
                &self.body_progress_source,
            ));
        }
        prepare_navigation_from_captured_raw_response_with_engine_async(
            &mut self.engine,
            self.page_reservation
                .expect("a Document response must reserve its renderer Page"),
            &self.load_inputs,
            self.requested_url,
            self.request_method,
            self.request_headers,
            self.head,
            self.body,
            self.body_progress_source,
            self.network_observation_journal,
            None,
            false,
            RendererReplyBoundary::Stage,
        )
        .await
        .map(NavigationLoadOutcome::response_commit_ready)
    }
}

impl CdpConnection {
    pub(crate) fn prepare_navigation_load_error(
        &mut self,
        token: Option<&DocumentNavigationToken>,
        navigation: &NavigationDispatchState,
        error: anyhow::Error,
    ) -> Result<PreparedNavigationLoadError> {
        let Some(network_error) = error.downcast_ref::<NavigationNetworkError>() else {
            return Err(error);
        };
        let load_inputs = self.navigation_load_inputs_for_navigation(token, navigation)?;
        let engine = self.navigation_engine_handle_for_load_inputs(&load_inputs);
        let page_reservation =
            self.reserve_renderer_page_for_owner(&navigation.owner, &load_inputs, &engine);
        Ok(PreparedNavigationLoadError {
            engine,
            page_reservation,
            load_inputs,
            unreachable_url: network_error.unreachable_url.clone(),
            request_method: network_error.request_method.clone(),
            request_headers: network_error.request_headers.clone(),
            error_text: network_error.kind.error_text().to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_captured_navigation_response(
        &mut self,
        token: Option<&DocumentNavigationToken>,
        navigation: &NavigationDispatchState,
        head: ResponseHead,
        body: CapturedBody,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<PreparedCapturedNavigationResponse> {
        let load_inputs = self.navigation_load_inputs_for_navigation(token, navigation)?;
        Ok(self.prepare_captured_navigation_response_with_load_inputs(
            &navigation.owner,
            load_inputs,
            navigation.requested_url.clone(),
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            head,
            body,
            network_observation_journal,
            body_progress_source,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_synthetic_navigation_response(
        &mut self,
        token: Option<&DocumentNavigationToken>,
        navigation: &NavigationDispatchState,
        final_url: Url,
        response_status: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: CapturedBody,
        initial_request_cookie_report: Option<StoredCookieQueryReport>,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> Result<PreparedCapturedNavigationResponse> {
        let load_inputs = self.navigation_load_inputs_for_navigation(token, navigation)?;
        let head = ResponseHead {
            request_include_credentials: false,
            status_text: None,
            cookie_set_reports: load_inputs
                .store_response_cookie_reports(&final_url, &response_headers)?,
            final_url,
            status: response_status,
            headers: response_headers,
            request_cookie_report: initial_request_cookie_report,
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            cache_state: Default::default(),
            negotiated_http_version: None,
        };
        Ok(self.prepare_captured_navigation_response_with_load_inputs(
            &navigation.owner,
            load_inputs,
            navigation.requested_url.clone(),
            navigation.request_method.clone(),
            navigation.request_headers.clone(),
            head,
            response_body,
            network_observation_journal,
            body_progress_source,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_captured_navigation_response_with_load_inputs(
        &mut self,
        owner: &CommandOwnerScope,
        load_inputs: TargetNavigationLoadInputs,
        requested_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        head: ResponseHead,
        body: CapturedBody,
        network_observation_journal: NetworkObservationJournal,
        body_progress_source: MainDocumentBodyProgressSource,
    ) -> PreparedCapturedNavigationResponse {
        let engine = self.navigation_engine_handle_for_load_inputs(&load_inputs);
        let page_reservation =
            (!super::super::downloads::response_headers_indicate_download(&head.headers)
                && !response_status_has_no_content(head.status))
            .then(|| self.reserve_renderer_page_for_owner(owner, &load_inputs, &engine));
        PreparedCapturedNavigationResponse {
            engine,
            page_reservation,
            load_inputs,
            requested_url,
            request_method,
            request_headers,
            head,
            body,
            network_observation_journal,
            body_progress_source,
        }
    }
}
