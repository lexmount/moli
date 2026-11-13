use super::*;

pub(super) fn native_context(
    state: &WorkerGlobalState,
    policy: &crate::document_runtime::DocumentConnectPolicySnapshot,
    redirect_state: &crate::network_host::FetchCspRedirectState,
    document_url: &Url,
    request_url: &Url,
) -> Option<crate::network_host::controlled_resource::XhrServiceWorkerContext> {
    let runtime = state.service_worker_runtime.clone()?;
    let client_id = state.service_worker_client_id?;
    runtime.matching_controller_for_client_fetch(client_id, request_url)?;
    let policy = policy.clone();
    let connect_policy = policy.clone();
    let redirect_state = redirect_state.clone();
    let document_url = document_url.clone();
    let request_url = request_url.clone();
    let completion = state.fetch_completion_tx.clone();
    Some(
        crate::network_host::controlled_resource::XhrServiceWorkerContext::new(
            runtime,
            client_id,
            state.policy_context,
            connect_policy,
            move |head, source| {
                if source != moli_fetch::RedirectSource::ServiceWorker
                    && (head.final_url == request_url
                        || redirect_state.was_checked(&head.final_url))
                {
                    return Ok(());
                }
                let status = if head.redirect_chain.is_empty() {
                    crate::content_security_policy::ContentSecurityPolicyRedirectStatus::NoRedirect
                } else {
                    crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect
                };
                let (reports, enforced) = policy
                    .check_url(&document_url, &head.final_url, status)
                    .into_violations();
                let failure = enforced.first().map(|violation| {
                    crate::document_runtime::document_content_security_policy_error_message(
                        violation,
                        "XMLHttpRequest",
                    )
                });
                for mut violation in reports.into_iter().chain(enforced) {
                    violation.blocked_uri =
                        crate::content_security_policy::csp_url_for_report(&request_url);
                    crate::content_security_policy::ContentSecurityPolicySourceLocation::default()
                        .apply_to(&mut violation);
                    let _ = completion.send(WorkerFetchEvent::PolicyViolations(vec![violation]));
                }
                redirect_state.record_checked_url(&head.final_url, failure.is_some());
                failure.map_or(Ok(()), Err)
            },
        ),
    )
}
