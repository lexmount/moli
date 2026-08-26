use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

fn hex_bytes(input: &str) -> Vec<u8> {
    assert!(
        input.len().is_multiple_of(2),
        "hex fixture must contain full bytes"
    );
    (0..input.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&input[index..index + 2], 16).unwrap())
        .collect()
}

fn pattern_bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|index| seed.wrapping_add((index as u8).wrapping_mul(37)))
        .collect()
}

#[test]
fn webcrypto_digest_algorithms_delegate_to_base_digest_owner() {
    let cases = [
        (
            WebCryptoHashAlgorithm::Sha1,
            "a9993e364706816aba3e25717850c26c9cd0d89d",
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            WebCryptoHashAlgorithm::Sha384,
            concat!(
                "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed",
                "8086072ba1e7cc2358baeca134c825a7"
            ),
        ),
        (
            WebCryptoHashAlgorithm::Sha512,
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
fn hmac_verify_uses_mac_verification_result() {
    let signature = hmac_signature(WebCryptoHashAlgorithm::Sha512, b"secret", b"payload")
        .expect("SHA-512 HMAC should sign");

    assert!(verify_hmac(
        WebCryptoHashAlgorithm::Sha512,
        b"secret",
        b"payload",
        &signature
    ));
    assert!(!verify_hmac(
        WebCryptoHashAlgorithm::Sha512,
        b"secret",
        b"tampered",
        &signature
    ));
    assert!(!verify_hmac(
        WebCryptoHashAlgorithm::Sha512,
        b"secret",
        b"payload",
        &signature[..16]
    ));

    let sha256 = hmac_signature(WebCryptoHashAlgorithm::Sha256, b"secret", b"payload")
        .expect("SHA-256 HMAC should sign");
    assert_eq!(sha256.len(), 32);
    assert!(verify_hmac(
        WebCryptoHashAlgorithm::Sha256,
        b"secret",
        b"payload",
        &sha256
    ));
}

#[test]
fn hmac_known_answers_match_chromium_backend_vectors() {
    // Ported from Chromium components/webcrypto/algorithms/hmac_unittest.cc.
    // These cover empty messages, long keys/messages, and every SHA family
    // Moli exposes through HMAC.
    let cases = [
        (
            WebCryptoHashAlgorithm::Sha1,
            "00",
            "",
            "fbdb1d1b18aa6c08324b7d64b71fb76370690e1d",
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            "00",
            "",
            "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            "59785928d72516e31272",
            concat!(
                "a3ce8899df1022e8d2d539b47bf0e309c66f84095e21438ec355bf119ce5fdcb4e73a619c",
                "df36f25b369d8c38ff419997f0c59830108223606e31223483fd39edeaa4d3f0d21198862",
                "d239c9fd26074130ff6c86493f5227ab895c8f244bd42c7afce5d147a20a590798c68e708",
                "e964902d124dadecdbda9dbd0051ed710e9bf"
            ),
            "3c8162589aafaee024fc9a5ca50dd2336fe3eb28",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            concat!(
                "ceb9aedf8d6efcf0ae52bea0fa99a9e26ae81bacea0cff4d5eecf201e3bca3c3577480621",
                "b818fd717ba99d6ff958ea3d59b2527b019c343bb199e648090225867d994607962f5866a",
                "a62930d75b58f6"
            ),
            concat!(
                "99958aa459604657c7bf6e4cdfcc8785f0abf06ffe636b5b64ecd931bd8a456305592421f",
                "c28dbcccb8a82acea2be8e54161d7a78e0399a6067ebaca3f2510274dc9f92f2c8ae4265e",
                "ec13d7d42e9f8612d7bc258f913ecb5a3a5c610339b49fb90e9037b02d684fc60da835657",
                "cb24eab352750c8b463b1a8494660d36c3ab2"
            ),
            "4ac41ab89f625c60125ed65ffa958c6b490ea670",
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            concat!(
                "9779d9120642797f1747025d5b22b7ac607cab08e1758f2f3a46c8be1e25c53b8c6a8f58f",
                "fefa176"
            ),
            concat!(
                "b1689c2591eaf3c9e66070f8a77954ffb81749f1b00346f9dfe0b2ee905dcc288baf4a92d",
                "e3f4001dd9f44c468c3d07d6c6ee82faceafc97c2fc0fc0601719d2dcd0aa2aec92d1b0ae",
                "933c65eb06a03c9c935c2bad0459810241347ab87e9f11adb30415424c6c7f5f22a003b8a",
                "b8de54f6ded0e3ab9245fa79568451dfa258e"
            ),
            "769f00d3e6a6cc1fb426a14a4f76c6462e6149726e0dee0ec0cf97a16605ac8b",
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            concat!(
                "4b7ab133efe99e02fc89a28409ee187d579e774f4cba6fc223e13504e3511bef8d4f638b9",
                "aca55d4a43b8fbd64cf9d74dcc8c9e8d52034898c70264ea911a3fd70813fa73b08337128",
                "9b"
            ),
            concat!(
                "138efc832c64513d11b9873c6fd4d8a65dbf367092a826ddd587d141b401580b798c69025",
                "ad510cff05fcfbceb6cf0bb03201aaa32e423d5200925bddfadd418d8e30e18050eb4f061",
                "8eb9959d9f78c1157d4b3e02cd5961f138afd57459939917d9144c95d8e6a94c8f6d4eef3",
                "418c17b1ef0b46c2a7188305d9811dccb3d99"
            ),
            "4f1ee7cb36c58803a8721d4ac8c4cf8cae5d8832392eed2a96dc59694252801b",
        ),
        (
            WebCryptoHashAlgorithm::Sha384,
            concat!(
                "d137f3e6cc4af28554beb03ba7a97e60c9d3959cd3bb08068edbf68d402d0498c6ee0ae9e",
                "3a20dc7d8586e5c352f605cee19"
            ),
            concat!(
                "64a884670d1c1dff555483dcd3da305dfba54bdc4d817c33ccb8fe7eb2ebf623624103109",
                "ec41644fa078491900c59a0f666f0356d9bc0b45bcc79e5fc9850f4543d96bc68009044ad",
                "d0838ac1260e80592fbc557b2ddaf5ed1b86d3ed8f09e622e567f1d39a340857f6a850cce",
                "ef6060c48dac3dd0071fe68eb4ed2ed9aca01"
            ),
            concat!(
                "c550fa53514da34f15e7f98ea87226ab6896cdfae25d3ec2335839f755cdc9a4992092e70",
                "b7e5bd422784380b6396cf5"
            ),
        ),
        (
            WebCryptoHashAlgorithm::Sha512,
            concat!(
                "c367aeb5c02b727883ffe2a4ceebf911b01454beb328fb5d57fc7f11bf744576aba421e2a",
                "63426ea8109bd28ff21f53cd2bf1a11c6c989623d6ec27cdb0bbf458250857d819ff84408",
                "b4f3dce08b98b1587ee59683af8852a0a5f55bda3ab5e132b4010e"
            ),
            concat!(
                "1a7331c8ff1b748e3cee96952190fdbbe4ee2f79e5753bbb368255ee5b19c05a4ed9f1b2c",
                "72ff1e9b9cb0348205087befa501e7793770faf0606e9c901836a9bc8afa00d7db94ee29e",
                "b191d5cf3fc3e8da95a0f9f4a2a7964289c3129b512bd890de8700a9205420f28a8965b6c",
                "67be28ba7fe278e5fcd16f0f22cf2b2eacbb9"
            ),
            concat!(
                "4459066109cb11e6870fa9c6bfd251adfa304c0a2928ca915049704972edc560cc7c0bc38",
                "249e9101aae2f7d4da62eaff83fb07134efc277de72b9e4ab360425"
            ),
        ),
    ];

    for (hash, key_hex, message_hex, signature_hex) in cases {
        let key = hex_bytes(key_hex);
        let message = hex_bytes(message_hex);
        let signature = hex_bytes(signature_hex);
        let actual = hmac_signature(hash, &key, &message).expect("HMAC should sign");

        assert_eq!(actual, signature);
        assert!(verify_hmac(hash, &key, &message, &signature));
        assert!(!verify_hmac(
            hash,
            &key,
            &message,
            &signature[..signature.len() - 1]
        ));
        assert!(!verify_hmac(hash, &key, &message, &[]));
        assert!(!verify_hmac(hash, &key, &message, &[0_u8; 1024]));
    }
}

#[test]
fn aes_gcm_known_answers_match_chromium_web_tests() {
    // Ported from Chromium third_party/blink/web_tests/crypto/subtle/aes-gcm.
    // The final case uses a non-96-bit IV and a 32-bit tag, which is outside
    // the parameter space supported by narrower AEAD wrappers.
    let cases = [
        (
            "cf063a34d4a9a76c2c86787d3f96db71",
            "113b9785971864c83b01c787",
            "",
            "",
            128,
            "",
            "72ac8493e3a5228b5d130a69d2510e42",
        ),
        (
            "6dfa1a07c14f978020ace450ad663d18",
            "34edfa462a14c6969a680ec1",
            "",
            "2a35c7f5f8578e919a581c60500c04f6",
            120,
            "",
            "751f3098d59cf4ea1d2fb0853bde1c",
        ),
        (
            "ed6cd876ceba555706674445c229c12d",
            "92ecbf74b765bc486383ca2e",
            "bfaaaea3880d72d4378561e2597a9b35",
            "95bd10d77dbe0e87fb34217f1a2e5efe",
            112,
            "bdd2ed6c66fa087dce617d7fd1ff6d93",
            "ba82e49c55a22ed02ca67da4ec6f",
        ),
        (
            "e03548984a7ec8eaf0870637df0ac6bc17f7159315d0ae26a764fd224e483810",
            concat!(
                "f4feb26b846be4cd224dbc5133a5ae13814ebe19d3032acdd3a006463fdb71e",
                "83a9d5d96679f26cc1719dd6b4feb3bab5b4b7993d0c0681f36d105ad300",
                "2fb66b201538e2b7479838ab83402b0d816cd6e0fe5857e6f4adf92de8",
                "ee72b122ba1ac81795024943b7d0151bbf84ce87c8911f512c397d141122",
                "96da7ecdd0da52a"
            ),
            concat!(
                "69fd0c9da10b56ec6786333f8d76d4b74f8a434195f2f241f088b2520fb5",
                "fa29455df9893164fb1638abe6617915d9497a8fe2"
            ),
            concat!(
                "aab26eb3e7acd09a034a9e2651636ab3868e51281590ecc948355e457da42",
                "b7ad1391c7be0d9e82895e506173a81857c3226829fbd6dfb3f9657a",
                "71a2934445d7c05fa9401cddd5109016ba32c3856afaadc48de80b8a01b57cb"
            ),
            32,
            concat!(
                "fda718aa1ec163487e21afc34f5a3a34795a9ee71dd3e7ee9a18fdb241",
                "81dc982b29c6ec723294a130ca2234952bb0ef68c0f3"
            ),
            "4795fbe0",
        ),
    ];

    for (key, iv, plaintext, additional_data, tag_bits, ciphertext, tag) in cases {
        let mut expected = hex_bytes(ciphertext);
        expected.extend_from_slice(&hex_bytes(tag));
        let actual = aes_gcm_encrypt(
            &hex_bytes(key),
            &hex_bytes(iv),
            &hex_bytes(additional_data),
            tag_bits,
            &hex_bytes(plaintext),
        )
        .expect("AES-GCM vector should encrypt");
        assert_eq!(actual, expected);
        let decrypted = aes_gcm_decrypt(
            &hex_bytes(key),
            &hex_bytes(iv),
            &hex_bytes(additional_data),
            tag_bits,
            &actual,
        )
        .expect("AES-GCM vector should decrypt");
        assert_eq!(decrypted, hex_bytes(plaintext));
    }
}

#[test]
fn aes_ctr_matches_nist_sp800_38a_vector_and_rejects_wraparound() {
    let key = hex_bytes("2b7e151628aed2a6abf7158809cf4f3c");
    let counter = hex_bytes("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
    let plaintext = hex_bytes(concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710"
    ));
    let ciphertext = hex_bytes(concat!(
        "874d6191b620e3261bef6864990db6ce",
        "9806f66b7970fdff8617187bb9fffdff",
        "5ae4df3edbd5d35e5b4f09020db03eab",
        "1e031dda2fbe03d1792170a0f3009cee"
    ));

    let actual =
        aes_ctr_crypt(&key, &counter, 128, &plaintext).expect("AES-CTR vector should encrypt");
    assert_eq!(actual, ciphertext);
    let decrypted =
        aes_ctr_crypt(&key, &counter, 128, &actual).expect("AES-CTR vector should decrypt");
    assert_eq!(decrypted, plaintext);

    let chunked_plaintext = (0..(16 * 1025 + 7))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let chunked_ciphertext =
        aes_ctr_crypt(&key, &counter, 128, &chunked_plaintext).expect("AES-CTR should encrypt");
    assert_ne!(chunked_ciphertext, chunked_plaintext);
    let chunked_decrypted =
        aes_ctr_crypt(&key, &counter, 128, &chunked_ciphertext).expect("AES-CTR should decrypt");
    assert_eq!(chunked_decrypted, chunked_plaintext);

    let mut almost_wrapped = [0_u8; 16];
    almost_wrapped[15] = 0xff;
    assert_eq!(
        aes_ctr_crypt(&key, &almost_wrapped, 8, &[0_u8; 17]),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn aes_cbc_round_trips_pkcs7_padded_data() {
    let key = hex_bytes("000102030405060708090a0b0c0d0e0f");
    let iv = hex_bytes("101112131415161718191a1b1c1d1e1f");
    let plaintext = b"webcrypto cbc plaintext";
    let ciphertext =
        aes_cbc_encrypt(&key, &iv, plaintext).expect("AES-CBC should encrypt with padding");
    assert_ne!(ciphertext, plaintext);
    assert!(ciphertext.len().is_multiple_of(16));
    let decrypted =
        aes_cbc_decrypt(&key, &iv, &ciphertext).expect("AES-CBC should decrypt with padding");
    assert_eq!(decrypted, plaintext);
    assert_eq!(
        aes_cbc_decrypt(&key, &iv, &[0_u8; 16]),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn aes_kw_matches_rfc3394_vector() {
    let kek = hex_bytes("000102030405060708090a0b0c0d0e0f");
    let plaintext = hex_bytes("00112233445566778899aabbccddeeff");
    let ciphertext = hex_bytes("1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5");
    let actual = aes_kw_wrap(&kek, &plaintext).expect("AES-KW should wrap");
    assert_eq!(actual, ciphertext);
    let unwrapped = aes_kw_unwrap(&kek, &ciphertext).expect("AES-KW should unwrap");
    assert_eq!(unwrapped, plaintext);
    assert_eq!(
        aes_kw_unwrap(&kek, &[0_u8; 16]),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn aes_chromium_wpt_operation_matrix_covers_supported_backend_modes() {
    // Ported as a compact backend matrix from Chromium WPT
    // WebCryptoAPI/encrypt_decrypt/aes_* and wrapKey_unwrapKey. The
    // browser-facing tests cover WebIDL ordering; this keeps the primitive
    // coverage dense across modes, key sizes, padding, counter lengths, GCM
    // tag sizes, and AES-KW payloads.
    let iv = [
        85, 170, 248, 155, 168, 148, 19, 213, 78, 167, 39, 167, 108, 39, 162, 132,
    ];
    let gcm_iv = [17, 59, 151, 133, 151, 24, 100, 200, 59, 1, 199, 135];
    let gcm_long_iv = [
        244, 254, 178, 107, 132, 107, 228, 205, 34, 77, 188, 81, 51, 165, 174, 19,
    ];
    let additional_data = b"chromium webcrypto aes aad";
    let mut cases = 0;

    for key_len in [16, 24, 32] {
        let key = pattern_bytes(key_len, key_len as u8);

        for plaintext_len in [0, 1, 15, 16, 31, 64] {
            let plaintext = pattern_bytes(plaintext_len, plaintext_len as u8);
            let ciphertext =
                aes_cbc_encrypt(&key, &iv, &plaintext).expect("AES-CBC matrix should encrypt");
            assert_ne!(ciphertext, plaintext);
            assert_eq!(
                aes_cbc_decrypt(&key, &iv, &ciphertext).expect("AES-CBC matrix should decrypt"),
                plaintext
            );
            cases += 1;
        }

        assert_eq!(
            aes_cbc_encrypt(&key, &iv[..8], b"payload"),
            Err(WebCryptoError::Operation)
        );
        assert_eq!(
            aes_cbc_decrypt(&key, &iv[..8], &[0_u8; 16]),
            Err(WebCryptoError::Operation)
        );
        let mut bad_padding =
            aes_cbc_encrypt(&key, &iv, b"bad padding case").expect("AES-CBC should encrypt");
        *bad_padding.last_mut().unwrap() ^= 1;
        assert_eq!(
            aes_cbc_decrypt(&key, &iv, &bad_padding),
            Err(WebCryptoError::Operation)
        );
        cases += 3;

        for length_bits in [1, 8, 64, 128] {
            let plaintext = pattern_bytes(if length_bits == 1 { 31 } else { 65 }, length_bits);
            let counter = [0_u8; 16];
            let ciphertext = aes_ctr_crypt(&key, &counter, length_bits, &plaintext)
                .expect("AES-CTR matrix should encrypt");
            assert_ne!(ciphertext, plaintext);
            assert_eq!(
                aes_ctr_crypt(&key, &counter, length_bits, &ciphertext)
                    .expect("AES-CTR matrix should decrypt"),
                plaintext
            );
            cases += 1;
        }
        assert_eq!(
            aes_ctr_crypt(&key, &[0_u8; 15], 64, b"payload"),
            Err(WebCryptoError::Operation)
        );
        assert_eq!(
            aes_ctr_crypt(&key, &[0_u8; 16], 0, b"payload"),
            Err(WebCryptoError::Operation)
        );
        assert_eq!(
            aes_ctr_crypt(&key, &[0_u8; 16], 129, b"payload"),
            Err(WebCryptoError::Operation)
        );
        let mut wrapping_counter = [0_u8; 16];
        wrapping_counter[15] = 0xff;
        assert_eq!(
            aes_ctr_crypt(&key, &wrapping_counter, 8, &[1_u8; 17]),
            Err(WebCryptoError::Operation)
        );
        cases += 4;

        for (iv_value, aad) in [
            (gcm_iv.as_slice(), additional_data.as_slice()),
            (gcm_long_iv.as_slice(), [].as_slice()),
        ] {
            for tag_bits in [32, 64, 96, 104, 112, 120, 128] {
                let plaintext = pattern_bytes(29, tag_bits as u8);
                let ciphertext = aes_gcm_encrypt(&key, iv_value, aad, tag_bits, &plaintext)
                    .expect("AES-GCM matrix should encrypt");
                assert_eq!(
                    aes_gcm_decrypt(&key, iv_value, aad, tag_bits, &ciphertext)
                        .expect("AES-GCM matrix should decrypt"),
                    plaintext
                );
                let mut tampered = ciphertext;
                tampered[0] ^= 1;
                assert_eq!(
                    aes_gcm_decrypt(&key, iv_value, aad, tag_bits, &tampered),
                    Err(WebCryptoError::Operation)
                );
                cases += 2;
            }
        }
        for bad_tag_bits in [24, 48, 72, 95, 129] {
            assert_eq!(
                aes_gcm_encrypt(&key, &gcm_iv, additional_data, bad_tag_bits, b"payload"),
                Err(WebCryptoError::Operation)
            );
            cases += 1;
        }

        for plaintext_len in [16, 24, 32] {
            let plaintext = pattern_bytes(plaintext_len, plaintext_len as u8);
            let wrapped = aes_kw_wrap(&key, &plaintext).expect("AES-KW matrix should wrap");
            assert_eq!(
                aes_kw_unwrap(&key, &wrapped).expect("AES-KW matrix should unwrap"),
                plaintext
            );
            let mut tampered = wrapped;
            tampered[0] ^= 1;
            assert_eq!(
                aes_kw_unwrap(&key, &tampered),
                Err(WebCryptoError::Operation)
            );
            cases += 2;
        }
    }

    assert!(cases >= 10, "AES matrix should keep at least 10 cases");
}

#[test]
fn aes_rejects_product_resource_limits_before_large_cipher_work() {
    let key = [0x11_u8; 16];
    let iv = [0x22_u8; 16];
    let too_large = vec![0_u8; MAX_AES_OPERATION_BYTES + 1];

    assert_eq!(
        aes_cbc_encrypt(&key, &iv, &too_large),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        aes_ctr_crypt(&key, &iv, 128, &too_large),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        aes_gcm_encrypt(&key, &iv, &[], 128, &too_large),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        aes_gcm_encrypt(&key, &iv, &too_large, 128, b"payload"),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        aes_kw_wrap(&key, &too_large),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        aes_kw_unwrap(&key, &too_large),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn chacha20_poly1305_matches_rfc_and_wpt_boundaries() {
    // The positive vector is the RFC 8439 ChaCha20-Poly1305 example also used
    // by OpenSSL's EVP tests. The failure cases mirror the modern WebCrypto
    // WPT contract: 256-bit keys, 96-bit IVs, 128-bit tags, and authenticated
    // AAD/ciphertext failures reported as OperationError.
    let key = hex_bytes("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let iv = hex_bytes("070000004041424344454647");
    let aad = hex_bytes("50515253c0c1c2c3c4c5c6c7");
    let plaintext = hex_bytes(concat!(
        "4c616469657320616e642047656e746c656d656e206f662074686520636c617373206f66202739393",
        "a204966204920636f756c64206f6666657220796f75206f6e6c79206f6e652074697020666f722074",
        "6865206675747572652c2073756e73637265656e20776f756c642062652069742e"
    ));
    let mut expected = hex_bytes(concat!(
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca967128",
        "2fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fa",
        "b324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116"
    ));
    expected.extend_from_slice(&hex_bytes("1ae10b594f09e26a7e902ecbd0600691"));

    assert_eq!(
        chacha20_poly1305_encrypt(&key, &iv, &aad, 128, &plaintext),
        Ok(expected.clone())
    );
    assert_eq!(
        chacha20_poly1305_decrypt(&key, &iv, &aad, 128, &expected),
        Ok(plaintext)
    );

    let mut tampered = expected.clone();
    tampered[0] ^= 1;
    assert_eq!(
        chacha20_poly1305_decrypt(&key, &iv, &aad, 128, &tampered),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        chacha20_poly1305_decrypt(&key, &iv, b"wrong aad", 128, &expected),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        chacha20_poly1305_encrypt(&key[..31], &iv, &aad, 128, b"payload"),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        chacha20_poly1305_encrypt(&key, &iv[..11], &aad, 128, b"payload"),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        chacha20_poly1305_encrypt(&key, &iv, &aad, 120, b"payload"),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        chacha20_poly1305_decrypt(&key, &iv, &aad, 128, &expected[..15]),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn chacha20_poly1305_jwk_and_resource_limits_are_bounded() {
    let raw = pattern_bytes(CHACHA20_POLY1305_KEY_LENGTH_BITS / 8, 0x51);
    let jwk = Chacha20Poly1305JsonWebKeyImport {
        kty: Some("oct".to_owned()),
        k: Some(URL_SAFE_NO_PAD.encode(&raw)),
        alg: None,
        key_ops: Some(vec!["encrypt".to_owned(), "decrypt".to_owned()]),
        ext: Some(true),
        public_key_use: Some("enc".to_owned()),
    };
    assert_eq!(
        import_chacha20_poly1305_jwk_key(&jwk, true, &["encrypt".to_owned(), "decrypt".to_owned()]),
        Ok(raw.clone())
    );
    let exported =
        export_chacha20_poly1305_jwk(&raw, vec!["encrypt".to_owned(), "decrypt".to_owned()], true)
            .expect("ChaCha20-Poly1305 JWK export should succeed");
    assert_eq!(exported.kty, "oct");
    assert_eq!(exported.k, URL_SAFE_NO_PAD.encode(&raw));
    assert_eq!(exported.key_ops, ["encrypt", "decrypt"]);
    assert!(exported.ext);

    let alg_jwk = Chacha20Poly1305JsonWebKeyImport {
        alg: Some("A256GCM".to_owned()),
        ..jwk
    };
    assert_eq!(
        import_chacha20_poly1305_jwk_key(&alg_jwk, true, &["encrypt".to_owned()]),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        validate_chacha20_poly1305_key_bytes(&raw[..31]),
        Err(WebCryptoError::Data)
    );
    let too_large = vec![0_u8; MAX_CHACHA20_POLY1305_OPERATION_BYTES + 1];
    assert_eq!(
        chacha20_poly1305_encrypt(&raw, &[0_u8; 12], &[], 128, &too_large),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        chacha20_poly1305_encrypt(&raw, &[0_u8; 12], &too_large, 128, b"payload"),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn hash_algorithm_names_and_hmac_key_lengths_are_derived() {
    assert_eq!(
        "sha-1".parse::<WebCryptoHashAlgorithm>().unwrap().as_ref(),
        "sha-1"
    );
    assert_eq!(
        "sha-256"
            .parse::<WebCryptoHashAlgorithm>()
            .unwrap()
            .default_hmac_key_len_bytes(),
        64
    );
    assert_eq!(
        "sha-384"
            .parse::<WebCryptoHashAlgorithm>()
            .unwrap()
            .default_hmac_key_len_bytes(),
        128
    );
    assert_eq!(
        "sha-512"
            .parse::<WebCryptoHashAlgorithm>()
            .unwrap()
            .default_hmac_key_len_bytes(),
        128
    );
    assert!("sha512".parse::<WebCryptoHashAlgorithm>().is_err());
}

#[test]
fn hmac_key_generation_caps_lengths_and_truncates_partial_bytes() {
    assert_eq!(
        generate_hmac_key(WebCryptoHashAlgorithm::Sha1, Some(0)),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        generate_hmac_key(
            WebCryptoHashAlgorithm::Sha1,
            Some(MAX_HMAC_KEY_LENGTH_BITS + 1)
        ),
        Err(WebCryptoError::Operation)
    );

    // Chromium backend: components/webcrypto/algorithms/hmac_unittest.cc,
    // Generating1BitKeyWorks. Generated key material keeps only the
    // requested high bits in the final byte.
    let key = generate_hmac_key(WebCryptoHashAlgorithm::Sha1, Some(1))
        .expect("1-bit HMAC key generation should work");
    assert_eq!(key.len(), 1);
    assert_eq!(key[0] & 0x7f, 0);
}

#[test]
fn key_algorithm_names_are_derived() {
    assert_eq!(
        "aes-cbc".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::AesCbc)
    );
    assert_eq!(
        "aes-gcm".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::AesGcm)
    );
    assert_eq!(
        "hkdf".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::Hkdf)
    );
    assert_eq!(
        "hmac".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::Hmac)
    );
    assert_eq!(
        "pbkdf2".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::Pbkdf2)
    );
    assert_eq!(
        "x25519".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::X25519)
    );
    assert_eq!(
        "rsa-oaep".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::RsaOaep)
    );
    assert_eq!(
        "rsassa-pkcs1-v1_5".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::RsassaPkcs1V15)
    );
    assert_eq!(
        "rsa-pss".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::RsaPss)
    );
    assert_eq!(
        "ecdh".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::Ecdh)
    );
    assert_eq!(
        "ecdsa".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::Ecdsa)
    );
    assert_eq!(
        "ed25519".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::Ed25519)
    );
    assert_eq!(
        "x448".parse::<WebCryptoKeyAlgorithm>(),
        Ok(WebCryptoKeyAlgorithm::X448)
    );
}

#[test]
fn hmac_jwk_import_rejects_mismatched_alg() {
    let jwk = HmacJsonWebKeyImport {
        kty: Some("oct".to_owned()),
        k: Some(URL_SAFE_NO_PAD.encode(b"secret")),
        alg: Some("HS256".to_owned()),
        key_ops: Some(vec!["sign".to_owned()]),
        ext: Some(true),
        public_key_use: Some("sig".to_owned()),
    };

    assert_eq!(
        import_hmac_jwk_key(
            &jwk,
            WebCryptoHashAlgorithm::Sha512,
            true,
            &["sign".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        import_hmac_jwk_key(
            &jwk,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["verify".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        import_hmac_jwk_key(
            &jwk,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["sign".to_owned()]
        )
        .expect("matching JWK should import"),
        b"secret"
    );

    let wrong_use = HmacJsonWebKeyImport {
        public_key_use: Some("enc".to_owned()),
        ..jwk
    };
    assert_eq!(
        import_hmac_jwk_key(
            &wrong_use,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["sign".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let duplicate_key_ops = HmacJsonWebKeyImport {
        public_key_use: Some("sig".to_owned()),
        key_ops: Some(vec!["sign".to_owned(), "sign".to_owned()]),
        ..wrong_use
    };
    assert_eq!(
        import_hmac_jwk_key(
            &duplicate_key_ops,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["sign".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let inconsistent_use_and_key_ops = HmacJsonWebKeyImport {
        public_key_use: Some("sig".to_owned()),
        key_ops: Some(vec![
            "sign".to_owned(),
            "verify".to_owned(),
            "encrypt".to_owned(),
        ]),
        ..duplicate_key_ops
    };
    assert_eq!(
        import_hmac_jwk_key(
            &inconsistent_use_and_key_ops,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["sign".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );
}

#[test]
fn hmac_import_key_length_matches_chromium_backend_rules() {
    assert_eq!(
        validate_hmac_import_key_bytes(Vec::new(), None),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        validate_hmac_import_key_bytes(vec![0_u8; 15], Some(128)),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        validate_hmac_import_key_bytes(vec![0_u8; 16], Some(120)),
        Err(WebCryptoError::Data)
    );

    let (bytes, length_bits) = validate_hmac_import_key_bytes(vec![0xb1, 0xff], Some(12))
        .expect("12-bit HMAC raw key should import");
    assert_eq!(length_bits, 12);
    assert_eq!(bytes, [0xb1, 0xf0]);

    // Chromium caps caller-allocated HMAC generation and derived-key
    // lengths separately from import. Import consumes already-provided key
    // bytes, so backend validation only requires the requested bit length
    // to round to the supplied byte length.
    let (bytes, length_bits) = validate_hmac_import_key_bytes(vec![0xff; 8193], Some(65_537))
        .expect("HMAC import should accept supplied bytes beyond generation cap");
    assert_eq!(length_bits, 65_537);
    assert_eq!(bytes.len(), 8193);
    assert_eq!(bytes[8192], 0x80);
}

#[test]
fn aes_jwk_import_export_validates_algorithm_and_length() {
    let raw = [1_u8; 16];
    let unsupported_192 = [2_u8; 24];
    let jwk = AesJsonWebKeyImport {
        kty: Some("oct".to_owned()),
        k: Some(URL_SAFE_NO_PAD.encode(raw)),
        alg: Some("A128GCM".to_owned()),
        key_ops: Some(vec!["encrypt".to_owned()]),
        ext: Some(true),
        public_key_use: Some("enc".to_owned()),
    };

    assert_eq!(
        import_aes_jwk_key(
            &jwk,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        )
        .expect("matching AES JWK should import"),
        raw
    );
    assert_eq!(
        import_aes_jwk_key(
            &jwk,
            WebCryptoKeyAlgorithm::AesCbc,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        export_aes_jwk(
            WebCryptoKeyAlgorithm::AesGcm,
            &raw,
            vec!["encrypt".to_owned()],
            true
        )
        .expect("AES JWK should export")
        .alg,
        "A128GCM"
    );
    assert_eq!(generate_aes_key(192).map(|bytes| bytes.len()), Ok(24));
    assert_eq!(validate_aes_key_bytes(&unsupported_192), Ok(192));

    let jwk_192 = AesJsonWebKeyImport {
        kty: Some("oct".to_owned()),
        k: Some(URL_SAFE_NO_PAD.encode(unsupported_192)),
        alg: Some("A192GCM".to_owned()),
        key_ops: Some(vec!["encrypt".to_owned()]),
        ext: Some(true),
        public_key_use: Some("enc".to_owned()),
    };
    assert_eq!(
        import_aes_jwk_key(
            &jwk_192,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        ),
        Ok(unsupported_192.to_vec())
    );
    assert_eq!(
        export_aes_jwk(
            WebCryptoKeyAlgorithm::AesGcm,
            &unsupported_192,
            vec!["encrypt".to_owned()],
            true
        )
        .expect("AES-192 JWK should export")
        .alg,
        "A192GCM"
    );

    let jwk_192_wrong_alg = AesJsonWebKeyImport {
        kty: Some("oct".to_owned()),
        k: Some(URL_SAFE_NO_PAD.encode(unsupported_192)),
        alg: Some("A128GCM".to_owned()),
        key_ops: Some(vec!["encrypt".to_owned()]),
        ext: Some(true),
        public_key_use: Some("enc".to_owned()),
    };
    assert_eq!(
        import_aes_jwk_key(
            &jwk_192_wrong_alg,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let wrong_use = AesJsonWebKeyImport {
        public_key_use: Some("sig".to_owned()),
        ..jwk
    };
    assert_eq!(
        import_aes_jwk_key(
            &wrong_use,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let duplicate_key_ops = AesJsonWebKeyImport {
        public_key_use: Some("enc".to_owned()),
        key_ops: Some(vec!["encrypt".to_owned(), "encrypt".to_owned()]),
        ..wrong_use
    };
    assert_eq!(
        import_aes_jwk_key(
            &duplicate_key_ops,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let inconsistent_use_and_key_ops = AesJsonWebKeyImport {
        public_key_use: Some("enc".to_owned()),
        key_ops: Some(vec!["encrypt".to_owned(), "sign".to_owned()]),
        ..duplicate_key_ops
    };
    assert_eq!(
        import_aes_jwk_key(
            &inconsistent_use_and_key_ops,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let unknown_extra_key_ops = AesJsonWebKeyImport {
        public_key_use: None,
        key_ops: Some(vec![
            "unknown".to_owned(),
            "alsoUnknown".to_owned(),
            "encrypt".to_owned(),
        ]),
        ..inconsistent_use_and_key_ops
    };
    assert_eq!(
        import_aes_jwk_key(
            &unknown_extra_key_ops,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        )
        .expect("unknown distinct JWK key_ops should be ignored like Chromium"),
        raw
    );

    let duplicate_unknown_key_ops = AesJsonWebKeyImport {
        kty: Some("oct".to_owned()),
        k: Some(URL_SAFE_NO_PAD.encode(raw)),
        alg: Some("A128GCM".to_owned()),
        key_ops: Some(vec![
            "unknown".to_owned(),
            "unknown".to_owned(),
            "encrypt".to_owned(),
        ]),
        ext: Some(true),
        public_key_use: None,
    };
    assert_eq!(
        import_aes_jwk_key(
            &duplicate_unknown_key_ops,
            WebCryptoKeyAlgorithm::AesGcm,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );
}

#[test]
fn hkdf_matches_chromium_legacy_rfc5869_vectors() {
    // Ported from Chromium crypto/subtle/hkdf/deriveBits-rfc5869-test-vectors.html.
    let cases = [
        (
            WebCryptoHashAlgorithm::Sha256,
            "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
            "000102030405060708090a0b0c",
            "f0f1f2f3f4f5f6f7f8f9",
            42,
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            concat!(
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                "404142434445464748494a4b4c4d4e4f"
            ),
            concat!(
                "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
                "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
                "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf"
            ),
            concat!(
                "b0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9",
                "cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3",
                "e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"
            ),
            82,
            concat!(
                "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c",
                "59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71",
                "cc30c58179ec3e87c14c01d5c1f3434f1d87"
            ),
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
            "",
            "",
            42,
            "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            "0b0b0b0b0b0b0b0b0b0b0b",
            "000102030405060708090a0b0c",
            "f0f1f2f3f4f5f6f7f8f9",
            42,
            "085a01ea1b10f36933068b56efa5ad81a4f14b822f5b091568a9cdd4f155fda2c22e422478d305f3f896",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            concat!(
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                "404142434445464748494a4b4c4d4e4f"
            ),
            concat!(
                "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
                "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
                "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf"
            ),
            concat!(
                "b0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9",
                "cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3",
                "e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"
            ),
            82,
            concat!(
                "0bd770a74d1160f7c9f12cd5912a06ebff6adcae899d92191fe4305673ba2ff",
                "e8fa3f1a4e5ad79f3f334b3b202b2173c486ea37ce3d397ed034c7f9dfeb",
                "15c5e927336d0441f4c4300e2cff0d0900b52d3b4"
            ),
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
            "",
            "",
            42,
            "0ac1af7002b3d761d1e55298da9d0506b9ae52057220a306e07b6b87e8df21d0ea00033de03984d34918",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            "0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c",
            "",
            "",
            42,
            "2c91117204d745f3500d636a62f64f0ab3bae548aa53d423b0d1f27ebba6f5e5673a081d70cce7acfc48",
        ),
    ];

    for (hash, input, salt, info, length_bytes, output) in cases {
        assert_eq!(
            derive_hkdf_bits(
                hash,
                &hex_bytes(input),
                &hex_bytes(salt),
                &hex_bytes(info),
                length_bytes * 8,
            )
            .expect("HKDF vector should derive"),
            hex_bytes(output)
        );
    }
}

#[test]
fn pbkdf2_matches_chromium_legacy_rfc6070_vectors() {
    // Ported from Chromium crypto/subtle/pbkdf2/deriveBits-rfc6070-test-vectors.html.
    let cases = [
        (
            WebCryptoHashAlgorithm::Sha1,
            b"password".as_slice(),
            b"salt".as_slice(),
            1,
            20,
            "0c60c80f961f0e71f3a9b524af6012062fe037a6",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            b"password".as_slice(),
            b"salt".as_slice(),
            2,
            20,
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            b"password".as_slice(),
            b"salt".as_slice(),
            4096,
            20,
            "4b007901b765489abead49d926f721d065a429c1",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            b"passwordPASSWORDpassword".as_slice(),
            b"saltSALTsaltSALTsaltSALTsaltSALTsalt".as_slice(),
            4096,
            25,
            "3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038",
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            b"pass\0word".as_slice(),
            b"sa\0lt".as_slice(),
            4096,
            16,
            "56fa6aa75548099dcc37d7f03425e0c3",
        ),
    ];

    for (hash, password, salt, iterations, length_bytes, output) in cases {
        assert_eq!(
            derive_pbkdf2_bits(hash, password, salt, iterations, length_bytes * 8)
                .expect("PBKDF2 vector should derive"),
            hex_bytes(output)
        );
    }
}

#[test]
fn kdf_bits_match_chromium_wpt_vectors() {
    let hkdf = derive_hkdf_bits(
        WebCryptoHashAlgorithm::Sha256,
        &[80, 64, 115, 115, 119, 48, 114, 100],
        &[
            83, 111, 100, 105, 117, 109, 32, 67, 104, 108, 111, 114, 105, 100, 101, 32, 99, 111,
            109, 112, 111, 117, 110, 100,
        ],
        &[
            72, 75, 68, 70, 32, 101, 120, 116, 114, 97, 32, 105, 110, 102, 111,
        ],
        256,
    )
    .expect("HKDF vector should derive");
    assert_eq!(
        hkdf,
        [
            42, 245, 144, 30, 40, 132, 156, 40, 68, 56, 87, 56, 106, 161, 172, 59, 177, 39, 233,
            38, 49, 193, 192, 81, 72, 45, 102, 144, 148, 23, 114, 180,
        ]
    );

    let pbkdf2 = derive_pbkdf2_bits(
        WebCryptoHashAlgorithm::Sha256,
        &[80, 64, 115, 115, 119, 48, 114, 100],
        &[78, 97, 67, 108],
        1,
        256,
    )
    .expect("PBKDF2 vector should derive");
    assert_eq!(
        pbkdf2,
        [
            198, 188, 85, 164, 4, 173, 206, 163, 106, 26, 181, 103, 152, 8, 94, 10, 175, 105, 127,
            107, 178, 193, 106, 80, 114, 248, 56, 241, 125, 254, 108, 182,
        ]
    );

    // Ported from Chromium WPT
    // WebCryptoAPI/derive_bits_keys/derived_bits_length_vectors.js.
    // This vector intentionally uses 100000 PBKDF2 iterations, so keep it
    // in the primitive crate where it is not subject to browser VM
    // microtask checkpoint limits.
    let pbkdf2_384 = derive_pbkdf2_bits(
        WebCryptoHashAlgorithm::Sha256,
        &[
            85, 115, 101, 114, 115, 32, 115, 104, 111, 117, 108, 100, 32, 112, 105, 99, 107, 32,
            108, 111, 110, 103, 32, 112, 97, 115, 115, 112, 104, 114, 97, 115, 101, 115, 32, 40,
            110, 111, 116, 32, 117, 115, 101, 32, 115, 104, 111, 114, 116, 32, 112, 97, 115, 115,
            119, 111, 114, 100, 115, 41, 33,
        ],
        &[
            83, 111, 100, 105, 117, 109, 32, 67, 104, 108, 111, 114, 105, 100, 101, 32, 99, 111,
            109, 112, 111, 117, 110, 100,
        ],
        100000,
        384,
    )
    .expect("PBKDF2 384-bit vector should derive");
    assert_eq!(
        pbkdf2_384,
        [
            17, 153, 45, 139, 129, 51, 17, 36, 76, 84, 75, 98, 41, 41, 69, 226, 8, 212, 3, 206,
            189, 107, 149, 82, 161, 165, 98, 6, 93, 153, 88, 234, 39, 104, 8, 112, 222, 57, 166,
            47, 102, 146, 195, 59, 219, 239, 238, 47,
        ]
    );

    assert_eq!(
        derive_hkdf_bits(WebCryptoHashAlgorithm::Sha256, b"key", b"salt", b"info", 44),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        derive_pbkdf2_bits(WebCryptoHashAlgorithm::Sha256, b"key", b"salt", 0, 256),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn kdf_rejects_product_resource_limits_before_large_work() {
    assert_eq!(
        derive_hkdf_bits(
            WebCryptoHashAlgorithm::Sha256,
            b"base",
            b"salt",
            b"info",
            MAX_KDF_DERIVED_BITS + 8
        ),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        derive_pbkdf2_bits(
            WebCryptoHashAlgorithm::Sha256,
            b"password",
            b"salt",
            MAX_PBKDF2_ITERATIONS + 1,
            128
        ),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        derive_pbkdf2_bits(
            WebCryptoHashAlgorithm::Sha256,
            b"password",
            b"salt",
            1,
            MAX_KDF_DERIVED_BITS + 8
        ),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn key_import_rejects_product_resource_limits_before_large_parse_work() {
    let oversized_der = vec![0_u8; MAX_DER_KEY_BYTES + 1];
    assert_eq!(
        import_rsa_spki_public_key(&oversized_der),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        import_ec_spki_public_key(&oversized_der, WebCryptoEcNamedCurve::P256),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        import_okp_spki_public_key(&oversized_der, WebCryptoOkpCurve::Ed25519),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        import_x25519_spki_public_key(&oversized_der),
        Err(WebCryptoError::Operation)
    );

    let oversized_member = "A".repeat(MAX_JWK_MEMBER_BYTES + 1);
    let oversized_key_ops = vec!["sign".to_owned(); MAX_JWK_KEY_OPS + 1];
    assert_eq!(
        import_hmac_jwk_key(
            &HmacJsonWebKeyImport {
                kty: Some("oct".to_owned()),
                k: Some(oversized_member.clone()),
                alg: None,
                key_ops: None,
                ext: None,
                public_key_use: None,
            },
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["sign".to_owned()]
        ),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        import_rsa_jwk_key(
            &RsaJsonWebKeyImport {
                kty: Some("RSA".to_owned()),
                n: Some(oversized_member),
                e: Some(URL_SAFE_NO_PAD.encode([1, 0, 1])),
                d: None,
                p: None,
                q: None,
                dp: None,
                dq: None,
                qi: None,
                alg: None,
                key_ops: Some(oversized_key_ops),
                ext: None,
                public_key_use: None,
            },
            WebCryptoKeyAlgorithm::RsaOaep,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Operation)
    );

    let oversized_modulus = {
        let mut bytes = vec![0_u8; MAX_RSA_MODULUS_LENGTH_BITS / 8 + 1];
        bytes[0] = 0x80;
        URL_SAFE_NO_PAD.encode(bytes)
    };
    assert_eq!(
        import_rsa_jwk_key(
            &RsaJsonWebKeyImport {
                kty: Some("RSA".to_owned()),
                n: Some(oversized_modulus),
                e: Some(URL_SAFE_NO_PAD.encode([1, 0, 1])),
                d: None,
                p: None,
                q: None,
                dp: None,
                dq: None,
                qi: None,
                alg: None,
                key_ops: None,
                ext: None,
                public_key_use: None,
            },
            WebCryptoKeyAlgorithm::RsaOaep,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    assert_eq!(
        import_rsa_jwk_key(
            &RsaJsonWebKeyImport {
                kty: Some("RSA".to_owned()),
                n: Some(URL_SAFE_NO_PAD.encode([0x80; 128])),
                e: Some(URL_SAFE_NO_PAD.encode([1_u8; MAX_RSA_PUBLIC_EXPONENT_BYTES + 1])),
                d: None,
                p: None,
                q: None,
                dp: None,
                dq: None,
                qi: None,
                alg: None,
                key_ops: None,
                ext: None,
                public_key_use: None,
            },
            WebCryptoKeyAlgorithm::RsaOaep,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["encrypt".to_owned()]
        ),
        Err(WebCryptoError::Data)
    );

    let oversized_private_component = {
        let mut bytes = vec![0_u8; MAX_RSA_MODULUS_LENGTH_BITS / 8 + 1];
        bytes[0] = 0x80;
        URL_SAFE_NO_PAD.encode(bytes)
    };
    let normal_private_component = URL_SAFE_NO_PAD.encode([1]);
    for component in ["d", "p", "q", "dp", "dq", "qi"] {
        let mut jwk = RsaJsonWebKeyImport {
            kty: Some("RSA".to_owned()),
            n: Some(URL_SAFE_NO_PAD.encode([0x80; 128])),
            e: Some(URL_SAFE_NO_PAD.encode([1, 0, 1])),
            d: Some(normal_private_component.clone()),
            p: Some(normal_private_component.clone()),
            q: Some(normal_private_component.clone()),
            dp: Some(normal_private_component.clone()),
            dq: Some(normal_private_component.clone()),
            qi: Some(normal_private_component.clone()),
            alg: None,
            key_ops: None,
            ext: None,
            public_key_use: None,
        };
        match component {
            "d" => jwk.d = Some(oversized_private_component.clone()),
            "p" => jwk.p = Some(oversized_private_component.clone()),
            "q" => jwk.q = Some(oversized_private_component.clone()),
            "dp" => jwk.dp = Some(oversized_private_component.clone()),
            "dq" => jwk.dq = Some(oversized_private_component.clone()),
            "qi" => jwk.qi = Some(oversized_private_component.clone()),
            _ => unreachable!("listed RSA private component"),
        }
        assert_eq!(
            import_rsa_jwk_key(
                &jwk,
                WebCryptoKeyAlgorithm::RsaOaep,
                WebCryptoHashAlgorithm::Sha256,
                true,
                &["decrypt".to_owned()]
            ),
            Err(WebCryptoError::Data),
            "oversized RSA JWK {component} should reject as invalid key material"
        );
    }
}

#[test]
fn rsa_import_rejects_decoded_key_material_bounds_as_data_error() {
    let exponent =
        openssl::bn::BigNum::from_u32(65537).expect("test exponent should be constructible");
    let rsa =
        openssl::rsa::Rsa::generate_with_e(512, &exponent).expect("test RSA key should generate");
    let key = openssl::pkey::PKey::from_rsa(rsa).expect("test RSA key should wrap");
    let spki = key
        .public_key_to_der()
        .expect("test RSA public key should encode as SPKI");
    let pkcs8 = key
        .private_key_to_pkcs8()
        .expect("test RSA private key should encode as PKCS8");

    assert_eq!(
        import_rsa_spki_public_key(&spki).map(|_| ()),
        Err(WebCryptoError::Data)
    );
    assert_eq!(
        import_rsa_pkcs8_private_key(&pkcs8).map(|_| ()),
        Err(WebCryptoError::Data)
    );
}

#[test]
fn rsa_rejects_product_key_generation_resource_limits_before_openssl() {
    assert_eq!(
        generate_rsa_key_pair(MAX_RSA_MODULUS_LENGTH_BITS + 1, &[1, 0, 1]).map(|_| ()),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        generate_rsa_key_pair(2048, &[1_u8; MAX_RSA_PUBLIC_EXPONENT_BYTES + 1]).map(|_| ()),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn non_container_inputs_reject_product_resource_limits_before_large_work() {
    let too_large_raw = vec![0_u8; MAX_RAW_KEY_IMPORT_BYTES + 1];
    assert_eq!(
        validate_hmac_import_key_bytes(too_large_raw.clone(), None),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        validate_aes_key_bytes(&too_large_raw),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        derive_hkdf_bits(
            WebCryptoHashAlgorithm::Sha256,
            &too_large_raw,
            b"salt",
            b"info",
            128
        ),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        derive_pbkdf2_bits(
            WebCryptoHashAlgorithm::Sha256,
            b"password",
            &too_large_raw,
            1,
            128
        ),
        Err(WebCryptoError::Operation)
    );

    let too_large_digest = vec![0_u8; MAX_DIGEST_OPERATION_BYTES + 1];
    assert_eq!(
        WebCryptoHashAlgorithm::Sha256.digest_with_limit(&too_large_digest),
        Err(WebCryptoError::Operation)
    );

    let too_large_operation = vec![0_u8; MAX_SIGNATURE_OPERATION_BYTES + 1];
    assert!(hmac_signature(WebCryptoHashAlgorithm::Sha256, b"key", &too_large_operation).is_none());
    assert!(!verify_hmac(
        WebCryptoHashAlgorithm::Sha256,
        b"key",
        b"data",
        &too_large_operation
    ));
    assert_eq!(
        rsa_pkcs1_sign(
            b"not der",
            WebCryptoHashAlgorithm::Sha256,
            &too_large_operation
        ),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        ecdsa_sign(
            b"not der",
            WebCryptoEcNamedCurve::P256,
            WebCryptoHashAlgorithm::Sha256,
            &too_large_operation
        ),
        Err(WebCryptoError::Operation)
    );
    assert_eq!(
        eddsa_sign(
            WebCryptoOkpCurve::Ed25519,
            b"not a valid key",
            &too_large_operation
        ),
        Err(WebCryptoError::Operation)
    );

    let too_large_label = vec![0_u8; MAX_RSA_OAEP_LABEL_BYTES + 1];
    assert_eq!(
        rsa_oaep_encrypt(
            b"not der",
            WebCryptoHashAlgorithm::Sha256,
            &too_large_label,
            b""
        ),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn kdf_chromium_wpt_matrix_covers_supported_hashes_inputs_and_iterations() {
    // Ported from Chromium WPT
    // WebCryptoAPI/derive_bits_keys/{hkdf,pbkdf2}_vectors.js. Keep a compact
    // 10+ case floor for each KDF so every supported SHA family, empty
    // salt/info, and low/high iteration boundary remains exercised locally.
    let hkdf_key = [80, 64, 115, 115, 119, 48, 114, 100];
    let hkdf_salt = [
        83, 111, 100, 105, 117, 109, 32, 67, 104, 108, 111, 114, 105, 100, 101, 32, 99, 111, 109,
        112, 111, 117, 110, 100,
    ];
    let hkdf_info = [
        72, 75, 68, 70, 32, 101, 120, 116, 114, 97, 32, 105, 110, 102, 111,
    ];
    let hkdf_cases: &[(WebCryptoHashAlgorithm, &[u8], &[u8], &[u8], &[u8])] = &[
        (
            WebCryptoHashAlgorithm::Sha384,
            &hkdf_key,
            &hkdf_salt,
            &hkdf_info,
            &[
                25, 186, 116, 54, 142, 107, 153, 51, 144, 242, 127, 233, 167, 208, 43, 195, 56, 23,
                63, 114, 190, 113, 161, 159, 199, 68, 252, 219, 63, 212, 184, 75,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha384,
            &hkdf_key,
            &hkdf_salt,
            &[],
            &[
                151, 96, 31, 78, 12, 83, 165, 211, 243, 162, 129, 0, 153, 188, 104, 32, 236, 80, 8,
                52, 52, 118, 155, 89, 252, 36, 164, 23, 169, 84, 55, 52,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha512,
            &hkdf_key,
            &hkdf_salt,
            &hkdf_info,
            &[
                75, 189, 109, 178, 67, 95, 182, 150, 21, 127, 96, 137, 201, 119, 195, 199, 63, 62,
                172, 94, 243, 221, 107, 170, 230, 4, 203, 83, 191, 187, 21, 62,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha512,
            &hkdf_key,
            &hkdf_salt,
            &[],
            &[
                47, 49, 87, 231, 254, 12, 16, 176, 18, 152, 200, 240, 136, 106, 144, 237, 207, 128,
                171, 222, 245, 219, 193, 223, 43, 20, 130, 83, 43, 82, 185, 52,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            &hkdf_key,
            &hkdf_salt,
            &hkdf_info,
            &[
                5, 173, 34, 237, 33, 56, 201, 96, 14, 77, 158, 39, 37, 222, 211, 1, 245, 210, 135,
                251, 251, 87, 2, 249, 153, 188, 101, 54, 211, 237, 239, 152,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            &hkdf_key,
            &hkdf_salt,
            &[],
            &[
                213, 27, 111, 183, 229, 153, 202, 48, 197, 238, 38, 69, 147, 228, 184, 95, 34, 32,
                199, 195, 171, 0, 49, 87, 191, 248, 203, 79, 54, 156, 117, 96,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &hkdf_key,
            &hkdf_salt,
            &hkdf_info,
            &[
                42, 245, 144, 30, 40, 132, 156, 40, 68, 56, 87, 56, 106, 161, 172, 59, 177, 39,
                233, 38, 49, 193, 192, 81, 72, 45, 102, 144, 148, 23, 114, 180,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &hkdf_key,
            &hkdf_salt,
            &[],
            &[
                158, 75, 113, 144, 51, 116, 33, 1, 233, 15, 26, 214, 30, 47, 243, 180, 37, 104, 99,
                102, 114, 150, 215, 67, 137, 241, 240, 42, 242, 196, 230, 166,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &hkdf_key,
            &[],
            &hkdf_info,
            &[
                115, 60, 139, 107, 207, 172, 135, 92, 127, 8, 152, 42, 110, 63, 251, 86, 10, 206,
                166, 241, 101, 71, 110, 184, 52, 96, 185, 53, 62, 212, 29, 254,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            &hkdf_key,
            &[],
            &hkdf_info,
            &[
                193, 38, 241, 230, 242, 90, 157, 228, 44, 247, 212, 39, 5, 154, 82, 237, 150, 1,
                242, 154, 88, 21, 203, 251, 198, 75, 199, 246, 104, 198, 163, 65,
            ],
        ),
    ];
    for (hash, key, salt, info, expected) in hkdf_cases {
        assert_eq!(
            derive_hkdf_bits(*hash, key, salt, info, 256)
                .expect("HKDF WPT matrix case should derive"),
            *expected
        );
    }

    let password_short = [80, 64, 115, 115, 119, 48, 114, 100];
    let password_long = [
        85, 115, 101, 114, 115, 32, 115, 104, 111, 117, 108, 100, 32, 112, 105, 99, 107, 32, 108,
        111, 110, 103, 32, 112, 97, 115, 115, 112, 104, 114, 97, 115, 101, 115, 32, 40, 110, 111,
        116, 32, 117, 115, 101, 32, 115, 104, 111, 114, 116, 32, 112, 97, 115, 115, 119, 111, 114,
        100, 115, 41, 33,
    ];
    let salt_short = [78, 97, 67, 108];
    let salt_long = hkdf_salt;
    let pbkdf2_cases: &[(WebCryptoHashAlgorithm, &[u8], &[u8], u32, &[u8])] = &[
        (
            WebCryptoHashAlgorithm::Sha384,
            &password_short,
            &salt_short,
            1,
            &[
                128, 205, 15, 21, 54, 67, 102, 167, 37, 81, 195, 121, 117, 247, 182, 55, 186, 137,
                194, 155, 70, 57, 236, 114, 15, 105, 167, 13, 187, 237, 81, 92,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha384,
            &password_short,
            &salt_short,
            1000,
            &[
                170, 236, 90, 151, 109, 77, 53, 203, 32, 36, 72, 111, 201, 249, 187, 154, 163, 234,
                231, 206, 242, 188, 230, 38, 100, 181, 179, 117, 28, 245, 15, 241,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha512,
            &password_short,
            &salt_short,
            1,
            &[
                105, 244, 213, 206, 245, 199, 216, 186, 147, 142, 136, 3, 136, 200, 246, 59, 107,
                36, 72, 178, 98, 109, 19, 67, 252, 92, 182, 139, 189, 127, 39, 178,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha512,
            &password_short,
            &salt_short,
            1000,
            &[
                134, 92, 89, 69, 225, 31, 91, 243, 221, 240, 2, 231, 203, 23, 72, 246, 34, 77, 38,
                113, 232, 6, 218, 212, 170, 240, 144, 160, 67, 103, 218, 41,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            &password_short,
            &salt_short,
            1,
            &[
                70, 36, 219, 210, 19, 115, 238, 86, 89, 193, 37, 177, 132, 238, 218, 162, 106, 51,
                183, 124, 161, 19, 20, 185, 240, 201, 218, 225, 228, 78, 155, 4,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha1,
            &password_short,
            &salt_short,
            1000,
            &[
                83, 136, 234, 94, 98, 225, 181, 87, 152, 26, 190, 92, 228, 19, 33, 39, 88, 170,
                106, 157, 44, 91, 240, 140, 1, 157, 69, 157, 186, 102, 107, 144,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &password_short,
            &salt_short,
            1,
            &[
                198, 188, 85, 164, 4, 173, 206, 163, 106, 26, 181, 103, 152, 8, 94, 10, 175, 105,
                127, 107, 178, 193, 106, 80, 114, 248, 56, 241, 125, 254, 108, 182,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &password_short,
            &salt_short,
            1000,
            &[
                78, 108, 165, 121, 87, 67, 155, 227, 167, 83, 112, 66, 66, 37, 226, 33, 29, 85,
                240, 90, 240, 5, 97, 223, 63, 62, 254, 233, 17, 107, 195, 76,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &password_long,
            &salt_long,
            1,
            &[
                253, 92, 174, 184, 179, 171, 229, 137, 188, 21, 156, 78, 81, 248, 0, 87, 14, 116,
                246, 67, 151, 166, 197, 238, 19, 29, 254, 217, 63, 5, 17, 170,
            ],
        ),
        (
            WebCryptoHashAlgorithm::Sha256,
            &password_long,
            &salt_long,
            1000,
            &[
                63, 213, 135, 201, 75, 169, 70, 184, 185, 220, 205, 221, 42, 91, 116, 246, 119,
                141, 79, 97, 230, 145, 248, 58, 196, 122, 47, 169, 88, 11, 253, 248,
            ],
        ),
    ];
    for (hash, password, salt, iterations, expected) in pbkdf2_cases {
        assert_eq!(
            derive_pbkdf2_bits(*hash, password, salt, *iterations, 256)
                .expect("PBKDF2 WPT matrix case should derive"),
            *expected
        );
    }

    assert!(hkdf_cases.len() >= 10);
    assert!(pbkdf2_cases.len() >= 10);
}

#[test]
fn x25519_derivation_truncates_to_requested_bits() {
    let alice = generate_x25519_key_pair().expect("alice keypair");
    let bob = generate_x25519_key_pair().expect("bob keypair");

    let alice_secret = derive_x25519_bits(&alice.private_key, bob.public_key, 128)
        .expect("alice should derive shared secret");
    let bob_secret = derive_x25519_bits(&bob.private_key, alice.public_key, 128)
        .expect("bob should derive shared secret");

    assert_eq!(alice_secret.len(), 16);
    assert_eq!(alice_secret, bob_secret);
}

#[test]
fn x25519_derivation_masks_non_byte_aligned_lengths() {
    let private_key = [
        200, 131, 142, 118, 208, 87, 223, 183, 216, 201, 90, 105, 225, 56, 22, 10, 221, 99, 115,
        253, 113, 164, 210, 118, 187, 86, 227, 168, 27, 100, 255, 97,
    ];
    let public_key = [
        28, 242, 177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250, 17, 84, 216, 62, 152,
        235, 100, 234, 81, 250, 229, 179, 48, 124, 254, 151, 6,
    ];

    let derived_230 = derive_x25519_bits(&private_key, public_key, 230)
        .expect("non-byte-aligned X25519 derivation should derive");
    assert_eq!(
        derived_230,
        [
            39, 104, 64, 157, 250, 185, 158, 194, 59, 140, 137, 185, 63, 245, 136, 2, 149, 247, 97,
            118, 8, 143, 137, 228, 61, 254, 190, 126, 160
        ]
    );

    assert_eq!(
        derive_x25519_bits(&private_key, public_key, 264),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn x25519_derivation_rejects_all_zero_shared_secret() {
    let private_key = [1_u8; 32];
    let public_key = [0_u8; 32];

    assert_eq!(
        derive_x25519_bits(&private_key, public_key, 256),
        Err(WebCryptoError::Operation)
    );
}

#[test]
fn x25519_import_export_matches_webcrypto_der_wrappers() {
    let public_key = [
        28, 242, 177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250, 17, 84, 216, 62, 152,
        235, 100, 234, 81, 250, 229, 179, 48, 124, 254, 151, 6,
    ];
    let private_key = [
        200, 131, 142, 118, 208, 87, 223, 183, 216, 201, 90, 105, 225, 56, 22, 10, 221, 99, 115,
        253, 113, 164, 210, 118, 187, 86, 227, 168, 27, 100, 255, 97,
    ];

    assert_eq!(x25519_public_key_from_private(&private_key), Ok(public_key));
    assert_eq!(
        import_x25519_spki_public_key(&export_x25519_spki_public_key(&public_key)),
        Ok(public_key)
    );
    assert_eq!(
        import_x25519_pkcs8_private_key(&export_x25519_pkcs8_private_key(&private_key))
            .expect("PKCS8 private key should import")
            .as_ref(),
        &private_key
    );
}

#[test]
fn x25519_jwk_import_rejects_invalid_shape() {
    let jwk = OkpJsonWebKeyImport {
        kty: Some("OKP".to_owned()),
        crv: Some("X25519".to_owned()),
        x: Some("HPKx5gIuxTc3Htf1PlT6EVTYPpjrZOpR-uWzMHz-lwY".to_owned()),
        d: Some("yIOOdtBX37fYyVpp4TgWCt1jc_1xpNJ2u1bjqBtk_2E".to_owned()),
        alg: None,
        key_ops: Some(vec!["deriveBits".to_owned()]),
        ext: Some(true),
        public_key_use: None,
    };

    assert!(matches!(
        import_x25519_jwk_key(&jwk, true, &["deriveBits".to_owned()]),
        Ok(X25519ImportedKey::Private(_))
    ));

    let wrong_usage = import_x25519_jwk_key(&jwk, true, &["deriveKey".to_owned()]);
    assert_eq!(wrong_usage, Err(WebCryptoError::Data));

    let duplicate_key_ops = OkpJsonWebKeyImport {
        key_ops: Some(vec!["deriveBits".to_owned(), "deriveBits".to_owned()]),
        ..jwk
    };
    assert_eq!(
        import_x25519_jwk_key(&duplicate_key_ops, true, &["deriveBits".to_owned()]),
        Err(WebCryptoError::Data)
    );

    let non_extractable = OkpJsonWebKeyImport {
        kty: Some("OKP".to_owned()),
        crv: Some("X25519".to_owned()),
        x: Some("HPKx5gIuxTc3Htf1PlT6EVTYPpjrZOpR-uWzMHz-lwY".to_owned()),
        d: Some("yIOOdtBX37fYyVpp4TgWCt1jc_1xpNJ2u1bjqBtk_2E".to_owned()),
        alg: None,
        key_ops: Some(vec!["deriveBits".to_owned()]),
        ext: Some(false),
        public_key_use: None,
    };
    assert_eq!(
        import_x25519_jwk_key(&non_extractable, true, &["deriveBits".to_owned()]),
        Err(WebCryptoError::Data)
    );

    let public_key = [
        28, 242, 177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250, 17, 84, 216, 62, 152,
        235, 100, 234, 81, 250, 229, 179, 48, 124, 254, 151, 6,
    ];
    let public_sig_use = OkpJsonWebKeyImport {
        kty: Some("OKP".to_owned()),
        crv: Some("X25519".to_owned()),
        x: Some("HPKx5gIuxTc3Htf1PlT6EVTYPpjrZOpR-uWzMHz-lwY".to_owned()),
        d: None,
        alg: None,
        key_ops: None,
        ext: Some(true),
        public_key_use: Some("sig".to_owned()),
    };
    assert_eq!(
        import_x25519_jwk_key(&public_sig_use, true, &[]),
        Ok(X25519ImportedKey::Public(public_key))
    );
    assert_eq!(
        import_x25519_jwk_key(&public_sig_use, true, &["deriveBits".to_owned()]),
        Err(WebCryptoError::Data)
    );

    let public_without_usage_metadata = OkpJsonWebKeyImport {
        public_key_use: None,
        ..public_sig_use
    };
    assert_eq!(
        import_x25519_jwk_key(
            &public_without_usage_metadata,
            true,
            &["deriveBits".to_owned()]
        ),
        Err(WebCryptoError::Syntax)
    );

    let private_without_usage_metadata = OkpJsonWebKeyImport {
        kty: Some("OKP".to_owned()),
        crv: Some("X25519".to_owned()),
        x: Some("HPKx5gIuxTc3Htf1PlT6EVTYPpjrZOpR-uWzMHz-lwY".to_owned()),
        d: Some("yIOOdtBX37fYyVpp4TgWCt1jc_1xpNJ2u1bjqBtk_2E".to_owned()),
        alg: None,
        key_ops: None,
        ext: Some(true),
        public_key_use: None,
    };
    assert_eq!(
        import_x25519_jwk_key(&private_without_usage_metadata, true, &[]),
        Err(WebCryptoError::Syntax)
    );
}

#[test]
fn okp_chromium_wpt_matrix_covers_cfrg_and_eddsa_cases() {
    // Ported from Chromium WPT
    // WebCryptoAPI/derive_bits_keys/cfrg_curves_bits_fixtures.js and the
    // EdDSA sign/verify matrix. This keeps X25519/X448 DER/raw/JWK handling,
    // small-order rejection, get-public-key, and Ed25519/Ed448 signing covered
    // with at least 10 independent backend cases.
    let x25519_pkcs8 = [
        48, 46, 2, 1, 0, 48, 5, 6, 3, 43, 101, 110, 4, 34, 4, 32, 200, 131, 142, 118, 208, 87, 223,
        183, 216, 201, 90, 105, 225, 56, 22, 10, 221, 99, 115, 253, 113, 164, 210, 118, 187, 86,
        227, 168, 27, 100, 255, 97,
    ];
    let x25519_spki = [
        48, 42, 48, 5, 6, 3, 43, 101, 110, 3, 33, 0, 28, 242, 177, 230, 2, 46, 197, 55, 55, 30,
        215, 245, 62, 84, 250, 17, 84, 216, 62, 152, 235, 100, 234, 81, 250, 229, 179, 48, 124,
        254, 151, 6,
    ];
    let x25519_expected = [
        39, 104, 64, 157, 250, 185, 158, 194, 59, 140, 137, 185, 63, 245, 136, 2, 149, 247, 97,
        118, 8, 143, 137, 228, 61, 254, 190, 126, 161, 149, 0, 8,
    ];
    let x448_pkcs8 = [
        48, 70, 2, 1, 0, 48, 5, 6, 3, 43, 101, 111, 4, 58, 4, 56, 88, 199, 210, 154, 62, 181, 25,
        178, 157, 0, 207, 177, 145, 187, 100, 252, 109, 138, 66, 216, 241, 113, 118, 39, 43, 137,
        242, 39, 45, 24, 25, 41, 92, 101, 37, 192, 130, 150, 113, 176, 82, 239, 7, 39, 83, 15, 24,
        142, 49, 208, 204, 83, 191, 38, 146, 158,
    ];
    let x448_spki = [
        48, 66, 48, 5, 6, 3, 43, 101, 111, 3, 57, 0, 182, 4, 161, 209, 165, 205, 29, 148, 38, 213,
        97, 239, 99, 10, 158, 177, 108, 190, 105, 213, 185, 202, 97, 94, 220, 83, 99, 62, 251, 82,
        234, 49, 230, 230, 160, 161, 219, 172, 198, 231, 108, 188, 230, 72, 45, 126, 75, 163, 213,
        93, 158, 128, 39, 101, 206, 111,
    ];
    let x448_expected = [
        240, 246, 197, 241, 127, 148, 244, 41, 30, 171, 113, 120, 134, 109, 55, 236, 137, 6, 221,
        108, 81, 65, 67, 220, 133, 190, 124, 242, 141, 239, 243, 155, 114, 110, 15, 109, 207, 129,
        14, 181, 148, 220, 169, 123, 72, 130, 189, 68, 196, 62, 167, 220, 103, 244, 154, 78,
    ];
    let mut cases = 0;

    let x25519_private = import_x25519_pkcs8_private_key(&x25519_pkcs8)
        .expect("X25519 WPT PKCS8 fixture should import");
    let x25519_public =
        import_x25519_spki_public_key(&x25519_spki).expect("X25519 WPT SPKI fixture should import");
    assert_eq!(
        x25519_public_key_from_private(&x25519_private),
        Ok(x25519_public)
    );
    assert_eq!(
        derive_x25519_bits(&x25519_private, x25519_public, 256)
            .expect("X25519 WPT fixture should derive"),
        x25519_expected
    );
    let mut x25519_expected_230 = x25519_expected[..29].to_vec();
    x25519_expected_230[28] &= 0b1111_1100;
    assert_eq!(
        derive_x25519_bits(&x25519_private, x25519_public, 230)
            .expect("X25519 truncated WPT fixture should derive"),
        x25519_expected_230
    );
    assert_eq!(
        import_x25519_raw_public_key(&x25519_public),
        Ok(x25519_public)
    );
    assert_eq!(
        import_x25519_spki_public_key(&export_x25519_spki_public_key(&x25519_public)),
        Ok(x25519_public)
    );
    assert_eq!(
        import_x25519_pkcs8_private_key(&export_x25519_pkcs8_private_key(&x25519_private))
            .expect("X25519 exported PKCS8 should import")
            .as_ref(),
        x25519_private.as_ref()
    );
    let x25519_jwk = export_x25519_jwk_private_key(
        &x25519_private,
        vec!["deriveBits".to_owned(), "deriveKey".to_owned()],
        true,
    )
    .expect("X25519 JWK should export");
    assert!(matches!(
        import_x25519_jwk_key(
            &OkpJsonWebKeyImport {
                kty: Some(x25519_jwk.kty.to_owned()),
                crv: Some(x25519_jwk.crv.to_owned()),
                x: Some(x25519_jwk.x),
                d: x25519_jwk.d,
                alg: x25519_jwk.alg.map(str::to_owned),
                key_ops: Some(x25519_jwk.key_ops),
                ext: Some(x25519_jwk.ext),
                public_key_use: None,
            },
            true,
            &["deriveBits".to_owned()],
        ),
        Ok(X25519ImportedKey::Private(_))
    ));
    assert_eq!(
        derive_x25519_bits(&x25519_private, [0_u8; 32], 256),
        Err(WebCryptoError::Operation)
    );
    cases += 8;

    let x448_private = import_okp_pkcs8_private_key(&x448_pkcs8, WebCryptoOkpCurve::X448)
        .expect("X448 WPT PKCS8 fixture should import");
    let x448_public = import_okp_spki_public_key(&x448_spki, WebCryptoOkpCurve::X448)
        .expect("X448 WPT SPKI fixture should import");
    assert_eq!(
        okp_public_key_from_private(WebCryptoOkpCurve::X448, x448_private.key_bytes.as_ref())
            .expect("X448 public key should derive from private key"),
        x448_public.key_bytes
    );
    assert_eq!(
        derive_x448_bits(x448_private.key_bytes.as_ref(), &x448_public.key_bytes, 448)
            .expect("X448 WPT fixture should derive"),
        x448_expected
    );
    assert_eq!(
        derive_x448_bits(x448_private.key_bytes.as_ref(), &x448_public.key_bytes, 257)
            .expect("X448 truncated WPT fixture should derive")
            .len(),
        33
    );
    assert_eq!(
        import_okp_spki_public_key(
            &export_okp_spki_public_key(WebCryptoOkpCurve::X448, &x448_public.key_bytes)
                .expect("X448 SPKI should export"),
            WebCryptoOkpCurve::X448,
        )
        .expect("X448 exported SPKI should import"),
        x448_public
    );
    assert_eq!(
        import_okp_pkcs8_private_key(
            &export_okp_pkcs8_private_key(WebCryptoOkpCurve::X448, x448_private.key_bytes.as_ref())
                .expect("X448 PKCS8 should export"),
            WebCryptoOkpCurve::X448,
        )
        .expect("X448 exported PKCS8 should import")
        .key_bytes
        .as_slice(),
        x448_private.key_bytes.as_slice()
    );
    assert_eq!(
        derive_x448_bits(x448_private.key_bytes.as_ref(), &[0_u8; 56], 448),
        Err(WebCryptoError::Operation)
    );
    cases += 6;

    for curve in [WebCryptoOkpCurve::Ed25519, WebCryptoOkpCurve::Ed448] {
        let pair = generate_okp_key_pair(curve).expect("EdDSA keygen should work");
        let signature = eddsa_sign(
            curve,
            pair.private_key.as_ref(),
            b"chromium eddsa matrix payload",
        )
        .expect("EdDSA should sign");
        assert!(
            eddsa_verify(
                curve,
                &pair.public_key,
                b"chromium eddsa matrix payload",
                &signature,
            )
            .expect("EdDSA should verify")
        );
        assert!(
            !eddsa_verify(curve, &pair.public_key, b"tampered", &signature)
                .expect("EdDSA tampered data should verify false")
        );
        assert!(
            !eddsa_verify(
                curve,
                &pair.public_key,
                b"chromium eddsa matrix payload",
                &signature[..signature.len() - 1],
            )
            .expect("EdDSA truncated signature should verify false")
        );
        assert_eq!(
            okp_public_key_from_private(curve, pair.private_key.as_ref())
                .expect("EdDSA public key should derive"),
            pair.public_key
        );
        assert_eq!(
            import_okp_raw_public_key(&pair.public_key, curve)
                .expect("EdDSA raw public key should import")
                .key_bytes,
            pair.public_key
        );
        let jwk = export_okp_jwk_private_key(
            curve,
            pair.private_key.as_ref(),
            vec!["sign".to_owned()],
            true,
        )
        .expect("EdDSA JWK should export");
        assert!(matches!(
            import_okp_jwk_key(
                &OkpJsonWebKeyImport {
                    kty: Some(jwk.kty.to_owned()),
                    crv: Some(jwk.crv.to_owned()),
                    x: Some(jwk.x),
                    d: jwk.d,
                    alg: jwk.alg.map(str::to_owned),
                    key_ops: Some(jwk.key_ops),
                    ext: Some(jwk.ext),
                    public_key_use: Some("sig".to_owned()),
                },
                curve,
                true,
                &["sign".to_owned()],
            ),
            Ok(OkpImportedKey::Private(_))
        ));
        cases += 6;
    }

    assert!(cases >= 10, "OKP matrix should keep at least 10 cases");
}

#[test]
fn okp_eddsa_verify_rejects_wpt_ed25519_small_order_points() {
    // Ported from WPT WebCryptoAPI/sign_verify/eddsa_vectors.js. OpenSSL's
    // Ed25519 verifier accepts several RFC 8032 small-order edge cases unless
    // the caller rejects the encoded public key / signature R point first.
    let small_order_points = [
        "0100000000000000000000000000000000000000000000000000000000000000",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "0000000000000000000000000000000000000000000000000000000000000080",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
        "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa",
        "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
        "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85",
        "0100000000000000000000000000000000000000000000000000000000000080",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ];
    let valid_pair =
        generate_okp_key_pair(WebCryptoOkpCurve::Ed25519).expect("Ed25519 keygen should work");
    let valid_signature = eddsa_sign(
        WebCryptoOkpCurve::Ed25519,
        valid_pair.private_key.as_ref(),
        b"small-order regression message",
    )
    .expect("Ed25519 signing should work");

    for point in small_order_points {
        let point = hex_bytes(point);
        assert_eq!(
            eddsa_verify(
                WebCryptoOkpCurve::Ed25519,
                &point,
                b"small-order regression message",
                &valid_signature,
            ),
            Ok(false),
            "small-order public key should resolve false"
        );

        let mut signature = valid_signature.clone();
        signature[..32].copy_from_slice(&point);
        assert_eq!(
            eddsa_verify(
                WebCryptoOkpCurve::Ed25519,
                &valid_pair.public_key,
                b"small-order regression message",
                &signature,
            ),
            Ok(false),
            "small-order signature R should resolve false"
        );
    }
}

#[test]
fn rsa_openssl_backend_round_trips_oaep_signatures_and_jwk() {
    let pair = generate_rsa_key_pair(2048, &[1, 0, 1]).expect("RSA keygen should work");
    assert_eq!(pair.modulus_length_bits, 2048);
    assert_eq!(pair.public_exponent, vec![1, 0, 1]);

    let ciphertext = rsa_oaep_encrypt(
        &pair.public_key,
        WebCryptoHashAlgorithm::Sha256,
        b"label",
        b"payload",
    )
    .expect("RSA-OAEP should encrypt");
    let plaintext = rsa_oaep_decrypt(
        pair.private_key.as_ref(),
        WebCryptoHashAlgorithm::Sha256,
        b"label",
        &ciphertext,
    )
    .expect("RSA-OAEP should decrypt");
    assert_eq!(plaintext, b"payload");
    assert_eq!(
        rsa_oaep_decrypt(
            pair.private_key.as_ref(),
            WebCryptoHashAlgorithm::Sha256,
            b"wrong",
            &ciphertext,
        ),
        Err(WebCryptoError::Operation)
    );

    let pkcs1_signature = rsa_pkcs1_sign(
        pair.private_key.as_ref(),
        WebCryptoHashAlgorithm::Sha384,
        b"signed payload",
    )
    .expect("RSASSA-PKCS1-v1_5 should sign");
    assert!(
        rsa_pkcs1_verify(
            &pair.public_key,
            WebCryptoHashAlgorithm::Sha384,
            b"signed payload",
            &pkcs1_signature,
        )
        .expect("RSASSA-PKCS1-v1_5 should verify")
    );
    assert!(
        !rsa_pkcs1_verify(
            &pair.public_key,
            WebCryptoHashAlgorithm::Sha384,
            b"tampered",
            &pkcs1_signature,
        )
        .expect("RSASSA-PKCS1-v1_5 bad input should verify false")
    );

    let pss_signature = rsa_pss_sign(
        pair.private_key.as_ref(),
        WebCryptoHashAlgorithm::Sha256,
        32,
        b"signed payload",
    )
    .expect("RSA-PSS should sign");
    assert!(
        rsa_pss_verify(
            &pair.public_key,
            WebCryptoHashAlgorithm::Sha256,
            32,
            b"signed payload",
            &pss_signature,
        )
        .expect("RSA-PSS should verify")
    );
    assert!(
        !rsa_pss_verify(
            &pair.public_key,
            WebCryptoHashAlgorithm::Sha256,
            20,
            b"signed payload",
            &pss_signature,
        )
        .expect("RSA-PSS wrong salt length should verify false")
    );

    let public = import_rsa_spki_public_key(&pair.public_key).expect("SPKI should import");
    let private =
        import_rsa_pkcs8_private_key(pair.private_key.as_ref()).expect("PKCS8 should import");
    assert_eq!(public.modulus_length_bits, 2048);
    assert_eq!(private.public_exponent, vec![1, 0, 1]);

    let exported = export_rsa_jwk_private_key(
        pair.private_key.as_ref(),
        WebCryptoKeyAlgorithm::RsaOaep,
        WebCryptoHashAlgorithm::Sha256,
        vec!["decrypt".to_owned()],
        true,
    )
    .expect("private RSA JWK should export");
    let import = RsaJsonWebKeyImport {
        kty: Some(exported.kty.to_owned()),
        n: Some(exported.n),
        e: Some(exported.e),
        d: exported.d,
        p: exported.p,
        q: exported.q,
        dp: exported.dp,
        dq: exported.dq,
        qi: exported.qi,
        alg: Some(exported.alg.to_owned()),
        key_ops: Some(exported.key_ops),
        ext: Some(exported.ext),
        public_key_use: Some("enc".to_owned()),
    };
    assert!(matches!(
        import_rsa_jwk_key(
            &import,
            WebCryptoKeyAlgorithm::RsaOaep,
            WebCryptoHashAlgorithm::Sha256,
            true,
            &["decrypt".to_owned()],
        ),
        Ok(RsaImportedKey::Private(_))
    ));
    assert_eq!(
        import_rsa_jwk_key(
            &import,
            WebCryptoKeyAlgorithm::RsaOaep,
            WebCryptoHashAlgorithm::Sha512,
            true,
            &["decrypt".to_owned()],
        ),
        Err(WebCryptoError::Data)
    );
}

#[test]
fn rsa_chromium_wpt_matrix_covers_oaep_pkcs1_pss_and_jwk_cases() {
    // Ported as a compact backend matrix from Chromium WPT
    // WebCryptoAPI/encrypt_decrypt/rsa_oaep and sign_verify/rsa*. The
    // renderer tests cover WebIDL and usage ordering; this test keeps the
    // OpenSSL RSA primitive surface above a 10-case floor.
    let pair = generate_rsa_key_pair(2048, &[1, 0, 1]).expect("RSA keygen should work");
    let hashes = [
        WebCryptoHashAlgorithm::Sha1,
        WebCryptoHashAlgorithm::Sha256,
        WebCryptoHashAlgorithm::Sha384,
        WebCryptoHashAlgorithm::Sha512,
    ];
    let mut cases = 0;

    for hash in hashes {
        let label = format!("oaep-label-{hash:?}");
        let plaintext = format!("oaep plaintext for {hash:?}");
        let ciphertext = rsa_oaep_encrypt(
            &pair.public_key,
            hash,
            label.as_bytes(),
            plaintext.as_bytes(),
        )
        .expect("RSA-OAEP matrix should encrypt");
        assert_eq!(
            rsa_oaep_decrypt(
                pair.private_key.as_ref(),
                hash,
                label.as_bytes(),
                &ciphertext,
            )
            .expect("RSA-OAEP matrix should decrypt"),
            plaintext.as_bytes()
        );
        assert_eq!(
            rsa_oaep_decrypt(pair.private_key.as_ref(), hash, b"wrong label", &ciphertext),
            Err(WebCryptoError::Operation)
        );
        cases += 2;

        let signature = rsa_pkcs1_sign(pair.private_key.as_ref(), hash, plaintext.as_bytes())
            .expect("RSASSA-PKCS1-v1_5 matrix should sign");
        assert!(
            rsa_pkcs1_verify(&pair.public_key, hash, plaintext.as_bytes(), &signature)
                .expect("RSASSA-PKCS1-v1_5 matrix should verify")
        );
        assert!(
            !rsa_pkcs1_verify(&pair.public_key, hash, b"tampered", &signature)
                .expect("RSASSA-PKCS1-v1_5 tampered data should verify false")
        );
        cases += 2;
    }

    for (hash, salt_length) in [
        (WebCryptoHashAlgorithm::Sha1, 0),
        (WebCryptoHashAlgorithm::Sha256, 16),
        (WebCryptoHashAlgorithm::Sha384, 32),
        (WebCryptoHashAlgorithm::Sha512, 64),
    ] {
        let signature = rsa_pss_sign(pair.private_key.as_ref(), hash, salt_length, b"pss payload")
            .expect("RSA-PSS matrix should sign");
        assert!(
            rsa_pss_verify(
                &pair.public_key,
                hash,
                salt_length,
                b"pss payload",
                &signature
            )
            .expect("RSA-PSS matrix should verify")
        );
        assert!(
            !rsa_pss_verify(
                &pair.public_key,
                hash,
                salt_length.saturating_add(1),
                b"pss payload",
                &signature,
            )
            .expect("RSA-PSS wrong salt length should verify false")
        );
        cases += 2;
    }

    let public = import_rsa_spki_public_key(&pair.public_key).expect("RSA SPKI should import");
    assert_eq!(public.modulus_length_bits, 2048);
    let private =
        import_rsa_pkcs8_private_key(pair.private_key.as_ref()).expect("RSA PKCS8 should import");
    assert_eq!(private.public_exponent, vec![1, 0, 1]);
    cases += 2;

    for (algorithm, hash, usage, public_key_use) in [
        (
            WebCryptoKeyAlgorithm::RsaOaep,
            WebCryptoHashAlgorithm::Sha256,
            "decrypt",
            "enc",
        ),
        (
            WebCryptoKeyAlgorithm::RsaPss,
            WebCryptoHashAlgorithm::Sha384,
            "sign",
            "sig",
        ),
        (
            WebCryptoKeyAlgorithm::RsassaPkcs1V15,
            WebCryptoHashAlgorithm::Sha512,
            "sign",
            "sig",
        ),
    ] {
        let jwk = export_rsa_jwk_private_key(
            pair.private_key.as_ref(),
            algorithm,
            hash,
            vec![usage.to_owned()],
            true,
        )
        .expect("RSA private JWK should export");
        let import = RsaJsonWebKeyImport {
            kty: Some(jwk.kty.to_owned()),
            n: Some(jwk.n),
            e: Some(jwk.e),
            d: jwk.d,
            p: jwk.p,
            q: jwk.q,
            dp: jwk.dp,
            dq: jwk.dq,
            qi: jwk.qi,
            alg: Some(jwk.alg.to_owned()),
            key_ops: Some(jwk.key_ops),
            ext: Some(jwk.ext),
            public_key_use: Some(public_key_use.to_owned()),
        };
        assert!(matches!(
            import_rsa_jwk_key(&import, algorithm, hash, true, &[usage.to_owned()]),
            Ok(RsaImportedKey::Private(_))
        ));
        assert_eq!(
            import_rsa_jwk_key(
                &import,
                algorithm,
                WebCryptoHashAlgorithm::Sha1,
                true,
                &[usage.to_owned()],
            ),
            Err(WebCryptoError::Data)
        );
        cases += 2;
    }

    assert!(cases >= 10, "RSA matrix should keep at least 10 cases");
}

#[test]
fn ec_openssl_backend_round_trips_ecdsa_ecdh_raw_and_jwk() {
    let pair = generate_ec_key_pair(WebCryptoEcNamedCurve::P256).expect("P-256 keygen should work");
    let peer =
        generate_ec_key_pair(WebCryptoEcNamedCurve::P256).expect("P-256 peer keygen should work");

    let signature = ecdsa_sign(
        pair.private_key.as_ref(),
        WebCryptoEcNamedCurve::P256,
        WebCryptoHashAlgorithm::Sha256,
        b"payload",
    )
    .expect("ECDSA should sign");
    assert_eq!(signature.len(), 64);
    assert!(
        ecdsa_verify(
            &pair.public_key,
            WebCryptoEcNamedCurve::P256,
            WebCryptoHashAlgorithm::Sha256,
            b"payload",
            &signature,
        )
        .expect("ECDSA should verify")
    );
    assert!(
        !ecdsa_verify(
            &pair.public_key,
            WebCryptoEcNamedCurve::P256,
            WebCryptoHashAlgorithm::Sha256,
            b"tampered",
            &signature,
        )
        .expect("ECDSA bad input should verify false")
    );

    let raw_public = export_ec_raw_public_key(&pair.public_key).expect("EC raw should export");
    assert_eq!(raw_public.len(), 65);
    let imported_public = import_ec_raw_public_key(&raw_public, WebCryptoEcNamedCurve::P256)
        .expect("EC raw should import");
    assert_eq!(
        export_ec_raw_public_key(&imported_public.key_bytes).unwrap(),
        raw_public
    );

    let first_secret = derive_ecdh_bits(
        pair.private_key.as_ref(),
        &peer.public_key,
        WebCryptoEcNamedCurve::P256,
        256,
    )
    .expect("ECDH should derive");
    let second_secret = derive_ecdh_bits(
        peer.private_key.as_ref(),
        &pair.public_key,
        WebCryptoEcNamedCurve::P256,
        256,
    )
    .expect("ECDH peer should derive");
    assert_eq!(first_secret, second_secret);
    assert_eq!(
        derive_ecdh_bits(
            pair.private_key.as_ref(),
            &peer.public_key,
            WebCryptoEcNamedCurve::P256,
            257,
        ),
        Err(WebCryptoError::Operation)
    );

    let private_jwk = export_ec_jwk_private_key(
        pair.private_key.as_ref(),
        WebCryptoKeyAlgorithm::Ecdsa,
        vec!["sign".to_owned()],
        true,
    )
    .expect("EC JWK should export");
    let import = EcJsonWebKeyImport {
        kty: Some(private_jwk.kty.to_owned()),
        crv: Some(private_jwk.crv.to_owned()),
        x: Some(private_jwk.x),
        y: Some(private_jwk.y),
        d: private_jwk.d,
        alg: private_jwk.alg.map(str::to_owned),
        key_ops: Some(private_jwk.key_ops),
        ext: Some(private_jwk.ext),
        public_key_use: Some("sig".to_owned()),
    };
    assert!(matches!(
        import_ec_jwk_key(
            &import,
            WebCryptoKeyAlgorithm::Ecdsa,
            WebCryptoEcNamedCurve::P256,
            true,
            &["sign".to_owned()],
        ),
        Ok(EcImportedKey::Private(_))
    ));
    assert_eq!(
        import_ec_jwk_key(
            &import,
            WebCryptoKeyAlgorithm::Ecdsa,
            WebCryptoEcNamedCurve::P384,
            true,
            &["sign".to_owned()],
        ),
        Err(WebCryptoError::Data)
    );
}

#[test]
fn ec_chromium_wpt_matrix_covers_ecdsa_ecdh_import_export_cases() {
    // Ported as a compact backend matrix from Chromium WPT
    // WebCryptoAPI/sign_verify/ecdsa_vectors.js and derive_bits_keys/ecdh_*.
    // It intentionally exercises every implemented NIST curve against all
    // supported hash families plus ECDH raw/SPKI/PKCS8/JWK handling.
    let hashes = [
        WebCryptoHashAlgorithm::Sha1,
        WebCryptoHashAlgorithm::Sha256,
        WebCryptoHashAlgorithm::Sha384,
        WebCryptoHashAlgorithm::Sha512,
    ];
    let curves = [
        WebCryptoEcNamedCurve::P256,
        WebCryptoEcNamedCurve::P384,
        WebCryptoEcNamedCurve::P521,
    ];
    let mut cases = 0;

    for curve in curves {
        let pair = generate_ec_key_pair(curve).expect("EC keygen should work");
        let peer = generate_ec_key_pair(curve).expect("EC peer keygen should work");
        let coordinate_len = curve.coordinate_len_bytes();
        let full_length_bits = coordinate_len * 8;

        for hash in hashes {
            let payload = format!("ecdsa payload {curve:?} {hash:?}");
            let signature = ecdsa_sign(pair.private_key.as_ref(), curve, hash, payload.as_bytes())
                .expect("ECDSA matrix should sign");
            assert_eq!(signature.len(), coordinate_len * 2);
            assert!(
                ecdsa_verify(
                    &pair.public_key,
                    curve,
                    hash,
                    payload.as_bytes(),
                    &signature
                )
                .expect("ECDSA matrix should verify")
            );
            assert!(
                !ecdsa_verify(&pair.public_key, curve, hash, b"tampered", &signature)
                    .expect("ECDSA tampered data should verify false")
            );
            assert!(
                !ecdsa_verify(
                    &pair.public_key,
                    curve,
                    hash,
                    payload.as_bytes(),
                    &signature[..signature.len() - 1],
                )
                .expect("ECDSA truncated signature should verify false")
            );
            cases += 3;
        }

        let raw_public =
            export_ec_raw_public_key(&pair.public_key).expect("EC raw public key should export");
        assert_eq!(raw_public.len(), 1 + coordinate_len * 2);
        assert_eq!(
            import_ec_raw_public_key(&raw_public, curve)
                .expect("EC raw public key should import")
                .curve,
            curve
        );
        assert_eq!(
            import_ec_spki_public_key(&pair.public_key, curve)
                .expect("EC SPKI should import")
                .curve,
            curve
        );
        assert_eq!(
            import_ec_pkcs8_private_key(pair.private_key.as_ref(), curve)
                .expect("EC PKCS8 should import")
                .curve,
            curve
        );
        cases += 3;

        let secret = derive_ecdh_bits(
            pair.private_key.as_ref(),
            &peer.public_key,
            curve,
            full_length_bits,
        )
        .expect("ECDH full-length matrix should derive");
        let peer_secret = derive_ecdh_bits(
            peer.private_key.as_ref(),
            &pair.public_key,
            curve,
            full_length_bits,
        )
        .expect("ECDH peer full-length matrix should derive");
        assert_eq!(secret, peer_secret);
        assert_eq!(
            derive_ecdh_bits(pair.private_key.as_ref(), &peer.public_key, curve, 129)
                .expect("ECDH truncated matrix should derive")
                .len(),
            17
        );
        assert_eq!(
            derive_ecdh_bits(
                pair.private_key.as_ref(),
                &peer.public_key,
                curve,
                full_length_bits + 1,
            ),
            Err(WebCryptoError::Operation)
        );
        cases += 3;

        for algorithm in [WebCryptoKeyAlgorithm::Ecdsa, WebCryptoKeyAlgorithm::Ecdh] {
            let usage = if algorithm == WebCryptoKeyAlgorithm::Ecdsa {
                "sign"
            } else {
                "deriveBits"
            };
            let public_key_use = if algorithm == WebCryptoKeyAlgorithm::Ecdsa {
                "sig"
            } else {
                "enc"
            };
            let jwk = export_ec_jwk_private_key(
                pair.private_key.as_ref(),
                algorithm,
                vec![usage.to_owned()],
                true,
            )
            .expect("EC private JWK should export");
            assert!(matches!(
                import_ec_jwk_key(
                    &EcJsonWebKeyImport {
                        kty: Some(jwk.kty.to_owned()),
                        crv: Some(jwk.crv.to_owned()),
                        x: Some(jwk.x),
                        y: Some(jwk.y),
                        d: jwk.d,
                        alg: jwk.alg.map(str::to_owned),
                        key_ops: Some(jwk.key_ops),
                        ext: Some(jwk.ext),
                        public_key_use: Some(public_key_use.to_owned()),
                    },
                    algorithm,
                    curve,
                    true,
                    &[usage.to_owned()],
                ),
                Ok(EcImportedKey::Private(_))
            ));
            cases += 1;
        }
    }

    assert!(cases >= 10, "EC matrix should keep at least 10 cases");
}

#[test]
fn okp_rustcrypto_ed448_matches_chromium_wpt_vector() {
    let pkcs8 = hex_bytes(concat!(
        "3047020100300506032b6571043b04390eff03458c28e0179c521de312c969b7",
        "834348ecab991a60e3b2e9a79e4cd9e480ef291712d2c83d047272d5c9f4286",
        "64f696d2670458f1d2e",
    ));
    let spki = hex_bytes(concat!(
        "3043300506032b6571033a00ab4bb885fd7d2c5af24e83710cffa0c74a57e27",
        "4801db2057b0bdc5ea032b6fe6bc78b8045365aeb26e86e1f14fd349d07c484",
        "95f5a46a5a80",
    ));
    let data = hex_bytes(concat!(
        "2b7ed0bc7795694ab4acd35903fe8cd7d80f6a1c8688a6c3414409457514a14",
        "57855bbb219e30a1beea8fe869082d99fc8282f9050d024e59eaf0730ba9db7",
        "0a",
    ));
    let expected_signature = hex_bytes(concat!(
        "76897e8c50ac6b1132735c09c55f506c0149d2677c75664f8bc10b826fbd9df",
        "0a03cd986bce8339e64c7d1720ea9361784dc73837765ac2980c0dac0814a8bc",
        "187d1c9c907c5dcc07956f85b70930fe42de764177217cb2d52bab7c1debe0c",
        "a89ccecbcd63f7025a2a5a572b9d23b0642f00",
    ));

    let private_key = import_okp_pkcs8_private_key(&pkcs8, WebCryptoOkpCurve::Ed448)
        .expect("Ed448 WPT PKCS8 fixture should import");
    let public_key = import_okp_spki_public_key(&spki, WebCryptoOkpCurve::Ed448)
        .expect("Ed448 WPT SPKI fixture should import");
    assert_eq!(
        okp_public_key_from_private(WebCryptoOkpCurve::Ed448, &private_key.key_bytes)
            .expect("Ed448 public key should derive"),
        public_key.key_bytes
    );
    assert_eq!(
        eddsa_sign(WebCryptoOkpCurve::Ed448, &private_key.key_bytes, &data)
            .expect("Ed448 WPT fixture should sign"),
        expected_signature
    );
    assert!(
        eddsa_verify(
            WebCryptoOkpCurve::Ed448,
            &public_key.key_bytes,
            &data,
            &expected_signature,
        )
        .expect("Ed448 WPT fixture should verify")
    );
    assert_eq!(
        export_okp_spki_public_key(WebCryptoOkpCurve::Ed448, &public_key.key_bytes)
            .expect("Ed448 SPKI should export"),
        spki
    );
    assert_eq!(
        export_okp_pkcs8_private_key(WebCryptoOkpCurve::Ed448, &private_key.key_bytes)
            .expect("Ed448 PKCS8 should export"),
        pkcs8
    );

    let mut identity = [0_u8; 57];
    identity[0] = 1;
    assert!(import_okp_raw_public_key(&identity, WebCryptoOkpCurve::Ed448).is_ok());
    assert_eq!(
        eddsa_verify(
            WebCryptoOkpCurve::Ed448,
            &identity,
            &data,
            &expected_signature,
        ),
        Ok(false)
    );
}

#[test]
fn okp_backends_cover_eddsa_and_x448() {
    let ed25519 =
        generate_okp_key_pair(WebCryptoOkpCurve::Ed25519).expect("Ed25519 keygen should work");
    let signature = eddsa_sign(
        WebCryptoOkpCurve::Ed25519,
        ed25519.private_key.as_ref(),
        b"payload",
    )
    .expect("Ed25519 should sign");
    assert!(
        eddsa_verify(
            WebCryptoOkpCurve::Ed25519,
            &ed25519.public_key,
            b"payload",
            &signature,
        )
        .expect("Ed25519 should verify")
    );
    assert!(
        !eddsa_verify(
            WebCryptoOkpCurve::Ed25519,
            &ed25519.public_key,
            b"tampered",
            &signature,
        )
        .expect("Ed25519 bad input should verify false")
    );

    let ed448 = generate_okp_key_pair(WebCryptoOkpCurve::Ed448).expect("Ed448 keygen should work");
    let ed448_signature = eddsa_sign(
        WebCryptoOkpCurve::Ed448,
        ed448.private_key.as_ref(),
        b"payload",
    )
    .expect("Ed448 should sign");
    assert!(
        eddsa_verify(
            WebCryptoOkpCurve::Ed448,
            &ed448.public_key,
            b"payload",
            &ed448_signature,
        )
        .expect("Ed448 should verify")
    );

    let first = generate_okp_key_pair(WebCryptoOkpCurve::X448).expect("X448 keygen should work");
    let second =
        generate_okp_key_pair(WebCryptoOkpCurve::X448).expect("X448 peer keygen should work");
    let first_secret = derive_x448_bits(first.private_key.as_ref(), &second.public_key, 448)
        .expect("X448 should derive");
    let second_secret = derive_x448_bits(second.private_key.as_ref(), &first.public_key, 448)
        .expect("X448 peer should derive");
    assert_eq!(first_secret, second_secret);
    assert_eq!(
        derive_x448_bits(first.private_key.as_ref(), &second.public_key, 449),
        Err(WebCryptoError::Operation)
    );

    let spki = export_okp_spki_public_key(WebCryptoOkpCurve::X448, &first.public_key)
        .expect("X448 SPKI should export");
    let imported_public = import_okp_spki_public_key(&spki, WebCryptoOkpCurve::X448)
        .expect("X448 SPKI should import");
    assert_eq!(imported_public.key_bytes, first.public_key);
    let pkcs8 = export_okp_pkcs8_private_key(WebCryptoOkpCurve::X448, first.private_key.as_ref())
        .expect("X448 PKCS8 should export");
    let imported_private = import_okp_pkcs8_private_key(&pkcs8, WebCryptoOkpCurve::X448)
        .expect("X448 PKCS8 should import");
    assert_eq!(
        imported_private.key_bytes.as_slice(),
        first.private_key.as_slice()
    );

    let jwk = export_okp_jwk_private_key(
        WebCryptoOkpCurve::X448,
        first.private_key.as_ref(),
        vec!["deriveBits".to_owned()],
        true,
    )
    .expect("X448 JWK should export");
    let import = OkpJsonWebKeyImport {
        kty: Some(jwk.kty.to_owned()),
        crv: Some(jwk.crv.to_owned()),
        x: Some(jwk.x),
        d: jwk.d,
        alg: jwk.alg.map(str::to_owned),
        key_ops: Some(jwk.key_ops),
        ext: Some(jwk.ext),
        public_key_use: Some("enc".to_owned()),
    };
    assert!(matches!(
        import_okp_jwk_key(
            &import,
            WebCryptoOkpCurve::X448,
            true,
            &["deriveBits".to_owned()],
        ),
        Ok(OkpImportedKey::Private(_))
    ));
}
