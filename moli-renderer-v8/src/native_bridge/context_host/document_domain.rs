use super::*;
use crate::dom::native::Node;
use std::net::IpAddr;
use url::{Host, Url};

impl JsContextHost {
    pub(crate) fn document_domain_value_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> String {
        if document_handle == self.document_handle() {
            return self
                .document_domain_override
                .clone()
                .unwrap_or_else(|| self.main_document_domain_host().unwrap_or_default());
        }
        if let Some(child_handle) =
            self.child_browsing_context_handle_for_stored_document(document_handle)
        {
            return self.child_browsing_context_document_domain_value(child_handle);
        }
        if let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document_handle) {
            return self.lightweight_popup_document_domain_value(popup_id);
        }
        self.dom_host()
            .dom()
            .node(document_handle)
            .and_then(Node::as_document)
            .and_then(|document| url_host_domain(document.url()))
            .unwrap_or_default()
    }

    pub(crate) fn set_document_domain_for_document_handle(
        &mut self,
        document_handle: DomHandle,
        value: &str,
    ) -> bool {
        if document_handle == self.document_handle() {
            if self.document_sandbox_policy().sandboxes_document_domain {
                return false;
            }
            let Some(current_host) = self.main_document_domain_host() else {
                return false;
            };
            let Some(domain) = normalize_document_domain_value(value) else {
                return false;
            };
            if !document_domain_is_allowed_for_host(&current_host, &domain) {
                return false;
            }
            self.document_domain_override = Some(domain);
            return true;
        }
        let Some(child_handle) =
            self.child_browsing_context_handle_for_stored_document(document_handle)
        else {
            let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document_handle)
            else {
                return false;
            };
            return self.set_lightweight_popup_document_domain(popup_id, document_handle, value);
        };
        self.set_child_browsing_context_document_domain(child_handle, value)
    }

    pub(crate) fn child_browsing_context_document_domain_override(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        match self.child_browsing_context_security_origin_owner(handle)? {
            ChildSecurityOriginOwner::Main => self.document_domain_override.clone(),
            ChildSecurityOriginOwner::Child(owner) => self
                .child_browsing_contexts
                .get(&owner)
                .and_then(|entry| entry.document_domain_override()),
        }
    }

    pub(crate) fn child_browsing_context_document_domain_value(&self, handle: DomHandle) -> String {
        if let Some(domain) = self.child_browsing_context_document_domain_override(handle) {
            return domain;
        }
        self.child_browsing_context_document_domain_host(handle)
            .unwrap_or_default()
    }

    pub(crate) fn lightweight_popup_document_domain_value(&self, popup_id: u64) -> String {
        if let Some(domain) = self.lightweight_popup_document_domain_override(popup_id) {
            return domain;
        }
        self.lightweight_popup_origin(popup_id)
            .and_then(|origin| Url::parse(&origin).ok())
            .and_then(|origin| url_host_domain(&origin))
            .unwrap_or_default()
    }

    pub(crate) fn set_child_browsing_context_document_domain(
        &mut self,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        if self.child_browsing_context_document_domain_is_sandboxed(handle) {
            return false;
        }
        let Some(current_host) = self.child_browsing_context_document_domain_host(handle) else {
            return false;
        };
        let Some(domain) = normalize_document_domain_value(value) else {
            return false;
        };
        if !document_domain_is_allowed_for_host(&current_host, &domain) {
            return false;
        }
        let Some(origin_owner) = self.child_browsing_context_security_origin_owner(handle) else {
            return false;
        };
        match origin_owner {
            ChildSecurityOriginOwner::Main => self.document_domain_override = Some(domain),
            ChildSecurityOriginOwner::Child(owner) => {
                let Some(entry) = self.child_browsing_contexts.get_mut(&owner) else {
                    return false;
                };
                entry.set_document_domain_override(domain);
            }
        }
        true
    }

    pub(crate) fn set_lightweight_popup_document_domain(
        &mut self,
        popup_id: u64,
        document_handle: DomHandle,
        value: &str,
    ) -> bool {
        if self.lightweight_popup_document_domain_is_sandboxed(popup_id) {
            return false;
        }
        let Some(current_host) = self
            .lightweight_popup_origin(popup_id)
            .map(|origin| {
                Url::parse(&origin)
                    .ok()
                    .and_then(|origin| url_host_domain(&origin))
            })
            .unwrap_or_else(|| {
                self.dom_host()
                    .dom()
                    .node(document_handle)
                    .and_then(Node::as_document)
                    .and_then(|document| url_host_domain(document.url()))
            })
        else {
            return false;
        };
        let Some(domain) = normalize_document_domain_value(value) else {
            return false;
        };
        if !document_domain_is_allowed_for_host(&current_host, &domain) {
            return false;
        }
        self.set_lightweight_popup_document_domain_override(popup_id, domain)
    }

    fn main_document_domain_host(&self) -> Option<String> {
        Url::parse(&self.main_document_security_origin())
            .ok()
            .and_then(|origin| url_host_domain(&origin))
    }

    fn child_browsing_context_document_domain_host(&self, handle: DomHandle) -> Option<String> {
        let origin = self.child_browsing_context_window_origin(handle)?;
        Url::parse(&origin)
            .ok()
            .and_then(|origin| url_host_domain(&origin))
    }

    fn child_browsing_context_security_origin_owner(
        &self,
        handle: DomHandle,
    ) -> Option<ChildSecurityOriginOwner> {
        if self.child_browsing_context_has_opaque_origin(handle) {
            return None;
        }
        let mut current = handle;
        let mut remaining = self.child_browsing_contexts.len();
        loop {
            if remaining == 0 {
                return None;
            }
            remaining -= 1;
            let entry = self.child_browsing_contexts.get(&current)?;
            if !entry.security_origin_inherited() {
                return Some(ChildSecurityOriginOwner::Child(current));
            }
            let Some(parent) = self.child_browsing_context_parent_handle(current) else {
                return Some(ChildSecurityOriginOwner::Main);
            };
            current = parent;
        }
    }

    fn child_browsing_context_document_domain_is_sandboxed(&self, handle: DomHandle) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.document_sandbox_policy().sandboxes_document_domain)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_handle_for_stored_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        self.child_browsing_context_document_handles
            .iter()
            .find_map(|(child_handle, child_document_handle)| {
                (*child_document_handle == document_handle).then_some(*child_handle)
            })
    }
}

