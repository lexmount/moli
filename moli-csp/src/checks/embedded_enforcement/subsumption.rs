use super::super::{
    BASE_URI, CHILD_SRC, CONNECT_SRC, ContentSecurityPolicy, CspHashAlgorithm, DEFAULT_SRC,
    FONT_SRC, FRAME_ANCESTORS, FRAME_SRC, IMG_SRC, MANIFEST_SRC, MEDIA_SRC, SCRIPT_SRC,
    SCRIPT_SRC_ATTR, SCRIPT_SRC_ELEM, STYLE_SRC, STYLE_SRC_ATTR, STYLE_SRC_ELEM, WORKER_SRC,
    csp_hash_source_value, csp_keyword_eq, csp_nonce_source_value, directive_source_list,
};
use super::sources::UrlSource;
use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use url::Url;

const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

// A missing directive is unrestricted, not an empty source list. Evaluate
// every effective directive, including independent element/attribute fallbacks.
const DIRECTIVES: &[&[&str]] = &[
    &[CHILD_SRC, DEFAULT_SRC],
    &[CONNECT_SRC, DEFAULT_SRC],
    &[FONT_SRC, DEFAULT_SRC],
    &[FRAME_SRC, CHILD_SRC, DEFAULT_SRC],
    &[IMG_SRC, DEFAULT_SRC],
    &[MANIFEST_SRC, DEFAULT_SRC],
    &[MEDIA_SRC, DEFAULT_SRC],
    &["object-src", DEFAULT_SRC],
    &[SCRIPT_SRC, DEFAULT_SRC],
    &[SCRIPT_SRC_ATTR, SCRIPT_SRC, DEFAULT_SRC],
    &[SCRIPT_SRC_ELEM, SCRIPT_SRC, DEFAULT_SRC],
    &[STYLE_SRC, DEFAULT_SRC],
    &[STYLE_SRC_ATTR, STYLE_SRC, DEFAULT_SRC],
    &[STYLE_SRC_ELEM, STYLE_SRC, DEFAULT_SRC],
    &[WORKER_SRC, CHILD_SRC, SCRIPT_SRC, DEFAULT_SRC],
    &[BASE_URI],
    &[FRAME_ANCESTORS],
    &["form-action"],
    &["fenced-frame-src", DEFAULT_SRC],
];

/// Whether all behavior permitted by the returned policy list is permitted by
/// the required policy. Nonces compare by presence after intersecting the
/// returned list; their actual values must not become an embedder oracle.
pub(super) fn subsumes(
    required: &ContentSecurityPolicy,
    returned: &[ContentSecurityPolicy],
    origin: &Url,
) -> bool {
    DIRECTIVES.iter().all(|fallbacks| {
        let Some((name, tokens)) = fallbacks
            .iter()
            .find_map(|name| directive_source_list(required, name).map(|tokens| (*name, tokens)))
        else {
            return true;
        };
        let required = SourceList::parse(&tokens, origin);
        let lists = returned
            .iter()
            .filter_map(|policy| {
                fallbacks
                    .iter()
                    .find_map(|name| directive_source_list(policy, name))
                    .map(|tokens| SourceList::parse(&tokens, origin))
            })
            .collect::<Vec<_>>();
        if lists.is_empty() {
            return false;
        }
        required.subsumes(&lists, name)
    })
}

#[derive(Default)]
struct SourceList {
    urls: Vec<UrlSource>,
    nonces: Vec<String>,
    hashes: Vec<(CspHashAlgorithm, Vec<u8>)>,
    inline: bool,
    eval: bool,
    wasm_eval: bool,
    trusted_eval: bool,
    dynamic: bool,
    unsafe_hashes: bool,
}

impl SourceList {
    fn parse(tokens: &[&str], origin: &Url) -> Self {
        let mut result = Self::default();
        for token in tokens {
            if let Some(nonce) = csp_nonce_source_value(token) {
                result.nonces.push(nonce.to_owned());
            } else if let Some(hash) = csp_hash_source_value(token) {
                let normalized = hash.digest.replace('-', "+").replace('_', "/");
                if let Ok(digest) = BASE64.decode(normalized) {
                    result.hashes.push((hash.algorithm, digest));
                }
            } else {
                result.inline |= csp_keyword_eq(token, "unsafe-inline");
                result.eval |= csp_keyword_eq(token, "unsafe-eval");
                result.wasm_eval |= csp_keyword_eq(token, "wasm-unsafe-eval");
                result.trusted_eval |= csp_keyword_eq(token, "trusted-types-eval");
                result.dynamic |= csp_keyword_eq(token, "strict-dynamic");
                result.unsafe_hashes |= csp_keyword_eq(token, "unsafe-hashes");
                result.urls.extend(UrlSource::parse(token, origin));
            }
        }
        result.wasm_eval |= result.eval;
        result.trusted_eval |= result.eval;
        result
    }

