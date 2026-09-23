use base64::Engine;
use moli_crypto::DigestAlgorithm;

#[derive(PartialEq, Eq)]
pub(crate) struct IntegrityMetadataHash<'a> {
    pub(crate) algorithm: DigestAlgorithm,
    pub(crate) digest: &'a str,
}

/// Preload consumption compares parsed metadata sets, including weaker hashes,
/// while ignoring unknown algorithms, duplicate tokens, and unknown options.
/// A consumer with no supported metadata can consume any matching preload,
/// including one whose integrity check failed.
pub(crate) fn integrity_metadata_allows_preload_consumption(
    preload: &str,
    consumer: Option<&str>,
) -> bool {
    let consumer: Vec<_> = integrity_metadata_hashes(consumer.unwrap_or_default()).collect();
    if consumer.is_empty() {
        return true;
    }
    let preload: Vec<_> = integrity_metadata_hashes(preload).collect();
    consumer.iter().all(|hash| preload.contains(hash))
        && preload.iter().all(|hash| consumer.contains(hash))
}

struct ParsedIntegrityMetadata {
    tokens: Vec<ParsedIntegrityToken>,
}

struct ParsedIntegrityToken {
    algorithm: DigestAlgorithm,
    expected_digest: Option<Vec<u8>>,
}

/// HTML scripts ignore empty or unsupported metadata. With supported metadata,
/// check response eligibility before hashing: observing a digest match against
/// opaque internal bytes would expose a cross-origin content oracle.
pub(crate) fn response_matches_subresource_integrity_metadata(
    body: &[u8],
    integrity: Option<&str>,
    response_is_eligible: bool,
) -> bool {
    let Some(integrity) = integrity.filter(|integrity| !integrity.is_empty()) else {
        return true;
    };
    let metadata = parse_integrity_metadata(integrity);
    let Some(strongest_algorithm) = metadata
        .tokens
        .iter()
        .map(|token| token.algorithm)
        .max_by_key(|algorithm| algorithm.output_len_bytes())
    else {
        return true;
    };
    if !response_is_eligible {
        return false;
    }
    let actual_digest = strongest_algorithm.digest_bytes(body);
    metadata.tokens.iter().any(|token| {
        token.algorithm == strongest_algorithm
            && token
                .expected_digest
                .as_deref()
                .is_some_and(|expected_digest| expected_digest == actual_digest)
    })
}

fn parse_integrity_metadata(integrity: &str) -> ParsedIntegrityMetadata {
    let tokens = integrity_metadata_hashes(integrity)
        .map(|hash| ParsedIntegrityToken {
            algorithm: hash.algorithm,
            expected_digest: decode_integrity_digest(hash.digest),
        })
        .collect();
    ParsedIntegrityMetadata { tokens }
}

/// Keep the encoded digest for CSP's literal hash-source comparison. Response
/// integrity verification separately decodes it and selects the strongest hash.
pub(crate) fn integrity_metadata_hashes(
    integrity: &str,
) -> impl Iterator<Item = IntegrityMetadataHash<'_>> {
    integrity
        .split_ascii_whitespace()
        .filter_map(parse_integrity_hash)
}

fn parse_integrity_hash(token: &str) -> Option<IntegrityMetadataHash<'_>> {
    let (algorithm, digest) = parse_integrity_algorithm_and_digest(token)?;
    let digest = digest.split_once('?').map_or(digest, |(digest, _)| digest);
    if !is_integrity_digest_syntax(digest) {
        return None;
    }
    Some(IntegrityMetadataHash { algorithm, digest })
}

fn is_integrity_digest_syntax(digest: &str) -> bool {
    // Recognizing metadata and decoding its digest are separate steps. Keep
    // hashes containing misplaced or excess padding: a decode failure must
    // fail verification, not erase a supported (possibly stronger) algorithm.
    !digest.is_empty()
        && digest.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_' | b'=')
        })
}

fn parse_integrity_algorithm_and_digest(token: &str) -> Option<(DigestAlgorithm, &str)> {
    const PREFIXES: &[(&str, DigestAlgorithm)] = &[
        ("sha256", DigestAlgorithm::Sha256),
        ("sha-256", DigestAlgorithm::Sha256),
        ("sha384", DigestAlgorithm::Sha384),
        ("sha-384", DigestAlgorithm::Sha384),
        ("sha512", DigestAlgorithm::Sha512),
        ("sha-512", DigestAlgorithm::Sha512),
    ];
    for (prefix, algorithm) in PREFIXES {
        let Some(rest) = token.strip_prefix(prefix) else {
            continue;
        };
        let Some(digest) = rest.strip_prefix('-') else {
            continue;
        };
        return Some((*algorithm, digest));
    }
    None
}