#[derive(Clone, Copy)]
enum ChildSecurityOriginOwner {
    Main,
    Child(DomHandle),
}

fn url_host_domain(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(domain) => Some(domain.to_ascii_lowercase()),
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
    }
}

fn normalize_document_domain_value(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    // Keep the parsed host's trailing dot: it is significant for origin-domain
    // comparisons, including dots produced by percent-decoding or IDNA.
    match Host::parse(value).ok()? {
        Host::Domain(domain) => Some(domain.to_ascii_lowercase()),
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
    }
}

fn document_domain_is_allowed_for_host(current_host: &str, domain: &str) -> bool {
    if domain == current_host {
        return true;
    }
    if host_is_ip_literal(current_host) || !host_matches_document_domain(current_host, domain) {
        return false;
    }
    // A parent must stay within the original host's registrable domain. Checking
    // only the candidate's PSL entry misses implicit single-label suffixes and
    // parents above a public suffix selected by a wildcard rule.
    host_matches_document_domain(domain, moli_site::registrable_site_host(current_host))
}

fn host_is_ip_literal(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

pub(crate) fn host_matches_document_domain(host: &str, domain: &str) -> bool {
    if host_is_ip_literal(host) {
        return host == domain;
    }
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_domain_validation_respects_the_original_hosts_public_suffix() {
        for (host, domain, allowed) in [
            ("www1.localhost", "localhost", false),
            ("localhost", "localhost", true),
            (
                "www1.web-platform.localhost",
                "web-platform.localhost",
                true,
            ),
            (
                "example.compute.amazonaws.com",
                "compute.amazonaws.com",
                false,
            ),
            (
                "www.example.compute.amazonaws.com",
                "example.compute.amazonaws.com",
                false,
            ),
            ("www.example.compute.amazonaws.com", "amazonaws.com", false),
            ("test.amazonaws.com", "amazonaws.com", true),
            (
                "example.compute.amazonaws.com",
                "example.compute.amazonaws.com",
                true,
            ),
            ("www.city.kawasaki.jp", "city.kawasaki.jp", true),
            ("www.foo.kawasaki.jp", "kawasaki.jp", false),
        ] {
            assert_eq!(
                document_domain_is_allowed_for_host(host, domain),
                allowed,
                "{host} must {}relax to {domain}",
                if allowed { "" } else { "not " }
            );
        }
    }

    #[test]
    fn document_domain_validation_allows_current_or_parent_domain() {
        assert_eq!(
            normalize_document_domain_value("::1").as_deref(),
            Some("::1")
        );
        assert_eq!(
            normalize_document_domain_value("2408:8207:1850:fd60::11").as_deref(),
            Some("2408:8207:1850:fd60::11")
        );
        assert!(document_domain_is_allowed_for_host(
            "www.example.test",
            "example.test"
        ));
        assert!(document_domain_is_allowed_for_host(
            "example.test",
            "example.test"
        ));
        assert!(!document_domain_is_allowed_for_host(
            "www1.localhost",
            "localhost"
        ));
        assert!(document_domain_is_allowed_for_host(
            "127.0.0.1",
            "127.0.0.1"
        ));
        assert!(!document_domain_is_allowed_for_host("127.0.0.1", "0.0.1"));
        assert!(document_domain_is_allowed_for_host("::1", "::1"));
        assert!(!document_domain_is_allowed_for_host("::1", "1"));
        assert!(!document_domain_is_allowed_for_host("example.test", "test"));
        assert!(document_domain_is_allowed_for_host(
            "www.example.co.uk",
            "example.co.uk"
        ));
        assert!(!document_domain_is_allowed_for_host("www.co.uk", "co.uk"));
        assert!(document_domain_is_allowed_for_host(
            "github.io",
            "github.io"
        ));
        assert!(!document_domain_is_allowed_for_host(
            "foo.github.io",
            "github.io"
        ));
        assert!(!document_domain_is_allowed_for_host(
            "example.test",
            "other.test"
        ));
        assert!(!document_domain_is_allowed_for_host(
            "badexample.test",
            "example.test"
        ));
    }
}