    fn subsumes(&self, others: &[Self], directive: &str) -> bool {
        let script = directive == DEFAULT_SRC || directive.starts_with("script-src");
        let style = directive == DEFAULT_SRC || directive.starts_with("style-src");
        if (script || style)
            && ((!self.eval && others.iter().all(|policy| policy.eval))
                || (!self.unsafe_hashes && others.iter().all(|policy| policy.unsafe_hashes))
                || (script
                    && ((!self.wasm_eval && others.iter().all(|policy| policy.wasm_eval))
                        || (!self.trusted_eval
                            && others.iter().all(|policy| policy.trusted_eval)))))
        {
            return false;
        }
        if script || style {
            let inline = ExecutionKind::Inline {
                script,
                attribute: directive.ends_with("-attr"),
            };
            if has_uncovered_execution(self, others, &ExecutionConstraint::default(), inline) {
                return false;
            }
            if directive.ends_with("-attr") {
                return true;
            }
            !has_uncovered_execution(
                self,
                others,
                &ExecutionConstraint::default(),
                ExecutionKind::External { script },
            )
        } else {
            !has_uncovered_execution(
                self,
                others,
                &ExecutionConstraint::default(),
                ExecutionKind::Url,
            )
        }
    }

    fn allows_all_inline(&self, script: bool) -> bool {
        self.inline
            && self.nonces.is_empty()
            && self.hashes.is_empty()
            && (!script || !self.dynamic)
    }

    fn alternatives(&self, kind: ExecutionKind) -> Vec<Permit<'_>> {
        let mut result = Vec::new();
        match kind {
            ExecutionKind::Inline { script, attribute } => {
                if self.allows_all_inline(script) {
                    return vec![Permit::Any];
                }
                if !attribute {
                    result.extend(self.nonces.iter().map(|value| Permit::Nonce(value)));
                }
                if !attribute || self.unsafe_hashes {
                    result.extend(self.hashes.iter().map(Permit::Hash));
                }
            }
            ExecutionKind::External { script } => {
                if script && self.dynamic {
                    result.push(Permit::Dynamic);
                } else {
                    result.extend(self.urls.iter().map(Permit::Url));
                }
                result.extend(self.nonces.iter().map(|value| Permit::Nonce(value)));
                result.extend(self.hashes.iter().map(Permit::Hash));
            }
            ExecutionKind::Url => result.extend(self.urls.iter().map(Permit::Url)),
        }
        result
    }

    fn covers(&self, execution: &ExecutionConstraint, kind: ExecutionKind) -> bool {
        for permit in self.alternatives(kind) {
            // Compare required nonce values by presence, without allowing the
            // embedder to probe which nonce the response actually chose.
            if matches!(permit, Permit::Nonce(_)) && execution.nonce.is_some() {
                return true;
            }
            if execution.satisfies(&permit) {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy)]
enum ExecutionKind {
    Url,
    Inline { script: bool, attribute: bool },
    External { script: bool },
}

#[derive(Clone, Copy)]
enum Permit<'a> {
    Any,
    Url(&'a UrlSource),
    Nonce(&'a str),
    Hash(&'a (CspHashAlgorithm, Vec<u8>)),
    Dynamic,
}

/// A conjunction of response-policy conditions, including combinations that
/// cannot be serialized as a single CSP source list (such as nonce AND hash).
#[derive(Clone, Default)]
struct ExecutionConstraint {
    url: Option<UrlSource>,
    nonce: Option<String>,
    hashes: Vec<(CspHashAlgorithm, Vec<u8>)>,
    dynamic: bool,
}

impl ExecutionConstraint {
    fn satisfies(&self, permit: &Permit<'_>) -> bool {
        match permit {
            Permit::Any => true,
            Permit::Url(source) => self.url.as_ref().is_some_and(|url| source.subsumes(url)),
            Permit::Nonce(nonce) => self.nonce.as_deref() == Some(*nonce),
            Permit::Hash(hash) => self.hashes.contains(hash),
            Permit::Dynamic => self.dynamic,
        }
    }

    fn intersect(&self, permit: Permit<'_>) -> Option<Self> {
        let mut result = self.clone();
        match permit {
            Permit::Any => {}
            Permit::Url(source) => {
                result.url = Some(match &self.url {
                    Some(url) => url.intersect(source)?,
                    None => source.clone(),
                });
            }
            Permit::Nonce(nonce) => {
                if self.nonce.as_ref().is_some_and(|value| value != nonce) {
                    return None;
                }
                result.nonce = Some(nonce.to_owned());
            }
            Permit::Hash(hash) => {
                if self
                    .hashes
                    .iter()
                    .any(|value| value.0 == hash.0 && value.1 != hash.1)
                {
                    return None;
                }
                if !self.hashes.contains(hash) {
                    result.hashes.push(hash.clone());
                }
            }
            Permit::Dynamic => result.dynamic = true,
        }
        Some(result)
    }
}

fn has_uncovered_execution(
    required: &SourceList,
    mut returned: &[SourceList],
    execution: &ExecutionConstraint,
    kind: ExecutionKind,
) -> bool {
    if required.covers(execution, kind) {
        return false;
    }
    loop {
        let Some((policy, remaining)) = returned.split_first() else {
            return true;
        };
        let alternatives = policy.alternatives(kind);
        if alternatives
            .iter()
            .any(|permit| execution.satisfies(permit))
        {
            returned = remaining;
            continue;
        }
        for permit in alternatives {
            if let Some(next) = execution.intersect(permit)
                && has_uncovered_execution(required, remaining, &next, kind)
            {
                return true;
            }
        }
        return false;
    }
}
