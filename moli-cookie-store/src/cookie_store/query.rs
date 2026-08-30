use std::ops::Deref;

use cookie::Cookie as RawCookie;
use url::Url;

use crate::cookie::Cookie;
use crate::cookie_domain::is_match as domain_match;
use crate::cookie_path::is_match as path_match;
use crate::utils::{is_http_scheme, is_secure};

use super::policy::*;
use super::query_policy::*;
use super::*;

impl CookieStore {
    #[deprecated(
        since = "0.14.1",
        note = "Please use the `get_request_values` function instead"
    )]
    /// Return an `Iterator` of the cookies for `url` in the store, suitable for submitting in an
    /// HTTP request. As the items are intended for use in creating a `Cookie` header in a GET request,
    /// they may contain only the `name` and `value` of a received cookie, eliding other parameters
    /// such as `path` or `expires`. For iteration over `Cookie` instances containing all data, please
    /// refer to [`CookieStore::matches`].
    pub fn get_request_cookies(&self, url: &Url) -> impl Iterator<Item = &RawCookie<'static>> {
        self.matches(url).into_iter().map(|c| c.deref())
    }

    /// Return an `Iterator` of the cookie (`name`, `value`) pairs for `url` in the store, suitable
    /// for use in the `Cookie` header of an HTTP request. For iteration over `Cookie` instances,
    /// please refer to [`CookieStore::matches`].
    pub fn get_request_values(&self, url: &Url) -> impl Iterator<Item = (&str, &str)> {
        self.matches(url).into_iter().map(|c| c.name_value())
    }

    /// Return a browser-style query result with both included and excluded
    /// cookies.
    ///
    /// This makes source semantics explicit instead of forcing higher layers to
    /// manually post-filter `matches()` results. Returned cookies are owned so
    /// the store can safely update access metadata after snapshotting the final
    /// included set. Excluded cookies are only collected when
    /// `context.return_excluded_cookies` is `true`.
    pub fn get_cookie_access_query_result(
        &mut self,
        context: &QueryContext<'_>,
    ) -> CookieAccessQueryResult {
        let mut result = CookieAccessQueryResult::default();
        for cookie in self
            .cookies
            .iter()
            .filter(|&(d, _)| domain_match(d, context.url))
            .flat_map(|(_, dcs)| dcs.values().flat_map(|pcs| pcs.values().cloned()))
        {
            let access_result = query_context_access_result(&cookie, context);
            let entry = CookieWithAccessResult {
                cookie,
                access_result,
            };
            if entry.access_result.status.is_included() {
                result.included_cookies.push(entry);
            } else if context.return_excluded_cookies {
                result.excluded_cookies.push(entry);
            }
        }

        if context.update_access_time {
            // Query paths need mutable access for last-access updates, but iteration above only
            // holds shared borrows. Snapshot first, then touch by key, so the context API owns
            // access-time truth without exposing awkward callback-based borrowing to callers.
            for cookie in &result.included_cookies {
                let _ = self.touch_with_partition_key(
                    &String::from(&cookie.cookie.domain),
                    &String::from(&cookie.cookie.path),
                    cookie.cookie.name(),
                    cookie.cookie.partition_key(),
                );
            }
        }

        result
    }

    /// Return a browser-style query result with both included and excluded
    /// cookies.
    ///
    /// This is the compatibility projection of
    /// [`CookieStore::get_cookie_access_query_result`]. New browser-facing
    /// callers that need richer diagnostics should prefer the access-result
    /// form directly.
    pub fn get_cookie_query_result(&mut self, context: &QueryContext<'_>) -> CookieQueryResult {
        let result = self.get_cookie_access_query_result(context);
        CookieQueryResult {
            included_cookies: result
                .included_cookies
                .into_iter()
                .map(|entry| entry.cookie)
                .collect(),
            excluded_cookies: result
                .excluded_cookies
                .into_iter()
                .filter_map(|entry| {
                    entry
                        .access_result
                        .status
                        .first_exclusion_reason()
                        .map(|reason| ExcludedCookie {
                            cookie: entry.cookie,
                            reason,
                        })
                })
                .collect(),
        }
    }

    /// Return a browser-style query result whose included cookies are sorted
    /// for projection into APIs like the HTTP `Cookie` header or
    /// `document.cookie`.
    ///
    /// The ordering matches browser cookie serialization rules: longer paths
    /// sort before shorter paths, and ties break by creation time.
    pub fn get_ordered_cookie_query_result(
        &mut self,
        context: &QueryContext<'_>,
    ) -> CookieQueryResult {
        let mut result = self.get_cookie_query_result(context);
        sort_included_cookies_for_projection(&mut result.included_cookies);
        result
    }

    /// Return a browser-style query result whose included cookies are sorted
    /// for projection into APIs like the HTTP `Cookie` header or
    /// `document.cookie`, while preserving access-result metadata.
    pub fn get_ordered_cookie_access_query_result(
        &mut self,
        context: &QueryContext<'_>,
    ) -> CookieAccessQueryResult {
        let mut result = self.get_cookie_access_query_result(context);
        sort_included_cookie_accesses_for_projection(&mut result.included_cookies);
        result
    }

    /// Return matched cookies for a browser-style query context.
    pub fn get_cookies_with_context(&mut self, context: &QueryContext<'_>) -> Vec<Cookie<'static>> {
        self.get_cookie_query_result(context).included_cookies
    }

    /// Return matched cookies for a browser-style query context in browser
    /// serialization order.
    pub fn get_ordered_cookies_with_context(
        &mut self,
        context: &QueryContext<'_>,
    ) -> Vec<Cookie<'static>> {
        self.get_ordered_cookie_query_result(context)
            .included_cookies
    }

    /// Return request-header cookie pairs for a browser-style query context.
    ///
    /// The output preserves only `name` and `value`, mirroring the eventual
    /// HTTP `Cookie` header projection.
    pub fn get_request_values_with_context(
        &mut self,
        context: &QueryContext<'_>,
    ) -> Vec<(String, String)> {
        self.get_cookies_with_context(context)
            .into_iter()
            .map(|cookie| (cookie.name().to_owned(), cookie.value().to_owned()))
            .collect()
    }

    /// Return request-header cookie pairs for a browser-style query context in
    /// browser serialization order.
    pub fn get_ordered_request_values_with_context(
        &mut self,
        context: &QueryContext<'_>,
    ) -> Vec<(String, String)> {
        self.get_ordered_cookies_with_context(context)
            .into_iter()
            .map(|cookie| (cookie.name().to_owned(), cookie.value().to_owned()))
            .collect()
    }

    /// Mark an existing cookie as recently accessed.
    ///
    /// Browsers update access metadata when a cookie is read so later eviction can make
    /// decisions from one canonical timestamp source. Exposing this as a small targeted API
    /// lets higher layers keep their own request-context filtering while still delegating the
    /// access-order truth to the core store.
    pub fn touch(&mut self, domain: &str, path: &str, name: &str) -> bool {
        self.touch_with_partition_key(domain, path, name, None)
    }

    pub fn touch_with_partition_key(
        &mut self,
        domain: &str,
        path: &str,
        name: &str,
        partition_key: Option<&crate::CookiePartitionKey>,
    ) -> bool {
        let access_index = self.bump_access_index();
        self.get_mut_with_partition_key(domain, path, name, partition_key)
            .map(|cookie| {
                cookie.touch_with_access_index(access_index);
            })
            .is_some()
    }

    /// Specify a public suffix lookup for the `CookieStore` to allow [public suffix
    /// matching](https://datatracker.ietf.org/doc/html/rfc6265#section-5.3).
    #[cfg(feature = "public_suffix")]
    pub fn with_suffix_list<L>(self, psl: L) -> CookieStore
    where
        L: crate::CookiePublicSuffixList + 'static,
    {
        self.with_shared_suffix_list(std::sync::Arc::new(psl))
    }

    /// Specify an immutable shared public suffix lookup without copying its rule table.
    #[cfg(feature = "public_suffix")]
    pub fn with_shared_suffix_list<L>(self, psl: std::sync::Arc<L>) -> CookieStore
    where
        L: crate::CookiePublicSuffixList + 'static,
    {
        CookieStore {
            cookies: self.cookies,
            next_creation_index: self.next_creation_index,
            next_access_index: self.next_access_index,
            limits: self.limits,
            public_suffix_list: Some(psl),
        }
    }

    /// Return a copy of this store with explicit cookie-count limits.
    ///
    /// Quota enforcement is owned by the core store because eviction depends on
    /// canonical priority and access metadata that already live here.
    pub fn with_limits(mut self, limits: CookieStoreLimits) -> CookieStore {
        self.limits = limits;
        self
    }

    /// Update the cookie-count limits used by the store.
    pub fn set_limits(&mut self, limits: CookieStoreLimits) {
        self.limits = limits;
    }

    /// Returns true if the `CookieStore` contains an __unexpired__ `Cookie` corresponding to the
    /// specified `domain`, `path`, and `name`.
    pub fn contains(&self, domain: &str, path: &str, name: &str) -> bool {
        self.get(domain, path, name).is_some()
    }

    /// Returns true if the `CookieStore` contains any (even an __expired__) `Cookie` corresponding
    /// to the specified `domain`, `path`, and `name`.
    pub fn contains_any(&self, domain: &str, path: &str, name: &str) -> bool {
        self.get_any(domain, path, name).is_some()
    }

    /// Returns a reference to the __unexpired__ `Cookie` corresponding to the specified `domain`,
    /// `path`, and `name`.
    pub fn get(&self, domain: &str, path: &str, name: &str) -> Option<&Cookie<'_>> {
        self.get_with_partition_key(domain, path, name, None)
    }

    /// Return an unexpired cookie from one exact partition. `None` denotes
    /// the unpartitioned cookie namespace.
    pub fn get_with_partition_key(
        &self,
        domain: &str,
        path: &str,
        name: &str,
        partition_key: Option<&crate::CookiePartitionKey>,
    ) -> Option<&Cookie<'_>> {
        self.get_any_with_partition_key(domain, path, name, partition_key)
            .filter(|cookie| !cookie.is_expired())
    }

    pub(super) fn get_mut_with_partition_key(
        &mut self,
        domain: &str,
        path: &str,
        name: &str,
        partition_key: Option<&crate::CookiePartitionKey>,
    ) -> Option<&mut Cookie<'static>> {
        self.get_mut_any_with_partition_key(domain, path, name, partition_key)
            .filter(|cookie| !cookie.is_expired())
    }

    /// Returns a reference to the (possibly __expired__) `Cookie` corresponding to the specified
    /// `domain`, `path`, and `name`.
    pub fn get_any(&self, domain: &str, path: &str, name: &str) -> Option<&Cookie<'static>> {
        self.get_any_with_partition_key(domain, path, name, None)
    }

    pub fn get_any_with_partition_key(
        &self,
        domain: &str,
        path: &str,
        name: &str,
        partition_key: Option<&crate::CookiePartitionKey>,
    ) -> Option<&Cookie<'static>> {
        let key = CookieKey::new(name, partition_key.cloned());
        self.cookies.get(domain).and_then(|domain_cookies| {
            domain_cookies
                .get(path)
                .and_then(|path_cookies| path_cookies.get(&key))
        })
    }

    /// Returns a mutable reference to the (possibly __expired__) `Cookie` corresponding to the
    /// specified `domain`, `path`, and `name`.
    fn get_mut_any_with_partition_key(
        &mut self,
        domain: &str,
        path: &str,
        name: &str,
        partition_key: Option<&crate::CookiePartitionKey>,
    ) -> Option<&mut Cookie<'static>> {
        let key = CookieKey::new(name, partition_key.cloned());
        self.cookies.get_mut(domain).and_then(|domain_cookies| {
            domain_cookies
                .get_mut(path)
                .and_then(|path_cookies| path_cookies.get_mut(&key))
        })
    }

    /// Removes a `Cookie` from the store, returning the `Cookie` if it was in the store
    pub fn remove(&mut self, domain: &str, path: &str, name: &str) -> Option<Cookie<'static>> {
        self.remove_with_partition_key(domain, path, name, None)
    }

    /// Remove one exact cookie identity. `None` denotes the unpartitioned
    /// namespace.
    pub fn remove_with_partition_key(
        &mut self,
        domain: &str,
        path: &str,
        name: &str,
        partition_key: Option<&crate::CookiePartitionKey>,
    ) -> Option<Cookie<'static>> {
        #[cfg(not(feature = "preserve_order"))]
        fn map_remove<K, V, Q>(map: &mut Map<K, V>, key: &Q) -> Option<V>
        where
            K: std::borrow::Borrow<Q> + std::cmp::Eq + std::hash::Hash,
            Q: std::cmp::Eq + std::hash::Hash + ?Sized,
        {
            map.remove(key)
        }
        #[cfg(feature = "preserve_order")]
        fn map_remove<K, V, Q>(map: &mut Map<K, V>, key: &Q) -> Option<V>
        where
            K: std::borrow::Borrow<Q> + std::cmp::Eq + std::hash::Hash,
            Q: std::cmp::Eq + std::hash::Hash + ?Sized,
        {
            map.shift_remove(key)
        }

        let cookie_key = CookieKey::new(name, partition_key.cloned());
        let (removed, remove_domain) = match self.cookies.get_mut(domain) {
            None => (None, false),
            Some(domain_cookies) => {
                let (removed, remove_path) = match domain_cookies.get_mut(path) {
                    None => (None, false),
                    Some(path_cookies) => {
                        let removed = map_remove(path_cookies, &cookie_key);
                        (removed, path_cookies.is_empty())
                    }
                };

                if remove_path {
                    map_remove(domain_cookies, path);
                    (removed, domain_cookies.is_empty())
                } else {
                    (removed, false)
                }
            }
        };

        if remove_domain {
            map_remove(&mut self.cookies, domain);
        }

        removed
    }

    /// Delete all unexpired cookies matching `filter` and return the number of
    /// removed cookies.
    ///
    /// `url_host` is interpreted with canonical cookie semantics: host-only
    /// cookies only match the exact host, while domain cookies match subdomains
    /// as well.
    pub fn delete_matching(&mut self, filter: &CookieDeleteFilter<'_>) -> usize {
        let removed_cookies = self
            .iter_unexpired()
            .filter(|cookie| cookie_matches_delete_filter(cookie, filter))
            .map(|cookie| {
                (
                    cookie.name().to_owned(),
                    canonical_cookie_domain(cookie),
                    String::from(&cookie.path),
                    cookie.partition_key().cloned(),
                )
            })
            .collect::<Vec<_>>();

        for (name, domain, path, partition_key) in &removed_cookies {
            self.remove_with_partition_key(domain, path, name, partition_key.as_ref());
        }

        removed_cookies.len()
    }

    /// Returns a collection of references to __unexpired__ cookies that path- and domain-match
    /// `request_url`, as well as having HttpOnly and Secure attributes compatible with the
    /// `request_url`.
    pub fn matches(&self, request_url: &Url) -> Vec<&Cookie<'static>> {
        // although we domain_match and path_match as we descend through the tree, we
        // still need to
        // do a full Cookie::matches() check in the last filter. Otherwise, we cannot
        // properly deal
        // with HostOnly Cookies.
        let cookies = self
            .cookies
            .iter()
            .filter(|&(d, _)| domain_match(d, request_url))
            .flat_map(|(_, dcs)| {
                dcs.iter()
                    .filter(|&(p, _)| path_match(p, request_url))
                    .flat_map(|(_, pcs)| {
                        pcs.values()
                            .filter(|c| !c.is_expired() && c.matches(request_url))
                    })
            });
        match (!is_http_scheme(request_url), !is_secure(request_url)) {
            (true, true) => cookies
                .filter(|c| !c.http_only().unwrap_or(false) && !c.secure().unwrap_or(false))
                .collect(),
            (true, false) => cookies
                .filter(|c| !c.http_only().unwrap_or(false))
                .collect(),
            (false, true) => cookies.filter(|c| !c.secure().unwrap_or(false)).collect(),
            (false, false) => cookies.collect(),
        }
    }
}
