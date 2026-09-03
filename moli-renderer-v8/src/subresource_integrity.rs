use base64::Engine;
use moli_crypto::DigestAlgorithm;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SubresourceIntegrityAlgorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl SubresourceIntegrityAlgorithm {
    fn output_len_bytes(self) -> usize {
        self.digest_algorithm().output_len_bytes()
    }

    fn digest_algorithm(self) -> DigestAlgorithm {
        match self {
            Self::Sha256 => DigestAlgorithm::Sha256,
            Self::Sha384 => DigestAlgorithm::Sha384,
            Self::Sha512 => DigestAlgorithm::Sha512,
        }
    }
}

struct ParsedIntegrityMetadata {
    tokens: Vec<ParsedIntegrityToken>,
}

struct ParsedIntegrityToken {
    algorithm: SubresourceIntegrityAlgorithm,
    expected_digest: Option<Vec<u8>>,
}

pub(crate) fn response_body_matches_subresource_integrity_metadata(
    body: &[u8],
    integrity: Option<&str>,
) -> bool {
    let Some(integrity) = integrity
        .map(str::trim)
        .filter(|integrity| !integrity.is_empty())
    else {
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
    let actual_digest = strongest_algorithm.digest_algorithm().digest_bytes(body);
    metadata.tokens.iter().any(|token| {
        token.algorithm == strongest_algorithm
            && token
                .expected_digest
                .as_deref()
                .is_some_and(|expected_digest| expected_digest == actual_digest)
    })
}

fn parse_integrity_metadata(integrity: &str) -> ParsedIntegrityMetadata {
    let tokens = integrity
        .split_ascii_whitespace()
        .filter_map(parse_integrity_token)
        .collect();
    ParsedIntegrityMetadata { tokens }
}

fn parse_integrity_token(token: &str) -> Option<ParsedIntegrityToken> {
    let (algorithm, digest) = parse_integrity_algorithm_and_digest(token)?;
    let digest = digest.split_once('?').map_or(digest, |(digest, _)| digest);
    if !is_integrity_digest_syntax(digest) {
        return None;
    }
    let expected_digest = decode_integrity_digest(digest);
    Some(ParsedIntegrityToken {
        algorithm,
        expected_digest,
    })
}

fn is_integrity_digest_syntax(digest: &str) -> bool {
    if digest.is_empty() {
        return false;
    }
    let mut padding = 0_u8;
    let mut data_characters = 0_usize;
    for byte in digest.bytes() {
        if byte == b'=' {
            padding = padding.saturating_add(1);
            if padding > 2 {
                return false;
            }
            continue;
        }
        if padding != 0
            || !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_'))
        {
            return false;
        }
        data_characters += 1;
    }
    data_characters != 0
}

fn parse_integrity_algorithm_and_digest(
    token: &str,
) -> Option<(SubresourceIntegrityAlgorithm, &str)> {
    const PREFIXES: &[(&str, SubresourceIntegrityAlgorithm)] = &[
        ("sha256", SubresourceIntegrityAlgorithm::Sha256),
        ("sha-256", SubresourceIntegrityAlgorithm::Sha256),
        ("sha384", SubresourceIntegrityAlgorithm::Sha384),
        ("sha-384", SubresourceIntegrityAlgorithm::Sha384),
        ("sha512", SubresourceIntegrityAlgorithm::Sha512),
        ("sha-512", SubresourceIntegrityAlgorithm::Sha512),
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
    [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::STANDARD_NO_PAD,
        &base64::engine::general_purpose::URL_SAFE,
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ]
    .into_iter()
    .find_map(|engine| engine.decode(digest).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_integrity_metadata_parses_matching_supported_hash() {
        let body = b"console.log('integrity ok')";
        let digest = base64::engine::general_purpose::STANDARD
            .encode(DigestAlgorithm::Sha384.digest_bytes(body));
        let integrity = format!("sha384-{digest}");

        let metadata = parse_integrity_metadata(&integrity);
        assert_eq!(metadata.tokens.len(), 1);
        assert_eq!(
            metadata.tokens[0].algorithm,
            SubresourceIntegrityAlgorithm::Sha384
        );
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
        assert_eq!(strongest, Some(SubresourceIntegrityAlgorithm::Sha384));
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