fn decode_integrity_digest(digest: &str) -> Option<Vec<u8>> {
    // SRI accepts unpadded and non-canonical padded representations. Normalize
    // only for byte verification; CSP and preload metadata comparison retain
    // the encoded value. Interior padding still causes decoding to fail.
    const DECODER: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::general_purpose::NO_PAD.with_decode_allow_trailing_bits(true),
    );
    let digest = digest.trim_end_matches('=');
    let normalized;
    let digest = if digest.contains(['-', '_']) {
        normalized = digest.replace('-', "+").replace('_', "/");
        &normalized
    } else {
        digest
    };
    DECODER.decode(digest).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_supported_digests_do_not_disable_integrity_or_downgrade_the_algorithm() {
        let body = b"integrity must still be checked";
        let valid = format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD
                .encode(DigestAlgorithm::Sha256.digest_bytes(body))
        );
        for malformed in [
            "sha512-AAAA===",
            "sha512-A=AAA",
            "sha512-====",
            "sha-512-AAAA===",
        ] {
            for metadata in [malformed.to_owned(), format!("{valid} {malformed}")] {
                for eligible in [true, false] {
                    assert!(
                        !response_matches_subresource_integrity_metadata(
                            body,
                            Some(&metadata),
                            eligible
                        ),
                        "{metadata}"
                    );
                }
            }
            assert!(!integrity_metadata_allows_preload_consumption(
                &valid,
                Some(malformed)
            ));
            assert!(integrity_metadata_allows_preload_consumption(
                malformed,
                Some(malformed)
            ));
            assert!(!integrity_metadata_allows_preload_consumption(
                "",
                Some(malformed)
            ));
        }
    }

    #[test]
    fn integrity_verification_accepts_noncanonical_base64_without_changing_metadata_identity() {
        let body = b"noncanonical integrity";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha256.digest_bytes(body));
        let metadata = format!("sha256-{digest}");
        for padding in ["", "=", "===", "======="] {
            let padded = format!("sha256-{}{padding}", digest.trim_end_matches('='));
            assert!(response_matches_subresource_integrity_metadata(
                body,
                Some(&padded),
                true
            ));
            assert!(!response_matches_subresource_integrity_metadata(
                body,
                Some(&padded),
                false
            ));
            assert_eq!(
                integrity_metadata_allows_preload_consumption(&metadata, Some(&padded)),
                metadata == padded
            );
        }
        // Only the top four bits of the final SHA-256 base64 symbol carry data.
        let mut noncanonical = digest.trim_end_matches('=').as_bytes().to_vec();
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let last = noncanonical.last_mut().unwrap();
        *last = alphabet[alphabet.iter().position(|byte| byte == last).unwrap() + 1];
        let noncanonical = format!("sha256-{}", String::from_utf8(noncanonical).unwrap());
        assert!(response_matches_subresource_integrity_metadata(
            body,
            Some(&noncanonical),
            true
        ));
        assert!(!integrity_metadata_allows_preload_consumption(
            &metadata,
            Some(&noncanonical)
        ));
    }

    #[test]
    fn preload_integrity_compares_sets_including_weaker_hashes() {
        let preload = "sha384-AAAA sha256-BBBB";
        for compatible in [
            None,
            Some(""),
            Some("unknown-AAAA"),
            Some("sha256-BBBB sha384-AAAA"),
            Some("sha384-AAAA?ignored sha256-BBBB sha384-AAAA unknown-AAAA"),
        ] {
            assert!(
                integrity_metadata_allows_preload_consumption(preload, compatible),
                "{compatible:?}"
            );
        }
        for incompatible in ["sha384-AAAA", "sha384-AAAA sha256-CCCC", "sha512-AAAA"] {
            assert!(
                !integrity_metadata_allows_preload_consumption(preload, Some(incompatible)),
                "{incompatible}"
            );
        }
        assert!(!integrity_metadata_allows_preload_consumption(
            "",
            Some("sha384-AAAA")
        ));
    }

    fn response_body_matches_subresource_integrity_metadata(
        body: &[u8],
        integrity: Option<&str>,
    ) -> bool {
        response_matches_subresource_integrity_metadata(body, integrity, true)
    }

    #[test]
    fn integrity_rejects_ineligible_responses_even_with_matching_metadata() {
        let body = b"console.log('opaque body')";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));
        let matching = format!("sha384-{digest}");
        assert!(response_matches_subresource_integrity_metadata(
            body,
            Some(&matching),
            true
        ));
        assert!(!response_matches_subresource_integrity_metadata(
            body,
            Some(&matching),
            false
        ));
    }

    #[test]
    fn empty_or_ignored_integrity_does_not_require_a_readable_response() {
        for integrity in [
            None,
            Some(""),
            Some(" \t\n"),
            Some("sha384-***"),
            Some("sha1-ignored"),
        ] {
            for response_is_eligible in [true, false] {
                assert!(response_matches_subresource_integrity_metadata(
                    b"opaque body",
                    integrity,
                    response_is_eligible,
                ));
            }
        }
    }

    #[test]
    fn script_integrity_metadata_parses_matching_supported_hash() {
        let body = b"console.log('integrity ok')";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));
        let integrity = format!("sha384-{digest}");

        let metadata = parse_integrity_metadata(&integrity);
        assert_eq!(metadata.tokens.len(), 1);
        assert_eq!(metadata.tokens[0].algorithm, DigestAlgorithm::Sha384);
        assert_eq!(
            metadata.tokens[0].expected_digest.as_ref().map(Vec::len),
            Some(48)
        );
        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&integrity)
        ));
    }

    #[test]
    fn script_integrity_metadata_accepts_unpadded_supported_hashes() {
        let body = b"console.log('unpadded integrity')";
        let sha256 = base64::engine::general_purpose::STANDARD_NO_PAD
            .encode(DigestAlgorithm::Sha256.digest_bytes(body));
        let sha512 = base64::engine::general_purpose::STANDARD_NO_PAD
            .encode(DigestAlgorithm::Sha512.digest_bytes(body));

        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&format!("sha256-{sha256}"))
        ));
        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&format!("sha512-{sha512}"))
        ));
    }

    #[test]
    fn script_integrity_metadata_accepts_chromium_algorithm_aliases() {
        let body = b"console.log('chromium algorithm aliases')";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));

        let metadata = parse_integrity_metadata(&format!("sha-384-{digest}"));
        assert_eq!(metadata.tokens.len(), 1);
    }

    #[test]
    fn script_integrity_metadata_accepts_base64url_hashes() {
        let body = b"console.log('base64url integrity')";
        let digest = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));

        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&format!("sha384-{digest}"))
        ));
    }

    #[test]
    fn script_integrity_metadata_ignores_unrecognized_algorithms() {
        let metadata = parse_integrity_metadata("sha1-this-is-ignored");
        assert!(metadata.tokens.is_empty());
    }

    #[test]
    fn script_integrity_matching_response_uses_raw_body_bytes() {
        let body = b"console.log('integrity ok')";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));

        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&format!("sha384-{digest}"))
        ));
        assert!(!response_body_matches_subresource_integrity_metadata(
            b"console.log('different')",
            Some(&format!("sha384-{digest}"))
        ));
    }

    #[test]
    fn script_integrity_metadata_tracks_strongest_supported_hash() {
        let body = b"console.log('strongest')";
        let weak = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha256.digest_bytes(b"wrong"));
        let strong = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));
        let integrity = format!("sha256-{weak} sha384-{strong}");

        let strongest = parse_integrity_metadata(&integrity)
            .tokens
            .iter()
            .map(|token| token.algorithm)
            .max_by_key(|algorithm| algorithm.output_len_bytes());
        assert_eq!(strongest, Some(DigestAlgorithm::Sha384));
        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&integrity)
        ));
    }

    #[test]
    fn script_integrity_rejects_when_strongest_supported_hash_does_not_match() {
        let body = b"console.log('strongest wrong length')";
        let weak = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));
        let integrity = format!("sha384-{weak} sha512-aW52YWxpZA==");

        assert!(!response_body_matches_subresource_integrity_metadata(
            body,
            Some(&integrity)
        ));
    }

    #[test]
    fn script_integrity_treats_syntactic_but_noncanonical_digest_as_mismatch() {
        assert!(!response_body_matches_subresource_integrity_metadata(
            b"console.log('body')",
            Some("sha384-foobar")
        ));
    }

    #[test]
    fn script_integrity_allows_empty_invalid_or_unsupported_metadata() {
        let body = b"console.log('no supported metadata')";

        assert!(response_body_matches_subresource_integrity_metadata(
            body, None
        ));
        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some("")
        ));
        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some("sha384-*** sha1-ignored")
        ));
    }

    #[test]
    fn script_integrity_accepts_any_matching_digest_at_strongest_level() {
        let body = b"console.log('one of two')";
        let wrong = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha512.digest_bytes(b"wrong"));
        let matching = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha512.digest_bytes(body));
        let weaker = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(b"wrong"));
        let integrity = format!("sha384-{weaker} sha512-{wrong} sha512-{matching}?ignored");

        assert!(response_body_matches_subresource_integrity_metadata(
            body,
            Some(&integrity)
        ));
    }
}
