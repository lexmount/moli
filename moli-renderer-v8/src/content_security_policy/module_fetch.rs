use std::{fmt, sync::Arc};

use parking_lot::Mutex;
use url::Url;

use super::{ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyUrlViolation};

type Check = dyn Fn(
        &Url,
        &Url,
        ContentSecurityPolicyRedirectStatus,
    ) -> (
        Option<ContentSecurityPolicyUrlViolation>,
        Option<ContentSecurityPolicyUrlViolation>,
    ) + Send
    + Sync;

/// One fetch's captured CSP authority and reports awaiting its owner terminal.
/// The Fetch thread only evaluates policy; the owning realm dispatches events.
#[derive(Clone)]
pub(crate) struct ModuleFetchCspContext(Arc<Inner>);

struct Inner {
    check: Box<Check>,
    original_url: Url,
    owner_scope: Option<crate::native_bridge::OwnerDispatchScope>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    checked_redirects: Vec<(usize, Url)>,
    violations: Vec<ContentSecurityPolicyUrlViolation>,
}

impl fmt::Debug for ModuleFetchCspContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleFetchCspContext")
            .finish_non_exhaustive()
    }
}

// A context identifies one admitted fetch, including its reporting channel.
impl PartialEq for ModuleFetchCspContext {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ModuleFetchCspContext {}

impl ModuleFetchCspContext {
    pub(crate) fn new(
        original_url: Url,
        owner_scope: Option<crate::native_bridge::OwnerDispatchScope>,
        check: impl Fn(
            &Url,
            &Url,
            ContentSecurityPolicyRedirectStatus,
        ) -> (
            Option<ContentSecurityPolicyUrlViolation>,
            Option<ContentSecurityPolicyUrlViolation>,
        ) + Send
        + Sync
        + 'static,
    ) -> Self {
        Self(Arc::new(Inner {
            check: Box::new(check),
            original_url,
            owner_scope,
            state: Mutex::new(State::default()),
        }))
    }

    pub(crate) fn owner_scope(&self) -> Option<crate::native_bridge::OwnerDispatchScope> {
        self.0.owner_scope
    }

    fn original_url(&self) -> &Url {
        &self.0.original_url
    }

    pub(crate) fn check_initial_request(&self, url: &Url) -> anyhow::Result<()> {
        // Main-Document and Worker admission already perform their initial
        // checks. Child module requests use the captured child authority here.
        if self.owner_scope().is_some() {
            self.check_redirect(url, 0)?;
        }
        Ok(())
    }

    pub(crate) fn apply(&self, request: moli_fetch::Request) -> moli_fetch::Request {
        let context = self.clone();
        request.with_redirect_policy(moli_fetch::RequestRedirectPolicy::new(
            move |url, ordinal| context.check_redirect(url, ordinal),
        ))
    }

    fn check_redirect(&self, url: &Url, ordinal: usize) -> anyhow::Result<()> {
        let (report, enforce) = (self.0.check)(
            url,
            self.original_url(),
            if ordinal == 0 {
                ContentSecurityPolicyRedirectStatus::NoRedirect
            } else {
                ContentSecurityPolicyRedirectStatus::FollowedRedirect
            },
        );
        let mut state = self.0.state.lock();
        if state.checked_redirects.contains(&(ordinal, url.clone())) {
            if let Some(enforce) = enforce {
                anyhow::bail!(
                    "Module request blocked by Content Security Policy directive `{}`",
                    enforce.effective_directive
                );
            }
            return Ok(());
        }
        state.checked_redirects.push((ordinal, url.clone()));
        if let Some(report) = report {
            state.violations.push(report);
        }
        if let Some(enforce) = enforce {
            let message = format!(
                "Module request blocked by Content Security Policy directive `{}`",
                enforce.effective_directive
            );
            state.violations.push(enforce);
            anyhow::bail!(message);
        }
        Ok(())
    }

    // Coalesced/cache consumers still validate every recorded redirect against
    // their own policy. The transport owner's checks are not another owner's authority.
    pub(crate) fn check_response(&self, response: &moli_fetch::Response) -> anyhow::Result<()> {
        let checked = self.0.state.lock().checked_redirects.clone();
        for (index, redirect) in response.redirect_chain.iter().enumerate() {
            if !checked.contains(&(index + 1, redirect.to_url.clone())) {
                self.check_redirect(&redirect.to_url, index + 1)?;
            }
        }
        if response.redirected {
            let (report, enforce) = (self.0.check)(
                &response.final_url,
                self.original_url(),
                ContentSecurityPolicyRedirectStatus::FollowedRedirect,
            );
            if let Some(report) = report {
                self.record_final_violation(report);
            }
            if let Some(enforce) = enforce {
                let message = format!(
                    "Module response blocked by Content Security Policy directive `{}`",
                    enforce.effective_directive
                );
                self.record_final_violation(enforce);
                anyhow::bail!(message);
            }
        }
        Ok(())
    }

    fn record_final_violation(&self, violation: ContentSecurityPolicyUrlViolation) {
        let mut state = self.0.state.lock();
        if !state.violations.contains(&violation) {
            state.violations.push(violation);
        }
    }

    pub(crate) fn take_violations(&self) -> Vec<ContentSecurityPolicyUrlViolation> {
        std::mem::take(&mut self.0.state.lock().violations)
    }
}
