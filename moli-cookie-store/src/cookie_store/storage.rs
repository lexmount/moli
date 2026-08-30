use std::io::{BufRead, Write};

use crate::cookie::{Cookie, CookiePriority};

use super::policy::{canonical_cookie_domain, domains_overlap, path_overlap};
use super::*;

impl CookieStore {
    /// Clear the contents of the store
    pub fn clear(&mut self) {
        self.cookies.clear()
    }

    /// An iterator visiting all the __unexpired__ cookies in the store
    pub fn iter_unexpired<'a>(&'a self) -> impl Iterator<Item = &'a Cookie<'static>> + 'a {
        self.cookies
            .values()
            .flat_map(|dcs| dcs.values())
            .flat_map(|pcs| pcs.values())
            .filter(|c| !c.is_expired())
    }

    /// An iterator visiting all (including __expired__) cookies in the store
    pub fn iter_any<'a>(&'a self) -> impl Iterator<Item = &'a Cookie<'static>> + 'a {
        self.cookies
            .values()
            .flat_map(|dcs| dcs.values())
            .flat_map(|pcs| pcs.values())
    }

    /// Serialize any __unexpired__ and __persistent__ cookies in the store with `cookie_to_string`
    /// and write them to `writer`
    pub fn save<W, E, F>(&self, writer: &mut W, cookie_to_string: F) -> StoreResult<()>
    where
        W: Write,
        F: Fn(&Cookie<'static>) -> Result<String, E>,
        crate::Error: From<E>,
    {
        for cookie in self.iter_unexpired().filter_map(|c| {
            if c.is_persistent() {
                Some(cookie_to_string(c))
            } else {
                None
            }
        }) {
            writeln!(writer, "{}", cookie?)?;
        }
        Ok(())
    }

    /// Serialize all (including __expired__ and __non-persistent__) cookies in the store with `cookie_to_string` and write them to `writer`
    pub fn save_incl_expired_and_nonpersistent<W, E, F>(
        &self,
        writer: &mut W,
        cookie_to_string: F,
    ) -> StoreResult<()>
    where
        W: Write,
        F: Fn(&Cookie<'static>) -> Result<String, E>,
        crate::Error: From<E>,
    {
        for cookie in self.iter_any() {
            writeln!(writer, "{}", cookie_to_string(cookie)?)?;
        }
        Ok(())
    }

    /// Load cookies from `reader`, deserializing with `cookie_from_str`, skipping any __expired__
    /// cookies
    pub fn load<R, E, F>(reader: R, cookie_from_str: F) -> StoreResult<CookieStore>
    where
        R: BufRead,
        F: Fn(&str) -> Result<Cookie<'static>, E>,
        crate::Error: From<E>,
    {
        CookieStore::load_from(reader, cookie_from_str, false)
    }

    /// Load cookies from `reader`, deserializing with `cookie_from_str`, loading both __unexpired__
    /// and __expired__ cookies
    pub fn load_all<R, E, F>(reader: R, cookie_from_str: F) -> StoreResult<CookieStore>
    where
        R: BufRead,
        F: Fn(&str) -> Result<Cookie<'static>, E>,
        crate::Error: From<E>,
    {
        CookieStore::load_from(reader, cookie_from_str, true)
    }

    fn load_from<R, E, F>(
        reader: R,
        cookie_from_str: F,
        include_expired: bool,
    ) -> StoreResult<CookieStore>
    where
        R: BufRead,
        F: Fn(&str) -> Result<Cookie<'static>, E>,
        crate::Error: From<E>,
    {
        let cookies = reader.lines().map(|line_result| {
            line_result
                .map_err(Into::into)
                .and_then(|line| cookie_from_str(&line).map_err(crate::Error::from))
        });
        Self::from_cookies(cookies, include_expired)
    }

    /// Create a `CookieStore` from an iterator of `Cookie` values.
    pub fn from_cookies<I, E>(iter: I, include_expired: bool) -> Result<Self, E>
    where
        I: IntoIterator<Item = Result<Cookie<'static>, E>>,
    {
        let mut cookies = Map::new();
        let mut next_creation_index = 0;
        let mut next_access_index = 0;
        for cookie in iter {
            let cookie = cookie?;
            if include_expired || !cookie.is_expired() {
                next_creation_index =
                    next_creation_index.max(cookie.creation_index().saturating_add(1));
                next_access_index =
                    next_access_index.max(cookie.last_access_index().saturating_add(1));
                cookies
                    .entry(String::from(&cookie.domain))
                    .or_insert_with(Map::new)
                    .entry(String::from(&cookie.path))
                    .or_insert_with(Map::new)
                    .insert(CookieKey::for_cookie(&cookie), cookie);
            }
        }
        Ok(Self {
            cookies,
            next_creation_index,
            next_access_index,
            limits: CookieStoreLimits::default(),
            #[cfg(feature = "public_suffix")]
            public_suffix_list: None,
        })
    }

    pub fn new() -> Self {
        Self {
            cookies: DomainMap::new(),
            next_creation_index: 0,
            next_access_index: 0,
            limits: CookieStoreLimits::default(),
            #[cfg(feature = "public_suffix")]
            public_suffix_list: None,
        }
    }

    pub(super) fn bump_creation_index(&mut self) -> u64 {
        let index = self.next_creation_index;
        self.next_creation_index = self.next_creation_index.saturating_add(1);
        index
    }

    pub(super) fn bump_access_index(&mut self) -> u64 {
        let index = self.next_access_index;
        self.next_access_index = self.next_access_index.saturating_add(1);
        index
    }

    pub(super) fn has_secure_overlay_conflict(&self, cookie: &Cookie<'_>) -> bool {
        let incoming_domain = String::from(&cookie.domain);
        let incoming_path = String::from(&cookie.path);

        // The overlay check is intentionally expressed over canonicalized
        // domain/path fields, not the raw Set-Cookie string, so host-only and
        // default-path resolution happen exactly once in the core.
        self.iter_unexpired().any(|existing| {
            existing.name() == cookie.name()
                && existing.partition_key() == cookie.partition_key()
                && existing.secure().unwrap_or(false)
                && domains_overlap(&incoming_domain, &String::from(&existing.domain))
                && path_overlap(&incoming_path, existing.path.as_ref())
        })
    }

    pub(super) fn make_room_for_cookie(
        &mut self,
        domain: &str,
        incoming_secure_cookie: bool,
        replacing_existing: bool,
    ) -> bool {
        while self
            .iter_unexpired()
            .count()
            .saturating_sub(usize::from(replacing_existing))
            >= self.limits.total_cookies
        {
            if !self.evict_one_cookie(None, incoming_secure_cookie) {
                return false;
            }
        }

        while self
            .iter_unexpired()
            .filter(|existing| canonical_cookie_domain(existing) == domain)
            .count()
            .saturating_sub(usize::from(replacing_existing))
            >= self.limits.per_domain_cookies
        {
            if !self.evict_one_cookie(Some(domain), incoming_secure_cookie) {
                return false;
            }
        }

        true
    }

    fn evict_one_cookie(&mut self, domain: Option<&str>, incoming_secure_cookie: bool) -> bool {
        for priority in [
            CookiePriority::Low,
            CookiePriority::Medium,
            CookiePriority::High,
        ] {
            if let Some((name, domain, path, partition_key)) =
                self.oldest_accessed_cookie_key(domain, false, priority)
            {
                self.remove_with_partition_key(&domain, &path, &name, partition_key.as_ref());
                return true;
            }
        }

        if !incoming_secure_cookie {
            return false;
        }

        for priority in [
            CookiePriority::Low,
            CookiePriority::Medium,
            CookiePriority::High,
        ] {
            if let Some((name, domain, path, partition_key)) =
                self.oldest_accessed_cookie_key(domain, true, priority)
            {
                self.remove_with_partition_key(&domain, &path, &name, partition_key.as_ref());
                return true;
            }
        }

        false
    }

    fn oldest_accessed_cookie_key(
        &self,
        domain: Option<&str>,
        secure: bool,
        priority: CookiePriority,
    ) -> Option<(String, String, String, Option<crate::CookiePartitionKey>)> {
        self.iter_unexpired()
            .filter(|cookie| {
                cookie.secure().unwrap_or(false) == secure
                    && cookie.effective_priority() == priority
                    && domain.is_none_or(|domain| canonical_cookie_domain(cookie) == domain)
            })
            .min_by(|left, right| {
                left.last_access_index()
                    .cmp(&right.last_access_index())
                    .then_with(|| left.creation_index().cmp(&right.creation_index()))
            })
            .map(|cookie| {
                (
                    cookie.name().to_owned(),
                    canonical_cookie_domain(cookie),
                    String::from(&cookie.path),
                    cookie.partition_key().cloned(),
                )
            })
    }
}
