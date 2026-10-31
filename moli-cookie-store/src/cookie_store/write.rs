use cookie::SameSite;
use url::Url;

use crate::cookie::{CanonicalCookieInput, Cookie};
use crate::utils::{is_secure, is_trustworthy_non_cryptographic};
use crate::CookieError;

use super::policy::*;
use super::write_policy::*;
use super::*;

impl CookieStore {
    /// Parses a new `Cookie` from `cookie_str` and inserts it into the store.
    pub fn parse(&mut self, cookie_str: &str, request_url: &Url) -> InsertResult {
        Cookie::parse(cookie_str, request_url)
            .and_then(|cookie| self.insert(cookie.into_owned(), request_url))
    }

    /// Parses a `Set-Cookie` header value and inserts it into the store.
    pub fn insert_response_cookie_str(
        &mut self,
        cookie_str: &str,
        request_url: &Url,
    ) -> InsertResult {
        self.set_response_cookie_str_with_context(cookie_str, &inferred_insert_context(request_url))
            .into_insert_result()
    }

    /// Parse a `Set-Cookie` header value and apply it using an explicit
    /// browser/API insertion context.
    pub fn set_response_cookie_str_with_context(
        &mut self,
        cookie_str: &str,
        context: &InsertContext<'_>,
    ) -> CookieSetResult {
        self.set_response_cookie_str_with_access_result(cookie_str, context)
            .into_set_result()
    }

    /// Parse a `Set-Cookie` header value and apply it using an explicit
    /// browser/API insertion context, returning a rich browser-style result.
    pub fn set_response_cookie_str_with_access_result(
        &mut self,
        cookie_str: &str,
        context: &InsertContext<'_>,
    ) -> CookieSetAccessResult {
        let sanitized = sanitize_cookie_line_for_browser_parse(cookie_str);
        match crate::cookie_date::parse_cookie(sanitized.line.as_ref())
            .map_err(CookieError::from)
            .and_then(|cookie| {
                Cookie::try_from_raw_cookie(&cookie, context.url).map(Cookie::into_owned)
            }) {
            Ok(cookie) => self.set_with_access_result_chain(
                cookie,
                context,
                CookieSetAccessResult {
                    status: CookieSetResult::Accepted(StoreAction::Inserted),
                    rejection_reasons: Vec::new(),
                    warning_reasons: sanitized.warning_reasons,
                    effective_same_site: None,
                },
            ),
            Err(error) => {
                let reason = CookieSetRejectionReason::from(error);
                CookieSetAccessResult {
                    status: CookieSetResult::Rejected(reason),
                    rejection_reasons: vec![reason],
                    warning_reasons: sanitized.warning_reasons,
                    effective_same_site: None,
                }
            }
        }
    }

    /// Parse a `Set-Cookie` header value and insert it into the store using an
    /// explicit browser/API context.
    pub fn insert_response_cookie_str_with_context(
        &mut self,
        cookie_str: &str,
        context: &InsertContext<'_>,
    ) -> InsertResult {
        self.set_response_cookie_str_with_access_result(cookie_str, context)
            .into_insert_result()
    }

    /// Construct and store a canonical cookie from structured fields using an
    /// explicit browser/API context.
    pub fn set_canonical_cookie_with_context(
        &mut self,
        input: CanonicalCookieInput,
        context: &InsertContext<'_>,
    ) -> CookieSetResult {
        self.set_canonical_cookie_with_access_result(input, context)
            .into_set_result()
    }

    /// Construct and store a canonical cookie from structured fields using an
    /// explicit browser/API context, returning a rich browser-style result.
    pub fn set_canonical_cookie_with_access_result(
        &mut self,
        input: CanonicalCookieInput,
        context: &InsertContext<'_>,
    ) -> CookieSetAccessResult {
        match Cookie::try_from_canonical_input(input, context.url) {
            Ok(cookie) => self.set_with_access_result(cookie, context),
            Err(error) => rejected_set_access_result(error.into()),
        }
    }

    /// Construct and store a canonical cookie from structured fields while
    /// continuing an existing rich set result.
    pub fn set_canonical_cookie_with_access_result_chain(
        &mut self,
        input: CanonicalCookieInput,
        context: &InsertContext<'_>,
        prior_result: CookieSetAccessResult,
    ) -> CookieSetAccessResult {
        match Cookie::try_from_canonical_input(input, context.url) {
            Ok(cookie) => self.set_with_access_result_chain(cookie, context, prior_result),
            Err(error) => merge_rejected_set_access_result(prior_result, error.into()),
        }
    }

