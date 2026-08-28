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
                .unwrap_or_else(|| url_host_domain(self.document_url()).unwrap_or_default());
        }
        if let Some(child_handle) =
            self.child_browsing_context_handle_for_stored_document(document_handle)
        {
            return self.child_browsing_context_document_domain_value(child_handle);
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
            let Some(current_host) = url_host_domain(self.document_url()) else {
                return false;
            };
            let Some(domain) = normalize_document_domain_value(value) else {
                return false;
            };
            if !document_domain_is_allowed_for_host(&current_host, &domain) {
                return false;
            }
            self.document_domain_override = Some(domain.clone());
            if let Some(environment) = self.page_script_environment.as_ref() {
                environment.replicate_current_top_level_document(
                    self.document_url(),
                    &self.main_document_serialized_origin,
                    self.main_window_opaque_origin_nonce(),
                    Some(domain),
                );
            }
            return true;
        }
        if let Some(child_handle) =
            self.child_browsing_context_handle_for_stored_document(document_handle)
        {
            return self.set_child_browsing_context_document_domain(child_handle, value);
        }
        false
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
            ChildSecurityOriginOwner::Main => {
                self.document_domain_override = Some(domain.clone());
                if let Some(environment) = self.page_script_environment.as_ref() {
                    environment.replicate_current_top_level_document(
                        self.document_url(),
                        &self.main_document_serialized_origin,
                        self.main_window_opaque_origin_nonce(),
                        Some(domain),
                    );
                }
            }
            ChildSecurityOriginOwner::Child(owner) => {
                let Some(entry) = self.child_browsing_contexts.get_mut(&owner) else {
                    return false;
                };
                entry.set_document_domain_override(domain);
            }
        }
        self.publish_related_page_remote_frame_tree();
        true
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
        Host::Domain(domain) => Some(domain.trim_end_matches('.').to_ascii_lowercase()),
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
    }
}

fn normalize_document_domain_value(value: &str) -> Option<String> {
    let value = value.trim_end_matches('.');
    if value.is_empty() {
        return None;
    }
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip.to_string());
    }
    match Host::parse(value).ok()? {
        Host::Domain(domain) => Some(domain.trim_end_matches('.').to_ascii_lowercase()),
        Host::Ipv4(ip) => Some(ip.to_string()),
        Host::Ipv6(ip) => Some(ip.to_string()),
    }
}

fn document_domain_is_allowed_for_host(current_host: &str, domain: &str) -> bool {
    if host_is_ip_literal(current_host) {
        return domain == current_host;
    }
    if !host_matches_document_domain(current_host, domain) {
        return false;
    }
    if domain != current_host && moli_cookie_jar::host_is_public_suffix(domain) {
        return false;
    }
    domain == current_host || domain == "localhost" || domain.contains('.')
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
        assert!(document_domain_is_allowed_for_host(
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
