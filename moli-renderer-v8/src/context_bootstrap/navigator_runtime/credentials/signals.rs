//! Opportunistic credential signals without an authenticator backend.
//!
//! The returned Promise describes validation, never whether a credential exists
//! or was changed. There are no available authenticators to notify. Related
//! origins remain unsupported, as advertised by getClientCapabilities().

use crate::{
    native_bridge::{WindowSecurityOrigin, throw_dom_exception},
    webidl,
};

use super::{
    base64url,
    value::{Dictionary, Text},
};

// The derive reads members in declaration order; WebIDL requires lexicographic
// dictionary conversion before any algorithm-specific validation.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "UnknownCredentialOptions")]
struct UnknownCredential {
    #[webidl(required, converter = "raw")]
    credential_id: Text,
    #[webidl(required, converter = "raw")]
    rp_id: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AllAcceptedCredentialsOptions")]
struct AllAcceptedCredentials {
    #[webidl(required, converter = "raw")]
    all_accepted_credential_ids: webidl::Sequence<Text>,
    #[webidl(required, converter = "raw")]
    rp_id: Text,
    #[webidl(required, converter = "raw")]
    user_id: Text,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CurrentUserDetailsOptions")]
struct CurrentUserDetails {
    #[webidl(required, converter = "raw")]
    display_name: Text,
    #[webidl(required, converter = "raw")]
    name: Text,
    #[webidl(required, converter = "raw")]
    rp_id: Text,
    #[webidl(required, converter = "raw")]
    user_id: Text,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.signalUnknownCredential")]
struct UnknownCredentialArgs {
    #[webidl(required, converter = "raw")]
    options: Dictionary<UnknownCredential>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.signalAllAcceptedCredentials")]
struct AllAcceptedCredentialsArgs {
    #[webidl(required, converter = "raw")]
    options: Dictionary<AllAcceptedCredentials>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.signalCurrentUserDetails")]
struct CurrentUserDetailsArgs {
    #[webidl(required, converter = "raw")]
    options: Dictionary<CurrentUserDetails>,
}

fn valid_base64url(scope: &mut v8::PinScope<'_, '_>, text: &Text) -> bool {
    if base64url::decode(text).is_some() {
        return true;
    }
    webidl::throw_type_error(scope, "Invalid base64url data.");
    false
}

fn domain_suffix_matches(suffix: &str, domain: &str) -> bool {
    domain == suffix
        || domain
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn rp_id_matches(rp_id: &str, effective_domain: &str) -> bool {
    // Valid domain excludes IP hosts and requires strict UTS #46 validation,
    // unlike URL host parsing (which also accepts underscores and long labels).
    if idna::domain_to_ascii_strict(effective_domain).is_err()
        || !matches!(url::Host::parse(effective_domain), Ok(url::Host::Domain(_)))
    {
        return false;
    }
    let Ok(url::Host::Domain(candidate)) = url::Host::parse(rp_id) else {
        return false;
    };
    // Bound suffixes by the original domain's eTLD+1. This also excludes
    // implicit single-label suffixes and parents above wildcard PSL entries.
    domain_suffix_matches(&candidate, effective_domain)
        && domain_suffix_matches(
            moli_site::registrable_site_host(effective_domain),
            &candidate,
        )
}

fn finish<'s>(scope: &mut v8::PinScope<'s, '_>, rp_id: &Text, rv: &mut v8::ReturnValue<'s>) {
    // Static bindings use the function's relevant settings object. Retained
    // functions keep their realm's origin after navigation; public location,
    // origin, document.domain and constructor receivers cannot supply it.
    let effective_domain = WindowSecurityOrigin::for_context(scope.get_current_context())
        .and_then(|origin| origin.effective_domain());
    let candidate = String::from_utf16(&rp_id.0.0).ok();
    if !effective_domain
        .as_deref()
        .zip(candidate.as_deref())
        .is_some_and(|(domain, candidate)| rp_id_matches(candidate, domain))
    {
        throw_dom_exception(
            scope,
            "SecurityError",
            18,
            "The RP ID is not valid for this origin.",
        );
        return;
    }
    let value = v8::undefined(scope).into();
    super::super::super::stream_adapter::set_resolved_promise(scope, rv, value);
}

pub(super) fn unknown_credential<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<UnknownCredentialArgs>(scope, &args) else {
        return;
    };
    let options = parsed.options.0;
    if valid_base64url(scope, &options.credential_id) {
        finish(scope, &options.rp_id, &mut rv);
    }
}

pub(super) fn all_accepted_credentials<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<AllAcceptedCredentialsArgs>(scope, &args) else {
        return;
    };
    let options = parsed.options.0;
    if valid_base64url(scope, &options.user_id)
        && options
            .all_accepted_credential_ids
            .0
            .iter()
            .all(|id| valid_base64url(scope, id))
    {
        finish(scope, &options.rp_id, &mut rv);
    }
}

pub(super) fn current_user_details<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CurrentUserDetailsArgs>(scope, &args) else {
        return;
    };
    let options = parsed.options.0;
    // Labels still undergo DOMString conversion even without a backend.
    let _ = (&options.display_name, &options.name);
    if valid_base64url(scope, &options.user_id) {
        finish(scope, &options.rp_id, &mut rv);
    }
}

#[cfg(test)]
mod tests {
    use super::rp_id_matches;

    #[test]
    fn rp_id_validation_uses_strict_domains_host_parsing_and_psl_boundaries() {
        for (domain, rp_id, expected) in [
            ("www.example.com", "example.com", true),
            ("www.example.com", "EXAMPLE.COM", true),
            ("www.example.com", "%65xample.com", true),
            ("www.example.com", "com", false),
            ("www.example.com", "evil-example.com", false),
            ("www.example.com", "example.com:443", false),
            ("www.example.com", "https://example.com", false),
            ("www.example.com", "example.com/", false),
            ("www.example.com", "example.com.", false),
            ("www.example.com.", "example.com.", false),
            ("www.example.com.", "example.com", false),
            ("localhost", "localhost", true),
            ("www.localhost", "localhost", false),
            ("www.web-platform.localhost", "web-platform.localhost", true),
            ("foo.github.io", "github.io", false),
            ("www.foo.github.io", "foo.github.io", true),
            (
                "www.example.compute.amazonaws.com",
                "example.compute.amazonaws.com",
                false,
            ),
            ("www.example.compute.amazonaws.com", "amazonaws.com", false),
            ("www.ck", "www.ck", true),
            ("www.ck", "ck", false),
            ("www.xn--bcher-kva.example", "bücher.example", true),
            ("www.example", "example", false),
            ("127.0.0.1", "127.0.0.1", false),
            ("[::1]", "[::1]", false),
            ("_invalid.example", "_invalid.example", false),
            ("example.com", "", false),
        ] {
            assert_eq!(
                rp_id_matches(rp_id, domain),
                expected,
                "{rp_id:?} for {domain:?}"
            );
        }
        let long_label = format!("{}.example", "a".repeat(64));
        assert!(!rp_id_matches(&long_label, &long_label));
    }
}
