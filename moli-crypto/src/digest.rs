use aws_lc_rs::digest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestAlgorithm {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlgorithm {
    pub fn digest_bytes(self, data: impl AsRef<[u8]>) -> Vec<u8> {
        digest::digest(self.aws_lc_algorithm(), data.as_ref())
            .as_ref()
            .to_vec()
    }

    pub fn output_len_bytes(self) -> usize {
        self.aws_lc_algorithm().output_len()
    }

    fn aws_lc_algorithm(self) -> &'static digest::Algorithm {
        match self {
            Self::Sha1 => &digest::SHA1_FOR_LEGACY_USE_ONLY,
            Self::Sha256 => &digest::SHA256,
            Self::Sha384 => &digest::SHA384,
            Self::Sha512 => &digest::SHA512,
        }
    }
}

pub struct Sha256Context {
    context: digest::Context,
}

impl Sha256Context {
    pub fn new() -> Self {
        Self {
            context: digest::Context::new(&digest::SHA256),
        }
    }

    pub fn update(&mut self, data: impl AsRef<[u8]>) {
        self.context.update(data.as_ref());
    }

    pub fn finish(self) -> [u8; 32] {
        digest_array(self.context.finish())
    }
}

impl Default for Sha256Context {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sha1_digest(data: impl AsRef<[u8]>) -> [u8; 20] {
    digest_array(digest::digest(
        &digest::SHA1_FOR_LEGACY_USE_ONLY,
        data.as_ref(),
    ))
}

pub fn sha256_digest(data: impl AsRef<[u8]>) -> [u8; 32] {
    digest_array(digest::digest(&digest::SHA256, data.as_ref()))
}

pub fn sha256_hex(data: impl AsRef<[u8]>) -> String {
    data_encoding::HEXLOWER.encode(&sha256_digest(data))
}

fn digest_array<const N: usize>(digest: digest::Digest) -> [u8; N] {
    digest
        .as_ref()
        .try_into()
        .expect("AWS-LC digest length must match the selected fixed-size helper")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_bytes(input: &str) -> Vec<u8> {
        assert!(input.len().is_multiple_of(2));
        (0..input.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&input[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn digest_algorithms_match_known_answers() {
        let cases = [
            (
                DigestAlgorithm::Sha1,
                "a9993e364706816aba3e25717850c26c9cd0d89d",
            ),
            (
                DigestAlgorithm::Sha256,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                DigestAlgorithm::Sha384,
                concat!(
                    "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed",
                    "8086072ba1e7cc2358baeca134c825a7"
                ),
            ),
            (
                DigestAlgorithm::Sha512,
                concat!(
                    "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                    "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                ),
            ),
        ];

        for (algorithm, expected) in cases {
            let expected = hex_bytes(expected);
            assert_eq!(algorithm.output_len_bytes(), expected.len());
            assert_eq!(algorithm.digest_bytes(b"abc"), expected);
        }
    }

    #[test]
    fn sha_helpers_match_one_shot_and_incremental_known_answers() {
        assert_eq!(
            sha1_digest(b"abc").as_slice(),
            hex_bytes("a9993e364706816aba3e25717850c26c9cd0d89d")
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let mut context = Sha256Context::new();
        context.update(b"a");
        context.update(b"bc");
        assert_eq!(context.finish(), sha256_digest(b"abc"));
    }
}