    /// Inserts `cookie`, received from `request_url`, into the store.
    pub fn insert(&mut self, cookie: Cookie<'static>, request_url: &Url) -> InsertResult {
        self.set_with_context(cookie, &inferred_insert_context(request_url))
            .into_insert_result()
    }

    /// Insert a canonical cookie using an explicit browser/API context.
    pub fn set_with_context(
        &mut self,
        cookie: Cookie<'static>,
        context: &InsertContext<'_>,
    ) -> CookieSetResult {
        self.set_with_access_result(cookie, context)
            .into_set_result()
    }

    /// Insert a canonical cookie using an explicit browser/API context and
    /// return a rich browser-style result.
    pub fn set_with_access_result(
        &mut self,
        cookie: Cookie<'static>,
        context: &InsertContext<'_>,
    ) -> CookieSetAccessResult {
        self.set_with_access_result_chain(cookie, context, provisional_set_access_result())
    }

    /// Insert a canonical cookie using an explicit browser/API context while
    /// continuing an existing rich set result.
    pub fn set_with_access_result_chain(
        &mut self,
        cookie: Cookie<'static>,
        context: &InsertContext<'_>,
        prior_result: CookieSetAccessResult,
    ) -> CookieSetAccessResult {
        self.set_with_ordering(cookie, context, prior_result, false)
    }

    /// Restore through canonical policy while retaining persisted ordering.
    pub fn restore_with_context(
        &mut self,
        cookie: Cookie<'static>,
        context: &InsertContext<'_>,
    ) -> CookieSetAccessResult {
        self.set_with_ordering(cookie, context, provisional_set_access_result(), true)
    }

    fn set_with_ordering(
        &mut self,
        mut cookie: Cookie<'static>,
        context: &InsertContext<'_>,
        mut prior_result: CookieSetAccessResult,
        preserve_ordering: bool,
    ) -> CookieSetAccessResult {
        if matches!(prior_result.status, CookieSetResult::Rejected(_)) {
            return prior_result;
        }

        if cookie.partitioned().unwrap_or(false) && cookie.partition_key().is_none() {
            cookie.set_partition_key(context.browser_context.cookie_partition_key.clone());
        } else if !cookie.partitioned().unwrap_or(false) {
            cookie.set_partition_key(None);
        }

        if prior_result.effective_same_site.is_none() {
            prior_result.effective_same_site = Some(effective_same_site(&cookie));
        }
        if cookie.secure().unwrap_or(false) && is_trustworthy_non_cryptographic(context.url) {
            prior_result.add_warning(CookieSetWarningReason::SecureAccessGrantedNonCryptographic);
        }

        for reason in collect_preinsert_rejection_reasons(self, &cookie, context) {
            prior_result.add_rejection(reason);
        }
        if matches!(prior_result.status, CookieSetResult::Rejected(_)) {
            return prior_result;
        }

        match self.insert_with_context_impl(cookie, context, preserve_ordering) {
            Ok(action) => {
                prior_result.status = CookieSetResult::Accepted(action);
                prior_result
            }
            Err(error) => {
                prior_result.add_rejection(error.into());
                prior_result
            }
        }
    }

    /// Insert a canonical cookie using an explicit browser/API context.
    pub fn insert_with_context(
        &mut self,
        cookie: Cookie<'static>,
        context: &InsertContext<'_>,
    ) -> InsertResult {
        self.set_with_access_result(cookie, context)
            .into_insert_result()
    }

