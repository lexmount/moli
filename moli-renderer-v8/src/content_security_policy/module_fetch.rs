use std::{fmt, sync::Arc};

use parking_lot::Mutex;
use url::Url;

use super::{ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyUrlViolation};

type Check = dyn Fn(
        &Url,
        &Url,
        ContentSecurityPolicyRedirectStatus,
    ) -> (
        Vec<ContentSecurityPolicyUrlViolation>,
        Vec<ContentSecurityPolicyUrlViolation>,
    ) + Send
    + Sync;

/// One fetch's captured CSP authority and reports awaiting owner dispatch.
/// The Fetch thread only evaluates policy; the owning realm dispatches events.
#[derive(Clone)]
pub(crate) struct ModuleFetchCspContext(Arc<Inner>);

struct Inner {
    check: Box<Check>,
    original_url: Url,
    owner_scope: Option<crate::native_bridge::OwnerDispatchScope>,
    state: Mutex<State>,
    violation_reporter: Option<Arc<dyn Fn(ContentSecurityPolicyUrlViolation) + Send + Sync>>,
}

#[derive(Default)]
struct State {
    checked_hops: Vec<(usize, Url)>,
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
    pub(crate) fn new<R, E>(
        original_url: Url,
        owner_scope: Option<crate::native_bridge::OwnerDispatchScope>,
        check: impl Fn(&Url, &Url, ContentSecurityPolicyRedirectStatus) -> (R, E)
        + Send
        + Sync
        + 'static,
    ) -> Self
    where
        R: IntoIterator<Item = ContentSecurityPolicyUrlViolation>,
        E: IntoIterator<Item = ContentSecurityPolicyUrlViolation>,
    {
        Self(Arc::new(Inner {
            check: Box::new(move |url, original, status| {
                let (reports, enforced) = check(url, original, status);
                (
                    reports.into_iter().collect(),
                    enforced.into_iter().collect(),
                )
            }),
            original_url,
            owner_scope,
            state: Mutex::new(State::default()),
            violation_reporter: None,
        }))
    }

    /// Install the networking report route before sharing this fetch context.
    pub(crate) fn with_violation_reporter(
        mut self,
        reporter: impl Fn(ContentSecurityPolicyUrlViolation) + Send + Sync + 'static,
    ) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("module reporting route is bound before fetch context sharing")
            .violation_reporter = Some(Arc::new(reporter));
        self
    }

    pub(crate) fn owner_scope(&self) -> Option<crate::native_bridge::OwnerDispatchScope> {
        self.0.owner_scope
    }

    fn original_url(&self) -> &Url {
        &self.0.original_url
    }

    pub(crate) fn check_initial_request(&self, url: &Url) -> anyhow::Result<()> {
        self.check_hop(url, 0).map_err(|error| {
            anyhow::anyhow!("Refused to load module `{}`: {error}", self.original_url())
        })
    }

    pub(crate) fn apply(&self, request: moli_fetch::Request) -> moli_fetch::Request {
        let context = self.clone();
        request.with_redirect_policy(moli_fetch::RequestRedirectPolicy::new(
            move |url, ordinal| context.check_hop(url, ordinal),
        ))
    }

    fn check_hop(&self, url: &Url, ordinal: usize) -> anyhow::Result<()> {
        let (report, enforce) = (self.0.check)(
            url,
            self.original_url(),
            if ordinal == 0 {
                ContentSecurityPolicyRedirectStatus::NoRedirect
            } else {
                ContentSecurityPolicyRedirectStatus::FollowedRedirect
            },
        );
        let message = enforce.first().map(|violation| {
            format!(
                "Module request blocked by Content Security Policy directive `{}`",
                violation.effective_directive,
            )
        });
        let mut state = self.0.state.lock();
        if !state
            .checked_hops
            .iter()
            .any(|(n, checked)| *n == ordinal && checked == url)
        {
            state.checked_hops.push((ordinal, url.clone()));
            if let Some(reporter) = &self.0.violation_reporter {
                drop(state);
                for violation in report.into_iter().chain(enforce) {
                    reporter(violation);
                }
            } else {
                state.violations.extend(report);
                state.violations.extend(enforce);
            }
        }
        match message {
            Some(message) => anyhow::bail!(message),
            None => Ok(()),
        }
    }

    // Coalesced/cache consumers still validate every recorded redirect against
    // their own policy. The transport owner's checks are not another owner's authority.

    pub(crate) fn check_response_head(
        &self,
        response: &moli_fetch::ResponseHead,
    ) -> anyhow::Result<()> {
        for (index, redirect) in response.redirect_chain.iter().enumerate() {
            self.check_hop(&redirect.to_url, index + 1)?;
        }
        if response.redirected {
            self.check_hop(&response.final_url, response.redirect_chain.len().max(1))?;
        }
        Ok(())
    }

    pub(crate) fn take_violations(&self) -> Vec<ContentSecurityPolicyUrlViolation> {
        std::mem::take(&mut self.0.state.lock().violations)
    }
}
