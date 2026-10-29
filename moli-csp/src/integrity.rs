use moli_crypto::DigestAlgorithm;

pub struct IntegrityMetadataHash<'a> {
    pub algorithm: DigestAlgorithm,
    pub digest: &'a str,
}

/// Keep the encoded digest for CSP's literal hash-source comparison. Response
/// integrity verification separately decodes it and selects the strongest hash.
pub fn integrity_metadata_hashes(
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