    fn insert_with_context_impl(
        &mut self,
        cookie: Cookie<'static>,
        context: &InsertContext<'_>,
        preserve_ordering: bool,
    ) -> InsertResult {
        let request_url = context.url;
        let cookie = canonicalize_cookie_for_store_checks(self, cookie, request_url)?;

        if cookie.http_only().unwrap_or(false) && context.source == CookieAccessSource::Document {
            return Err(CookieError::NonHttpScheme);
        }
        if context.enforce_browser_policy
            && cookie.secure().unwrap_or(false)
            && !is_secure(request_url)
        {
            return Err(CookieError::SecureOnly);
        }
        if context.enforce_browser_policy
            && cookie.same_site() == Some(SameSite::None)
            && !cookie.secure().unwrap_or(false)
        {
            return Err(CookieError::SameSiteNoneRequiresSecure);
        }
        if context.enforce_browser_policy
            && cookie_name_value_too_large(cookie.name(), cookie.value())
        {
            return Err(CookieError::NameValueTooLarge);
        }
        if cookie.partitioned().unwrap_or(false) {
            if context.enforce_browser_policy && !cookie.secure().unwrap_or(false) {
                return Err(CookieError::PartitionedRequiresSecure);
            }
            if cookie.partition_key().is_none() {
                return Err(CookieError::PartitionedMissingPartitionKey);
            }
        }
        if context.enforce_browser_policy && !prefixes_are_valid(&cookie) {
            return Err(CookieError::PrefixViolation);
        }
        if !cookie.domain.matches(request_url) {
            return Err(CookieError::DomainMismatch);
        }
        if context.enforce_browser_policy
            && context.source != CookieAccessSource::Cdp
            && !cookie.secure().unwrap_or(false)
            && !is_secure(request_url)
            && self.has_secure_overlay_conflict(&cookie)
        {
            return Err(CookieError::SecureOverlay);
        }

        {
            let cookie_domain = cookie
                .domain
                .as_cow()
                .ok_or(CookieError::UnspecifiedDomain)?;
            let observed = self.observer.is_some();
            if let Some(old_cookie) = self.get_mut_with_partition_key(
                &cookie_domain,
                &cookie.path,
                cookie.name(),
                cookie.partition_key(),
            ) {
                if old_cookie.http_only().unwrap_or(false)
                    && context.source == CookieAccessSource::Document
                {
                    return Err(CookieError::NonHttpScheme);
                } else if cookie.is_expired() {
                    let key = observed.then(|| CookieStoreKey::for_cookie(old_cookie));
                    old_cookie.expire();
                    if let Some(key) = key {
                        self.notify(CookieStoreMutation::Delete(key));
                    }
                    return Ok(StoreAction::ExpiredExisting);
                }
            }
        }

        if !cookie.is_expired() {
            let mut cookie = cookie;
            let cookie_domain = String::from(&cookie.domain);
            let cookie_path = String::from(&cookie.path);
            let cookie_name = cookie.name().to_owned();
            let cookie_key = CookieKey::for_cookie(&cookie);
            let existing_creation_index = self
                .get_with_partition_key(
                    &cookie_domain,
                    &cookie_path,
                    &cookie_name,
                    cookie.partition_key(),
                )
                .map(|existing| existing.creation_index());
            let replacing_existing = existing_creation_index.is_some();
            if !self.make_room_for_cookie(
                &cookie_domain,
                cookie.partition_key(),
                cookie.secure().unwrap_or(false),
                replacing_existing,
            ) {
                return Err(CookieError::StorageFull);
            }
            if preserve_ordering {
                self.next_creation_index = self
                    .next_creation_index
                    .max(cookie.creation_index().saturating_add(1));
                self.next_access_index = self
                    .next_access_index
                    .max(cookie.last_access_index().saturating_add(1));
            } else {
                let creation_index = if let Some(creation_index) = existing_creation_index {
                    creation_index
                } else {
                    // Expired tombstones should not count as an in-place replacement. Remove any
                    // stale entry for this key before inserting so quota checks and store actions
                    // treat the new cookie as a fresh live entry.
                    let _ = self.remove_with_partition_key(
                        &cookie_domain,
                        &cookie_path,
                        &cookie_name,
                        cookie.partition_key(),
                    );
                    self.bump_creation_index()
                };
                cookie.set_creation_index(creation_index);
                cookie.touch_with_access_index(self.bump_access_index());
            }
            let observed = self.observer.is_some().then(|| cookie.clone());
            let replaced = self
                .cookies
                .entry(cookie_domain)
                .or_default()
                .entry(cookie_path)
                .or_default()
                .insert(cookie_key, cookie)
                .is_some();
            if let Some(cookie) = observed {
                self.notify(CookieStoreMutation::Upsert { cookie });
            }
            Ok(if replaced {
                StoreAction::UpdatedExisting
            } else {
                StoreAction::Inserted
            })
        } else {
            Err(CookieError::Expired)
        }
    }
}
