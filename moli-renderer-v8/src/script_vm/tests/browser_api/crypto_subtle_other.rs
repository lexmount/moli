use super::*;

#[test]
fn crypto_subtle_same_object_does_not_leak_internal_slot() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const cryptoOwnNames = Object.getOwnPropertyNames(crypto);
              const subtleOwnNames = Object.getOwnPropertyNames(crypto.subtle);
              return [
                crypto.subtle === crypto.subtle,
                crypto.subtle instanceof SubtleCrypto,
                Object.getPrototypeOf(crypto) === Crypto.prototype,
                Object.getPrototypeOf(crypto.subtle) === SubtleCrypto.prototype,
                cryptoOwnNames.some((name) => name.startsWith("__moli")),
                subtleOwnNames.some((name) => name.startsWith("__moli")),
                Object.keys(crypto).join(","),
                Object.keys(crypto.subtle).join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("crypto subtle surface probe should evaluate");

    assert_eq!(result, "true|true|true|true|false|false||");
}
#[test]
fn crypto_subtle_prototype_methods_reject_wrong_receiver() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoSubtleReceiverProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const prototype = SubtleCrypto.prototype;
            const bytes = new Uint8Array(16);
            bytes.forEach((_, index) => { bytes[index] = index + 1; });
            const data = new Uint8Array([1, 2, 3]);
            const hmac = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign", "verify"]
            );
            const signature = await subtle.sign("HMAC", hmac, data);
            const aesCbc = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["encrypt", "decrypt", "wrapKey", "unwrapKey"]
            );
            const aesKw = await subtle.importKey(
              "raw",
              bytes,
              "AES-KW",
              true,
              ["wrapKey", "unwrapKey"]
            );
            const hkdf = await subtle.importKey(
              "raw",
              bytes,
              "HKDF",
              false,
              ["deriveBits", "deriveKey"]
            );
            const x25519 = await subtle.generateKey(
              "X25519",
              true,
              ["deriveBits", "deriveKey"]
            );
            const hkdfParams = {
              name: "HKDF",
              hash: "SHA-256",
              salt: new Uint8Array([9]),
              info: new Uint8Array([8])
            };
            const callName = (method, ...args) => {
              try {
                return Promise.resolve(prototype[method].call({}, ...args)).then(
                  () => "resolved",
                  (error) => error.name
                );
              } catch (error) {
                return Promise.resolve(error.name);
              }
            };

            // WebIDL instance operations require a SubtleCrypto receiver before
            // argument conversion or operation-specific DOMException checks.
            globalThis.__cryptoSubtleReceiverProbe = await Promise.all([
              callName("digest", "SHA-256", data),
              callName("generateKey", { name: "HMAC", hash: "SHA-256" }, true, ["sign"]),
              callName("sign", "HMAC", hmac, data),
              callName("verify", "HMAC", hmac, signature, data),
              callName("encrypt", { name: "AES-CBC", iv: new Uint8Array(16) }, aesCbc, data),
              callName("decrypt", { name: "AES-CBC", iv: new Uint8Array(16) }, aesCbc, data),
              callName("deriveBits", hkdfParams, hkdf, 128),
              callName(
                "deriveKey",
                hkdfParams,
                hkdf,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              ),
              callName("getPublicKey", x25519.privateKey, []),
              callName("importKey", "raw", bytes, "AES-CBC", true, ["encrypt"]),
              callName("exportKey", "raw", hmac),
              callName("wrapKey", "raw", hmac, aesKw, "AES-KW"),
              callName(
                "unwrapKey",
                "raw",
                new Uint8Array(16),
                aesKw,
                "AES-KW",
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
              )
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle receiver brand probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoSubtleReceiverProbe)")
        .expect("crypto subtle receiver brand promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
}
#[test]
fn crypto_subtle_operation_lengths_match_webcrypto_idl() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const prototype = SubtleCrypto.prototype;
              return [
                prototype.digest.length,
                prototype.generateKey.length,
                prototype.encrypt.length,
                prototype.decrypt.length,
                prototype.sign.length,
                prototype.verify.length,
                // WebCrypto defines deriveBits length as optional nullable, so
                // Function.length stops before that third argument.
                prototype.deriveBits.length,
                prototype.deriveKey.length,
                prototype.importKey.length,
                prototype.exportKey.length,
                prototype.wrapKey.length,
                prototype.unwrapKey.length,
                SubtleCrypto.supports.length
              ].join(",");
            })()
            "#,
        )
        .expect("crypto subtle operation length probe should evaluate");

    assert_eq!(result, "2,3,3,3,3,4,2,5,5,2,4,7,2");
}
#[test]
fn crypto_subtle_generated_key_algorithm_objects_keep_expected_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKeyAlgorithmProbe = ["pending"];
          (async () => {
            const hmac = await crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign", "verify"]
            );
            const aes = await crypto.subtle.generateKey(
              { name: "AES-GCM", length: 128 },
              true,
              ["encrypt", "decrypt"]
            );
            const rsa = await crypto.subtle.generateKey(
              {
                name: "RSA-OAEP",
                modulusLength: 1024,
                publicExponent: new Uint8Array([1, 0, 1]),
                hash: "SHA-256"
              },
              true,
              ["encrypt", "decrypt"]
            );
            const ecdsa = await crypto.subtle.generateKey(
              { name: "ECDSA", namedCurve: "P-256" },
              true,
              ["sign", "verify"]
            );
            globalThis.__cryptoKeyAlgorithmProbe = [
              Object.keys(hmac.algorithm).join(","),
              hmac.algorithm.name,
              Object.keys(hmac.algorithm.hash).join(","),
              hmac.algorithm.hash.name,
              String(hmac.algorithm.length),
              hmac.extractable,
              hmac.usages.join(","),
              Object.keys(aes.algorithm).join(","),
              aes.algorithm.name,
              String(aes.algorithm.length),
              Object.keys(rsa.publicKey.algorithm).join(","),
              rsa.publicKey.algorithm.name,
              Object.keys(rsa.publicKey.algorithm.hash).join(","),
              rsa.publicKey.algorithm.hash.name,
              String(rsa.publicKey.algorithm.modulusLength),
              Array.from(rsa.publicKey.algorithm.publicExponent).join("."),
              Object.keys(ecdsa.publicKey.algorithm).join(","),
              ecdsa.publicKey.algorithm.name,
              ecdsa.publicKey.algorithm.namedCurve
            ];
          })();
        })()
        "#,
    )
    .expect("crypto.subtle generateKey probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKeyAlgorithmProbe)")
        .expect("crypto.subtle generateKey promise should settle");

    assert_eq!(
        result,
        r#"["name,hash,length","HMAC","name","SHA-256","512",true,"sign,verify","name,length","AES-GCM","128","name,hash,modulusLength,publicExponent","RSA-OAEP","name","SHA-256","1024","1.0.1","name,namedCurve","ECDSA","P-256"]"#
    );
}
#[test]
fn crypto_subtle_openssl_asymmetric_algorithms_are_reachable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoOpenSslAsymmetricFailures = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const failures = [];
            const data = new TextEncoder().encode("openssl asymmetric webcrypto");
            const sameBytes = (left, right) => {
              const a = new Uint8Array(left);
              const b = new Uint8Array(right);
              return a.length === b.length && a.every((value, index) => value === b[index]);
            };
            const expect = (condition, label) => {
              if (!condition) failures.push(label);
            };

            try {
              const rsaOaep = await subtle.generateKey(
                {
                  name: "RSA-OAEP",
                  modulusLength: 1024,
                  publicExponent: new Uint8Array([1, 0, 1]),
                  hash: "SHA-256"
                },
                true,
                ["encrypt", "decrypt", "wrapKey", "unwrapKey"]
              );
              const label = new Uint8Array([7, 8, 9]);
              const ciphertext = await subtle.encrypt(
                { name: "RSA-OAEP", label },
                rsaOaep.publicKey,
                data
              );
              const plaintext = await subtle.decrypt(
                { name: "RSA-OAEP", label },
                rsaOaep.privateKey,
                ciphertext
              );
              expect(sameBytes(plaintext, data), "rsa-oaep-roundtrip");
              const aes = await subtle.generateKey(
                { name: "AES-GCM", length: 128 },
                true,
                ["encrypt", "decrypt"]
              );
              const wrapped = await subtle.wrapKey(
                "raw",
                aes,
                rsaOaep.publicKey,
                "RSA-OAEP"
              );
              const unwrapped = await subtle.unwrapKey(
                "raw",
                wrapped,
                rsaOaep.privateKey,
                "RSA-OAEP",
                "AES-GCM",
                true,
                ["encrypt", "decrypt"]
              );
              expect(
                sameBytes(await subtle.exportKey("raw", aes), await subtle.exportKey("raw", unwrapped)),
                "rsa-oaep-wrap"
              );

              const rsaPss = await subtle.generateKey(
                {
                  name: "RSA-PSS",
                  modulusLength: 1024,
                  publicExponent: new Uint8Array([1, 0, 1]),
                  hash: "SHA-256"
                },
                true,
                ["sign", "verify"]
              );
              const pssSignature = await subtle.sign(
                { name: "RSA-PSS", saltLength: 32 },
                rsaPss.privateKey,
                data
              );
              expect(
                await subtle.verify(
                  { name: "RSA-PSS", saltLength: 32 },
                  rsaPss.publicKey,
                  pssSignature,
                  data
                ),
                "rsa-pss"
              );

              const rsassa = await subtle.generateKey(
                {
                  name: "RSASSA-PKCS1-v1_5",
                  modulusLength: 1024,
                  publicExponent: new Uint8Array([1, 0, 1]),
                  hash: "SHA-256"
                },
                true,
                ["sign", "verify"]
              );
              const rsassaSignature = await subtle.sign("RSASSA-PKCS1-v1_5", rsassa.privateKey, data);
              expect(
                await subtle.verify("RSASSA-PKCS1-v1_5", rsassa.publicKey, rsassaSignature, data),
                "rsassa"
              );

              const ecdsa = await subtle.generateKey(
                { name: "ECDSA", namedCurve: "P-256" },
                true,
                ["sign", "verify"]
              );
              const ecdsaSignature = await subtle.sign(
                { name: "ECDSA", hash: "SHA-256" },
                ecdsa.privateKey,
                data
              );
              const ecdsaRawPublic = await subtle.exportKey("raw", ecdsa.publicKey);
              const importedEcdsaPublic = await subtle.importKey(
                "raw",
                ecdsaRawPublic,
                { name: "ECDSA", namedCurve: "P-256" },
                true,
                ["verify"]
              );
              expect(
                await subtle.verify(
                  { name: "ECDSA", hash: "SHA-256" },
                  importedEcdsaPublic,
                  ecdsaSignature,
                  data
                ),
                "ecdsa"
              );

              const ecdhA = await subtle.generateKey(
                { name: "ECDH", namedCurve: "P-256" },
                true,
                ["deriveBits", "deriveKey"]
              );
              const ecdhB = await subtle.generateKey(
                { name: "ECDH", namedCurve: "P-256" },
                true,
                ["deriveBits", "deriveKey"]
              );
              const ecdhBitsA = await subtle.deriveBits(
                { name: "ECDH", public: ecdhB.publicKey },
                ecdhA.privateKey,
                256
              );
              const ecdhBitsB = await subtle.deriveBits(
                { name: "ECDH", public: ecdhA.publicKey },
                ecdhB.privateKey,
                256
              );
              expect(sameBytes(ecdhBitsA, ecdhBitsB), "ecdh-bits");
              const ecdhHmac = await subtle.deriveKey(
                { name: "ECDH", public: ecdhB.publicKey },
                ecdhA.privateKey,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              );
              expect((await subtle.exportKey("raw", ecdhHmac)).byteLength === 16, "ecdh-derive-key");

              const ed25519 = await subtle.generateKey("Ed25519", true, ["sign", "verify"]);
              const ed25519Signature = await subtle.sign("Ed25519", ed25519.privateKey, data);
              expect(
                await subtle.verify("Ed25519", ed25519.publicKey, ed25519Signature, data),
                "ed25519"
              );
              const ed448 = await subtle.generateKey("Ed448", true, ["sign", "verify"]);
              const ed448Signature = await subtle.sign("Ed448", ed448.privateKey, data);
              expect(
                await subtle.verify("Ed448", ed448.publicKey, ed448Signature, data),
                "ed448"
              );

              const x448A = await subtle.generateKey("X448", true, ["deriveBits", "deriveKey"]);
              const x448B = await subtle.generateKey("X448", true, ["deriveBits", "deriveKey"]);
              const x448APublic = await subtle.getPublicKey(x448A.privateKey, []);
              const x448RawA = await subtle.exportKey("raw", x448A.publicKey);
              expect(
                sameBytes(await subtle.exportKey("raw", x448APublic), x448RawA),
                "x448-get-public"
              );
              const x448BitsA = await subtle.deriveBits(
                { name: "X448", public: x448B.publicKey },
                x448A.privateKey,
                448
              );
              const x448BitsB = await subtle.deriveBits(
                { name: "X448", public: x448A.publicKey },
                x448B.privateKey,
                448
              );
              expect(sameBytes(x448BitsA, x448BitsB), "x448-bits");
            } catch (error) {
              failures.push(`throw:${error && error.name}:${error && error.message}`);
            }

            globalThis.__cryptoOpenSslAsymmetricFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle OpenSSL asymmetric probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoOpenSslAsymmetricFailures)")
        .expect("crypto subtle OpenSSL asymmetric promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_buffer_source_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoBufferSourceProbe = [];
          const bytes = new Uint8Array([97, 98, 99]);
          const view = new Uint8Array(bytes.buffer, 1, 1);
          const dataView = new DataView(new Uint8Array([100]).buffer);
          const shared = new WebAssembly.Memory({shared:true,initial:1,maximum:1}).buffer.slice(0,16);
          const rejection = (promise) => promise.then(
            () => "resolved",
            (error) => error.name
          );
          Promise.all([
            crypto.subtle.digest("SHA-256", bytes.buffer)
              .then((value) => new Uint8Array(value).length),
            crypto.subtle.digest("SHA-1", view)
              .then((value) => new Uint8Array(value).length),
            crypto.subtle.digest("SHA-384", dataView)
              .then((value) => new Uint8Array(value).length),
            rejection(crypto.subtle.digest("SHA-256")),
            rejection(crypto.subtle.digest("SHA-256", null)),
            rejection(crypto.subtle.digest("SHA-256", {})),
            crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-512" },
              true,
              ["sign", "verify"]
            ).then((key) => crypto.subtle.sign("HMAC", key, view)
              .then((signature) => crypto.subtle.verify("HMAC", key, signature, view)
                .then((verified) => [
                  new Uint8Array(signature).length,
                  String(verified)
                ].join(":")))),
            shared
              ? crypto.subtle.importKey("raw", bytes, "HKDF", false, ["deriveBits"])
                  .then((key) => rejection(crypto.subtle.deriveBits(
                    { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(shared), info: new Uint8Array(0) },
                    key,
                    128
                  )))
              : Promise.resolve("unsupported"),
            shared
              ? crypto.subtle.importKey("raw", bytes, "HKDF", false, ["deriveBits"])
                  .then((key) => rejection(crypto.subtle.deriveBits(
                    { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(0), info: new Uint8Array(shared) },
                    key,
                    128
                  )))
              : Promise.resolve("unsupported"),
            shared
              ? crypto.subtle.importKey("raw", new Uint8Array(16), "AES-CBC", true, ["encrypt"])
                  .then((key) => rejection(crypto.subtle.encrypt(
                    { name: "AES-CBC", iv: new Uint8Array(shared) },
                    key,
                    bytes
                  )))
              : Promise.resolve("unsupported"),
            shared
              ? rejection(crypto.subtle.digest("SHA-256", shared))
              : Promise.resolve("unsupported"),
            shared
              ? rejection(crypto.subtle.digest("SHA-256", new Uint8Array(shared)))
              : Promise.resolve("unsupported"),
            shared
              ? rejection(crypto.subtle.importKey(
                  "raw",
                  new Uint8Array(shared),
                  "AES-CBC",
                  true,
                  ["encrypt"]
                ))
              : Promise.resolve("unsupported")
          ]).then((values) => {
            globalThis.__cryptoBufferSourceProbe = values;
          });
        })()
        "#,
    )
    .expect("crypto subtle BufferSource probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoBufferSourceProbe)")
        .expect("crypto subtle BufferSource promise chain should settle");

    assert_eq!(
        result,
        r#"[32,20,48,"TypeError","TypeError","TypeError","64:true","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
}
#[test]
fn crypto_subtle_shared_array_buffer_operation_parameters_reject() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoSharedArrayBufferOperationProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const shared = new WebAssembly.Memory({shared:true, initial:1, maximum:1}).buffer.slice(0,32);
            const sharedView = () => new Uint8Array(shared);
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const data = new Uint8Array([1, 2, 3]);
            const raw128 = new Uint8Array([
              0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6,
              0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c
            ]);
            const raw256 = new Uint8Array([
              0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe,
              0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d, 0x77, 0x81,
              0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7,
              0x2d, 0x98, 0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4
            ]);
            const hmac = await subtle.importKey(
              "raw",
              new Uint8Array([1, 2, 3, 4]),
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign", "verify"]
            );
            const signature = await subtle.sign("HMAC", hmac, data);
            const aesGcm = await subtle.importKey(
              "raw",
              raw128,
              "AES-GCM",
              true,
              ["encrypt", "decrypt"]
            );
            const aesKw = await subtle.importKey(
              "raw",
              raw128,
              "AES-KW",
              false,
              ["unwrapKey"]
            );
            const chacha = await subtle.importKey(
              "raw-secret",
              raw256,
              "ChaCha20-Poly1305",
              true,
              ["encrypt", "decrypt"]
            );
            const rsaOaep = await subtle.generateKey(
              {
                name: "RSA-OAEP",
                modulusLength: 1024,
                publicExponent: new Uint8Array([1, 0, 1]),
                hash: "SHA-256"
              },
              true,
              ["encrypt", "decrypt"]
            );

            globalThis.__cryptoSharedArrayBufferOperationProbe = [
              await rejectionName(subtle.sign("HMAC", hmac, sharedView())),
              await rejectionName(subtle.verify("HMAC", hmac, sharedView(), data)),
              await rejectionName(subtle.verify("HMAC", hmac, signature, sharedView())),
              await rejectionName(subtle.encrypt(
                { name: "AES-GCM", iv: new Uint8Array(shared, 0, 12) },
                aesGcm,
                data
              )),
              await rejectionName(subtle.encrypt(
                { name: "AES-GCM", iv: new Uint8Array(12), additionalData: sharedView() },
                aesGcm,
                data
              )),
              await rejectionName(subtle.encrypt(
                { name: "AES-GCM", iv: new Uint8Array(12) },
                aesGcm,
                sharedView()
              )),
              await rejectionName(subtle.encrypt(
                { name: "ChaCha20-Poly1305", iv: new Uint8Array(shared, 0, 12) },
                chacha,
                data
              )),
              await rejectionName(subtle.encrypt(
                { name: "ChaCha20-Poly1305", iv: new Uint8Array(12), additionalData: sharedView() },
                chacha,
                data
              )),
              await rejectionName(subtle.encrypt(
                { name: "ChaCha20-Poly1305", iv: new Uint8Array(12) },
                chacha,
                sharedView()
              )),
              await rejectionName(subtle.unwrapKey(
                "raw",
                sharedView(),
                aesKw,
                "AES-KW",
                { name: "AES-GCM", length: 128 },
                true,
                ["encrypt"]
              )),
              await rejectionName(subtle.encrypt(
                { name: "RSA-OAEP", label: sharedView() },
                rsaOaep.publicKey,
                data
              ))
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle SharedArrayBuffer operation parameter probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoSharedArrayBufferOperationProbe)")
        .expect("crypto subtle SharedArrayBuffer operation parameter promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
}
#[test]
fn crypto_subtle_import_key_copies_key_data_before_algorithm_normalization() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoImportKeyOrderFailures = ["pending"];
          (async () => {
            const failures = [];
            const subtle = crypto.subtle;
            const sameBytes = (left, right) => {
              const a = new Uint8Array(left);
              return a.length === right.length && a.every((value, index) => value === right[index]);
            };
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            // Chromium: crypto/subtle/modify-importKey-data-during-normalization.html.
            // BufferSource keyData is copied before algorithm normalization, so
            // side effects from algorithm.name must not alter the imported key.
            const original = new Uint8Array([
              0x30, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
              0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ]);
            const keyData = new Uint8Array(original);
            const mutatingAlgorithm = {
              get name() {
                keyData[0] = 0;
                keyData[1] = 0;
                return "aes-cbc";
              }
            };
            const imported = await subtle.importKey(
              "raw",
              keyData,
              mutatingAlgorithm,
              true,
              ["encrypt", "decrypt"]
            );
            if (!sameBytes(await subtle.exportKey("raw", imported), original)) {
              failures.push("raw-bytes-mutated-during-normalization");
            }

            const mutated = new Uint8Array(original);
            mutated[0] = 0;
            mutated[1] = 0;
            const importedAfterMutation = await subtle.importKey(
              "raw",
              keyData,
              "AES-CBC",
              true,
              ["encrypt", "decrypt"]
            );
            if (!sameBytes(await subtle.exportKey("raw", importedAfterMutation), mutated)) {
              failures.push("raw-bytes-not-read-on-second-import");
            }

            // Chromium importKey parses JsonWebKey dictionary members before
            // algorithm normalization. Getter side effects from algorithm.name
            // must not change the already-copied JWK key material.
            const jwkEvents = [];
            let jwkKeyMaterial = "MBEiM0RVZneImaq7zN3u_w";
            const jwkKeyData = {
              get kty() {
                jwkEvents.push("jwk-kty");
                return "oct";
              },
              get k() {
                jwkEvents.push("jwk-k");
                return jwkKeyMaterial;
              }
            };
            const jwkAlgorithm = {
              get name() {
                jwkEvents.push("alg-name");
                jwkKeyMaterial = "AAAAAAAAAAAAAAAAAAAAAA";
                return "AES-CBC";
              }
            };
            const importedJwk = await subtle.importKey(
              "jwk",
              jwkKeyData,
              jwkAlgorithm,
              true,
              ["encrypt", "decrypt"]
            );
            if (
              !sameBytes(await subtle.exportKey("raw", importedJwk), original) ||
              jwkEvents.indexOf("alg-name") < jwkEvents.indexOf("jwk-k") ||
              jwkEvents.indexOf("alg-name") < jwkEvents.indexOf("jwk-kty")
            ) {
              failures.push("jwk-not-copied-before-normalization:" + jwkEvents.join(","));
            }

            // Chromium: crypto/subtle/importKey-badParameters.html. The format
            // gate happens before usage validation, and invalid raw keyData is
            // rejected before algorithm.name is observed.
            const invalidFormat = await rejectionName(subtle.importKey(
              "invalid format",
              new Uint8Array(16),
              { name: "AES-CBC" },
              true,
              ["ENCRYPT"]
            ));
            if (invalidFormat !== "TypeError") {
              failures.push("invalid-format-order:" + invalidFormat);
            }

            const invalidUsage = await rejectionName(subtle.importKey(
              "raw",
              new Uint8Array(16),
              { name: "AES-CBC" },
              true,
              ["ENCRYPT"]
            ));
            if (invalidUsage !== "TypeError") {
              failures.push("invalid-key-usage:" + invalidUsage);
            }

            let invalidUsageAlgorithmGetterObserved = false;
            const invalidUsageBeforeAlgorithm = await rejectionName(subtle.importKey(
              "raw",
              new Uint8Array(16),
              { get name() { invalidUsageAlgorithmGetterObserved = true; return "AES-CBC"; } },
              true,
              ["ENCRYPT"]
            ));
            if (invalidUsageBeforeAlgorithm !== "TypeError" || invalidUsageAlgorithmGetterObserved) {
              failures.push("invalid-usage-order:" + invalidUsageBeforeAlgorithm + ":" + invalidUsageAlgorithmGetterObserved);
            }

            let algorithmGetterObserved = false;
            const invalidData = await rejectionName(subtle.importKey(
              "raw",
              [],
              { get name() { algorithmGetterObserved = true; return "AES-CBC"; } },
              true,
              ["encrypt"]
            ));
            if (invalidData !== "TypeError" || algorithmGetterObserved) {
              failures.push("invalid-key-data-order:" + invalidData + ":" + algorithmGetterObserved);
            }

            globalThis.__cryptoImportKeyOrderFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle importKey ordering probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoImportKeyOrderFailures)")
        .expect("crypto subtle importKey ordering promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_import_key_format_errors_match_chromium_backend_order() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoImportKeyFormatOrderFailures = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const failures = [];
            const keyBytes = new Uint8Array(16);
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            // Chromium's AES/HMAC/KDF ImportKey backends switch on KeyFormat
            // before entering key-creation usage checks. Unsupported formats
            // must therefore remain NotSupportedError even when usages or KDF
            // extractability would also be invalid.
            const errors = await Promise.all([
              rejectionName(subtle.importKey("spki", keyBytes, "AES-GCM", true, ["sign"])),
              rejectionName(subtle.importKey("pkcs8", keyBytes, "AES-KW", true, ["encrypt"])),
              rejectionName(subtle.importKey(
                "spki",
                keyBytes,
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.importKey(
                "pkcs8",
                keyBytes,
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.importKey("jwk", { kty: "HKDF" }, "HKDF", true, ["encrypt"])),
              rejectionName(subtle.importKey("spki", keyBytes, "HKDF", true, ["encrypt"])),
              rejectionName(subtle.importKey("pkcs8", keyBytes, "PBKDF2", true, ["encrypt"])),
              rejectionName(subtle.importKey("raw", keyBytes, "HKDF", true, ["deriveBits"])),
              rejectionName(subtle.importKey("raw", keyBytes, "PBKDF2", false, ["encrypt"]))
            ]);
            const expected = [
              "NotSupportedError",
              "NotSupportedError",
              "NotSupportedError",
              "NotSupportedError",
              "NotSupportedError",
              "NotSupportedError",
              "NotSupportedError",
              "SyntaxError",
              "SyntaxError"
            ];
            if (errors.join(",") !== expected.join(",")) {
              failures.push("format-order:" + errors.join(","));
            }
            globalThis.__cryptoImportKeyFormatOrderFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle importKey format ordering probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoImportKeyFormatOrderFailures)")
        .expect("crypto subtle importKey format ordering promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_algorithm_normalization_survives_child_context_detach() {
    let mut vm = new_parsed_test_vm(
        "https://webcrypto-context-detach.test/",
        "<!doctype html><html><body></body></html>",
    );

    vm.eval(
        r#"
            (() => {
              const makeFrame = (label, body) => {
                const frame = document.createElement("iframe");
                frame.srcdoc = `<!doctype html><script>
                  function closeOnAccess(object, key) {
                    const value = object[key];
                    Object.defineProperty(object, key, {
                      get() {
                        parent.document.body.removeChild(parent.__cryptoDetachFrame);
                        return value;
                      }
                    });
                  }
                  async function run() {
                    ${body}
                  }
                <\/script>`;
                document.body.appendChild(frame);
                return { label, frame };
              };

              globalThis.__cryptoDetachFrames = [
                makeFrame("digest", `
                  const algorithm = { name: "SHA-256" };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.digest(algorithm, new Uint8Array());
                `),
                makeFrame("generateKey", `
                  const algorithm = { name: "AES-GCM", length: 128 };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.generateKey(algorithm, true, ["encrypt"]);
                `),
                makeFrame("importKey", `
                  const algorithm = { name: "AES-GCM" };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.importKey(
                    "raw",
                    new Uint8Array(16),
                    algorithm,
                    true,
                    ["encrypt"]
                  );
                `),
                makeFrame("sign", `
                  const key = await crypto.subtle.importKey(
                    "raw",
                    new Uint8Array([1, 2, 3, 4]),
                    { name: "HMAC", hash: "SHA-256" },
                    true,
                    ["sign"]
                  );
                  const algorithm = { name: "HMAC" };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.sign(algorithm, key, new Uint8Array([5]));
                `),
                makeFrame("verify", `
                  const key = await crypto.subtle.importKey(
                    "raw",
                    new Uint8Array([1, 2, 3, 4]),
                    { name: "HMAC", hash: "SHA-256" },
                    true,
                    ["sign", "verify"]
                  );
                  const data = new Uint8Array([5]);
                  const signature = await crypto.subtle.sign("HMAC", key, data);
                  const algorithm = { name: "HMAC" };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.verify(algorithm, key, signature, data);
                `),
                makeFrame("deriveBits", `
                  const baseKey = await crypto.subtle.importKey(
                    "raw",
                    new Uint8Array([1, 2, 3, 4]),
                    "HKDF",
                    false,
                    ["deriveBits"]
                  );
                  const algorithm = {
                    name: "HKDF",
                    hash: "SHA-256",
                    salt: new Uint8Array([9]),
                    info: new Uint8Array([10])
                  };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.deriveBits(algorithm, baseKey, 128);
                `),
                makeFrame("deriveKey", `
                  const baseKey = await crypto.subtle.importKey(
                    "raw",
                    new Uint8Array([1, 2, 3, 4]),
                    "HKDF",
                    false,
                    ["deriveKey"]
                  );
                  const algorithm = {
                    name: "HKDF",
                    hash: "SHA-256",
                    salt: new Uint8Array([9]),
                    info: new Uint8Array([10])
                  };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.deriveKey(
                    algorithm,
                    baseKey,
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["encrypt"]
                  );
                `),
                makeFrame("deriveKeyTarget", `
                  const baseKey = await crypto.subtle.importKey(
                    "raw",
                    new Uint8Array([1, 2, 3, 4]),
                    "HKDF",
                    false,
                    ["deriveKey"]
                  );
                  const derivedAlgorithm = { name: "AES-GCM", length: 128 };
                  closeOnAccess(derivedAlgorithm, "name");
                  return crypto.subtle.deriveKey(
                    {
                      name: "HKDF",
                      hash: "SHA-256",
                      salt: new Uint8Array([9]),
                      info: new Uint8Array([10])
                    },
                    baseKey,
                    derivedAlgorithm,
                    true,
                    ["encrypt"]
                  );
                `),
                makeFrame("encrypt", `
                  const key = await crypto.subtle.generateKey(
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["encrypt"]
                  );
                  const algorithm = { name: "AES-GCM", iv: new Uint8Array(12) };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.encrypt(algorithm, key, new Uint8Array());
                `),
                makeFrame("decrypt", `
                  const key = await crypto.subtle.generateKey(
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["decrypt"]
                  );
                  const algorithm = { name: "AES-GCM", iv: new Uint8Array(12) };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.decrypt(algorithm, key, new Uint8Array());
                `),
                makeFrame("wrapKey", `
                  const key = await crypto.subtle.importKey(
                    "raw",
                    new Uint8Array([1, 2, 3, 4]),
                    { name: "HMAC", hash: "SHA-256" },
                    true,
                    ["sign"]
                  );
                  const wrappingKey = await crypto.subtle.generateKey(
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["wrapKey"]
                  );
                  const algorithm = { name: "AES-GCM", iv: new Uint8Array(12) };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.wrapKey("raw", key, wrappingKey, algorithm);
                `),
                makeFrame("unwrapKey", `
                  const unwrappingKey = await crypto.subtle.generateKey(
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["unwrapKey"]
                  );
                  const algorithm = { name: "AES-GCM", iv: new Uint8Array(12) };
                  closeOnAccess(algorithm, "name");
                  return crypto.subtle.unwrapKey(
                    "raw",
                    new Uint8Array(),
                    unwrappingKey,
                    algorithm,
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["encrypt"]
                  );
                `),
                makeFrame("unwrapKeyTarget", `
                  const unwrappingKey = await crypto.subtle.generateKey(
                    { name: "AES-GCM", length: 128 },
                    true,
                    ["unwrapKey"]
                  );
                  const keyAlgorithm = { name: "AES-GCM", length: 128 };
                  closeOnAccess(keyAlgorithm, "name");
                  return crypto.subtle.unwrapKey(
                    "raw",
                    new Uint8Array(),
                    unwrappingKey,
                    { name: "AES-GCM", iv: new Uint8Array(12) },
                    keyAlgorithm,
                    true,
                    ["encrypt"]
                  );
                `)
              ];
            })()
            "#,
    )
    .expect("crypto subtle detached context normalization setup should evaluate");
    let ready_count_expression = "String(globalThis.__cryptoDetachFrames.filter(({ frame }) => typeof frame.contentWindow.run === 'function').length)";
    for _ in 0..128 {
        if vm
            .eval(ready_count_expression)
            .expect("WebCrypto child-realm readiness should evaluate")
            == "13"
        {
            break;
        }
        let progressed = crate::script_vm::expect_ready_child_frame_owner_source_future_for_test(
            vm.run_next_child_frame_semantic_turn_for_test(),
        )
        .is_some();
        assert!(
            progressed,
            "WebCrypto child realms should have a runnable production owner source"
        );
    }
    assert_eq!(
        vm.eval(ready_count_expression)
            .expect("final WebCrypto child-realm readiness should evaluate"),
        "13",
        "all WebCrypto context-detach child realms should materialize"
    );

    vm.eval(
        r#"
            (() => {
              globalThis.__cryptoDetachResults = ["pending"];
              const runFrame = async ({ label, frame }) => {
                globalThis.__cryptoDetachFrame = frame;
                const child = frame.contentWindow;
                let outcome = "not-called";
                try {
                  const value = child.run();
                  if (value && typeof value.then === "function") {
                    outcome = "promise";
                    await value.catch(() => {});
                  } else {
                    outcome = String(value);
                  }
                } catch (error) {
                  outcome = error.name;
                }
                const detached = frame.contentWindow === null;
                delete globalThis.__cryptoDetachFrame;
                if (frame.parentNode) {
                  frame.remove();
                }
                return `${label}:${detached}:${outcome}`;
              };

              // Chromium WPT: WebCryptoAPI/algorithm-discards-context. The
              // algorithm.name getter can detach its originating child context;
              // operations must keep using the already converted values rather
              // than synchronously throwing while normalization continues.
              (async () => {
                const results = [];
                for (const frame of globalThis.__cryptoDetachFrames) {
                  results.push(await runFrame(frame));
                }
                globalThis.__cryptoDetachResults = results;
              })();
            })()
            "#,
    )
    .expect("crypto subtle detached context normalization probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoDetachResults)")
        .expect("crypto subtle detached context normalization promise chain should settle");

    assert_eq!(
        result,
        r#"["digest:true:promise","generateKey:true:promise","importKey:true:promise","sign:true:promise","verify:true:promise","deriveBits:true:promise","deriveKey:true:promise","deriveKeyTarget:true:promise","encrypt:true:promise","decrypt:true:promise","wrapKey:true:promise","unwrapKey:true:promise","unwrapKeyTarget:true:promise"]"#
    );
}
#[test]
fn crypto_subtle_import_key_unsupported_algorithm_promise_settles() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoImportKeyUnsupportedPromiseFailures = ["pending"];
          (async () => {
            const failures = [];
            const outcomes = [];
            let settled = false;

            // Chromium WPT crashtest:
            // WebCryptoAPI/import_export/crashtests/importKey-unsettled-promise.https.any.js.
            // The upstream test guards against an unsettled rejected promise
            // after unsupported JWK algorithm normalization. Locally we assert
            // the promise reaches the NotSupportedError rejection path after
            // microtasks run.
            crypto.subtle.importKey(
              "jwk",
              {},
              { name: "UNSUPPORTED", hash: "SHA-224" },
              true,
              []
            ).then(
              () => {
                settled = true;
                outcomes.push("resolved");
              },
              (error) => {
                settled = true;
                outcomes.push(error.name);
              }
            );

            await Promise.resolve();
            await Promise.resolve();
            if (!settled || outcomes.join(",") !== "NotSupportedError") {
              failures.push(`unsupported-importKey:${settled}:${outcomes.join(",")}`);
            }

            globalThis.__cryptoImportKeyUnsupportedPromiseFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle unsupported importKey promise probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoImportKeyUnsupportedPromiseFailures)")
        .expect("crypto subtle unsupported importKey promise probe should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_algorithm_names_are_case_insensitive_but_not_trimmed() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoAlgorithmWhitespaceProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const data = new Uint8Array([1, 2, 3, 4]);
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            const hmac = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign", "verify"]
            );
            const hkdf = await subtle.importKey(
              "raw",
              data,
              "HKDF",
              false,
              ["deriveBits"]
            );
            const digestLength = new Uint8Array(
              await subtle.digest("sHa-256", data)
            ).byteLength;

            // Chromium's WebCrypto algorithm lookup is length-exact after
            // ASCII-case folding, so surrounding whitespace is not accepted.
            // Reference: third_party/blink/renderer/modules/crypto/
            // normalize_algorithm.cc LookupAlgorithmIdByName().
            globalThis.__cryptoAlgorithmWhitespaceProbe = [
              String(digestLength),
              await rejectionName(subtle.digest(" SHA-256", data)),
              await rejectionName(subtle.digest("SHA-256 ", data)),
              await rejectionName(subtle.generateKey(
                { name: " HMAC", hash: "SHA-256" },
                true,
                ["sign"]
              )),
              await rejectionName(subtle.generateKey(
                { name: "HMAC", hash: " SHA-256" },
                true,
                ["sign"]
              )),
              await rejectionName(subtle.importKey(
                "raw",
                new Uint8Array(16),
                { name: "AES-CBC " },
                true,
                ["encrypt"]
              )),
              await rejectionName(subtle.sign({ name: " HMAC " }, hmac, data)),
              await rejectionName(subtle.deriveBits(
                { name: " HKDF", salt: new Uint8Array(), info: new Uint8Array(), hash: "SHA-256" },
                hkdf,
                128
              )),
              await rejectionName(subtle.deriveBits(
                { name: "HKDF", salt: new Uint8Array(), info: new Uint8Array(), hash: " SHA-256 " },
                hkdf,
                128
              )),
              String(SubtleCrypto.supports("digest", " SHA-256")),
              String(SubtleCrypto.supports(
                "generateKey",
                { name: " AES-GCM", length: 128 }
              )),
              String(SubtleCrypto.supports(
                "deriveBits",
                { name: "HKDF", salt: new Uint8Array(), info: new Uint8Array(), hash: " SHA-256" },
                128
              ))
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle algorithm whitespace probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoAlgorithmWhitespaceProbe)")
        .expect("crypto subtle algorithm whitespace promise chain should settle");

    assert_eq!(
        result,
        r#"["32","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","NotSupportedError","false","false","false"]"#
    );
}
#[test]
fn crypto_subtle_algorithm_identifier_strings_match_chromium_legacy() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoAlgorithmStringProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const hex = (buffer) => Array.from(new Uint8Array(buffer))
              .map((byte) => byte.toString(16).padStart(2, "0"))
              .join("");

            // Chromium legacy test:
            // crypto/subtle/algorithm-identifier-as-string.html.
            const aesKey = await subtle.importKey(
              "raw",
              new Uint8Array(16),
              "aes-cbc",
              true,
              ["encrypt"]
            );
            const digest = await subtle.digest("sha-1", new Uint8Array());
            const hmacKey = await subtle.importKey(
              "raw",
              new Uint8Array(15),
              { name: "hmac", hash: "sha-256" },
              false,
              ["sign"]
            );
            globalThis.__cryptoAlgorithmStringProbe = [
              [aesKey.type, aesKey.algorithm.name, aesKey.algorithm.length].join(":"),
              hex(digest),
              [
                hmacKey.type,
                hmacKey.algorithm.name,
                hmacKey.algorithm.hash.name,
                hmacKey.algorithm.length
              ].join(":")
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle algorithm string probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoAlgorithmStringProbe)")
        .expect("crypto subtle algorithm string promise chain should settle");

    assert_eq!(
        result,
        r#"["secret:AES-CBC:128","da39a3ee5e6b4b0d3255bfef95601890afd80709","secret:HMAC:SHA-256:120"]"#
    );
}
#[test]
fn crypto_subtle_key_format_arguments_match_chromium_errors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKeyFormatArgumentProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const keyData = new Uint8Array(16);
            const rejectionName = promise => promise.then(
              () => "resolved",
              error => error.name
            );
            const key = await subtle.importKey(
              "raw",
              keyData,
              "AES-CBC",
              true,
              ["encrypt", "decrypt"]
            );

            // Chromium legacy tests:
            // crypto/subtle/importKey-badParameters.html,
            // crypto/subtle/exportKey-badParameters.html, and
            // crypto/subtle/aes-export-key.html.
            // KeyFormat is enum-like at the API boundary: unsupported format
            // values reject with TypeError before later usage validation.
            globalThis.__cryptoKeyFormatArgumentProbe = await Promise.all([
              rejectionName(subtle.importKey(
                "invalid format",
                keyData,
                "AES-CBC",
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.importKey(
                "invalid format",
                keyData,
                "AES-CBC",
                true,
                ["ENCRYPT"]
              )),
              rejectionName(subtle.importKey(
                "raw",
                new Uint8Array(20),
                { name: "HMAC" },
                true,
                ["sign"]
              )),
              rejectionName(subtle.importKey(3, keyData, "AES-CBC", true, ["encrypt"])),
              rejectionName(subtle.importKey(null, keyData, "AES-CBC", true, ["encrypt"])),
              rejectionName(subtle.importKey({}, keyData, "AES-CBC", true, ["encrypt"])),
              rejectionName(subtle.importKey("", keyData, "AES-CBC", true, ["encrypt"])),
              rejectionName(subtle.exportKey("raw")),
              rejectionName(subtle.exportKey("raw", null)),
              rejectionName(subtle.exportKey("raw", undefined)),
              rejectionName(subtle.exportKey("raw", {})),
              rejectionName(subtle.exportKey("raw", 1)),
              rejectionName(subtle.exportKey(3, key)),
              rejectionName(subtle.exportKey(null, key)),
              rejectionName(subtle.exportKey({}, key)),
              rejectionName(subtle.exportKey("", key)),
              rejectionName(subtle.exportKey("foobar", key))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle key format argument probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKeyFormatArgumentProbe)")
        .expect("crypto subtle key format argument promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
}
#[test]
fn crypto_subtle_modern_raw_key_formats_match_chromium_boundaries() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoModernRawKeyFormatProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const sameBytes = (left, right) => {
              const a = new Uint8Array(left);
              const b = new Uint8Array(right);
              return a.length === b.length && a.every((value, index) => value === b[index]);
            };
            const rejectionName = promise => promise.then(
              () => "resolved",
              error => error.name
            );
            const aesBytes = new Uint8Array([
              0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15
            ]);
            const hmacBytes = new Uint8Array([
              16, 17, 18, 19, 20, 21, 22, 23,
              24, 25, 26, 27, 28, 29, 30, 31
            ]);
            const publicRaw = new Uint8Array([
              28, 242, 177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250, 17,
              84, 216, 62, 152, 235, 100, 234, 81, 250, 229, 179, 48, 124, 254, 151, 6
            ]);

            // Chromium treats modern raw-* KeyFormat values as enum-valid at
            // the binding boundary. Existing secret-key algorithms accept
            // raw-secret as raw key bytes; X25519 accepts raw-public as raw
            // public key bytes.
            const aesRawSecret = await subtle.importKey(
              "raw-secret",
              aesBytes,
              "AES-GCM",
              true,
              ["encrypt"]
            );
            const hmacRawSecret = await subtle.importKey(
              "raw-secret",
              hmacBytes,
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign"]
            );
            const hkdfRawSecret = await subtle.importKey(
              "raw-secret",
              new Uint8Array([1, 2, 3]),
              "HKDF",
              false,
              ["deriveBits"]
            );
            const pbkdf2RawSecret = await subtle.importKey(
              "raw-secret",
              new Uint8Array([4, 5, 6]),
              "PBKDF2",
              false,
              ["deriveBits"]
            );
            const x25519RawPublic = await subtle.importKey(
              "raw-public",
              publicRaw,
              "X25519",
              true,
              []
            );
            const ecdsaPair = await subtle.generateKey(
              { name: "ECDSA", namedCurve: "P-256" },
              true,
              ["sign", "verify"]
            );
            const ed25519Pair = await subtle.generateKey(
              "Ed25519",
              true,
              ["sign", "verify"]
            );
            const rsaPair = await subtle.generateKey(
              {
                name: "RSA-PSS",
                modulusLength: 1024,
                publicExponent: new Uint8Array([1, 0, 1]),
                hash: "SHA-256"
              },
              true,
              ["sign", "verify"]
            );

            const aesExport = await subtle.exportKey("raw-secret", aesRawSecret);
            const hmacExport = await subtle.exportKey("raw-secret", hmacRawSecret);
            const x25519Export = await subtle.exportKey("raw-public", x25519RawPublic);
            const unsupported = await Promise.all([
              rejectionName(subtle.importKey("raw-public", aesBytes, "AES-GCM", true, ["encrypt"])),
              rejectionName(subtle.importKey("raw-private", publicRaw, "X25519", true, ["deriveBits"])),
              rejectionName(subtle.importKey("raw-seed", publicRaw, "X25519", true, ["deriveBits"])),
              rejectionName(subtle.importKey("raw-secret", publicRaw, "X25519", true, ["deriveBits"])),
              rejectionName(subtle.importKey("raw-private", publicRaw, { name: "ECDSA", namedCurve: "P-256" }, true, ["sign"])),
              rejectionName(subtle.importKey("raw-seed", publicRaw, { name: "ECDSA", namedCurve: "P-256" }, true, ["sign"])),
              rejectionName(subtle.importKey("raw-private", publicRaw, "Ed25519", true, ["sign"])),
              rejectionName(subtle.importKey("raw-seed", publicRaw, "Ed25519", true, ["sign"])),
              rejectionName(subtle.importKey("raw-private", publicRaw, { name: "RSA-PSS", hash: "SHA-256" }, true, ["sign"])),
              rejectionName(subtle.importKey("raw-seed", publicRaw, { name: "RSA-PSS", hash: "SHA-256" }, true, ["sign"])),
              rejectionName(subtle.exportKey("raw-private", x25519RawPublic)),
              rejectionName(subtle.exportKey("raw-seed", x25519RawPublic)),
              rejectionName(subtle.exportKey("raw-secret", x25519RawPublic)),
              rejectionName(subtle.exportKey("raw-private", ecdsaPair.privateKey)),
              rejectionName(subtle.exportKey("raw-seed", ecdsaPair.privateKey)),
              rejectionName(subtle.exportKey("raw-private", ed25519Pair.privateKey)),
              rejectionName(subtle.exportKey("raw-seed", ed25519Pair.privateKey)),
              rejectionName(subtle.exportKey("raw-private", rsaPair.privateKey)),
              rejectionName(subtle.exportKey("raw-seed", rsaPair.privateKey))
            ]);

            globalThis.__cryptoModernRawKeyFormatProbe = [
              `${aesRawSecret.type}:${aesRawSecret.algorithm.name}:${aesRawSecret.algorithm.length}`,
              String(sameBytes(aesBytes, aesExport)),
              `${hmacRawSecret.type}:${hmacRawSecret.algorithm.name}:${hmacRawSecret.algorithm.hash.name}:${hmacRawSecret.algorithm.length}`,
              String(sameBytes(hmacBytes, hmacExport)),
              `${hkdfRawSecret.type}:${hkdfRawSecret.algorithm.name}:${hkdfRawSecret.extractable}:${hkdfRawSecret.usages.join(",")}`,
              `${pbkdf2RawSecret.type}:${pbkdf2RawSecret.algorithm.name}:${pbkdf2RawSecret.extractable}:${pbkdf2RawSecret.usages.join(",")}`,
              `${x25519RawPublic.type}:${x25519RawPublic.algorithm.name}:${sameBytes(publicRaw, x25519Export)}`,
              unsupported.join(",")
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle modern raw key format probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoModernRawKeyFormatProbe)")
        .expect("crypto subtle modern raw key format promise chain should settle");

    assert_eq!(
        result,
        r#"["secret:AES-GCM:128","true","secret:HMAC:SHA-256:128","true","secret:HKDF:false:deriveBits","secret:PBKDF2:false:deriveBits","public:X25519:true","NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError,NotSupportedError"]"#
    );
}
#[test]
fn crypto_subtle_export_unextractable_keys_match_chromium() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoExportUnextractableProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const aesImported = await subtle.importKey(
              "raw",
              new Uint8Array(16),
              { name: "aes-cbc" },
              false,
              ["encrypt"]
            );
            const aesGenerated = await subtle.generateKey(
              { name: "AES-GCM", length: 128 },
              false,
              ["encrypt"]
            );
            const hmacImported = await subtle.importKey(
              "raw",
              new Uint8Array(16),
              { name: "HMAC", hash: { name: "sha-1" } },
              false,
              ["sign"]
            );
            const hmacGenerated = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign"]
            );
            const x25519 = await subtle.generateKey(
              "X25519",
              false,
              ["deriveBits"]
            );

            // Chromium legacy test:
            // crypto/subtle/exportKey-unextractable.html. Chromium's original
            // private-key case is RSA; X25519 exercises the same supported
            // private-key extractability boundary in Moli's current
            // WebCrypto scope.
            globalThis.__cryptoExportUnextractableProbe = await Promise.all([
              rejectionName(subtle.exportKey("raw", aesImported)),
              rejectionName(subtle.exportKey("jwk", aesImported)),
              rejectionName(subtle.exportKey("raw", aesGenerated)),
              rejectionName(subtle.exportKey("jwk", aesGenerated)),
              rejectionName(subtle.exportKey("raw", hmacImported)),
              rejectionName(subtle.exportKey("jwk", hmacImported)),
              rejectionName(subtle.exportKey("raw", hmacGenerated)),
              rejectionName(subtle.exportKey("jwk", hmacGenerated)),
              rejectionName(subtle.exportKey("pkcs8", x25519.privateKey)),
              rejectionName(subtle.exportKey("jwk", x25519.privateKey))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle unextractable export probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoExportUnextractableProbe)")
        .expect("crypto subtle unextractable export promise chain should settle");

    assert_eq!(
        result,
        r#"["InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError","InvalidAccessError"]"#
    );
}
#[test]
fn crypto_subtle_scalar_and_sequence_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoScalarProbe = [];
          const rejection = (promise) => promise.then(
            () => "resolved",
            (error) => error.message
          );
          Promise.all([
            crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              undefined,
              new Set(["sign", "verify"])
            ).then((key) => [
              String(key.extractable),
              key.usages.join(",")
            ].join(":")),
            rejection(crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" }
            )),
            rejection(crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              undefined
            )),
            rejection(crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              [Symbol("sign")]
            )),
            crypto.subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign"]
            ).then((key) => Promise.all([
              crypto.subtle.exportKey(new String("raw"), key)
                .then((value) => new Uint8Array(value).length),
              rejection(crypto.subtle.exportKey(Symbol("raw"), key)),
              rejection(crypto.subtle.exportKey(undefined, key))
            ]).then((values) => values.join(":")))
          ]).then((values) => {
            globalThis.__cryptoScalarProbe = values;
          });
        })()
        "#,
    )
    .expect("crypto subtle scalar probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoScalarProbe)")
        .expect("crypto subtle scalar promise chain should settle");

    assert_eq!(
        result,
        r#"["false:sign,verify","TypeError","TypeError","TypeError","64:TypeError:TypeError"]"#
    );
}
#[test]
fn crypto_subtle_generate_key_error_order_matches_wpt_failures() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoGenerateKeyFailureOrder = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            // Chromium WPT: WebCryptoAPI/generateKey/failures.js.
            // Illegal non-empty usages are reported before algorithm
            // properties; empty usages are checked after algorithm properties.
            let invalidUsageGetterObserved = false;
            const invalidUsageBeforeAlgorithm = await rejectionName(subtle.generateKey(
              {
                get name() { invalidUsageGetterObserved = true; return "AES-CBC"; },
                length: 128
              },
              true,
              ["ENCRYPT"]
            ));

            let nonSequenceGetterObserved = false;
            const nonSequenceBeforeAlgorithm = await rejectionName(subtle.generateKey(
              {
                get name() { nonSequenceGetterObserved = true; return "AES-CBC"; },
                length: 128
              },
              true,
              null
            ));

            const results = [
              `${invalidUsageBeforeAlgorithm}:${invalidUsageGetterObserved}`,
              `${nonSequenceBeforeAlgorithm}:${nonSequenceGetterObserved}`,
              ...(await Promise.all([
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: 64 },
                true,
                []
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: 64 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: 128 },
                true,
                ["ENCRYPT"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: 128 },
                true,
                []
              )),
              // Chromium legacy test:
              // crypto/subtle/aes-cbc/generateKey-failures.html.
              rejectionName(subtle.generateKey(
                { name: "AES-CBC" },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: undefined },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: {} },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: 70000 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: -3 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "AES-CBC", length: -Infinity },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "HMAC", hash: "MD5" },
                true,
                []
              )),
              rejectionName(subtle.generateKey(
                { name: "HMAC", hash: "SHA-256", length: 0 },
                true,
                []
              )),
              rejectionName(subtle.generateKey(
                { name: "HMAC", hash: "SHA-256", length: 256 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.generateKey(
                { name: "RSA-PSS", hash: "SHA", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]) },
                false,
                ["decrypt"]
              )),
              rejectionName(subtle.generateKey(
                "X25519",
                true,
                []
              )),
              rejectionName(subtle.generateKey(
                "X25519",
                true,
                ["sign"]
              ))
              ]))
            ];
            globalThis.__cryptoGenerateKeyFailureOrder = results;
          })();
        })()
        "#,
    )
    .expect("crypto subtle generateKey failure ordering probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoGenerateKeyFailureOrder)")
        .expect("crypto subtle generateKey failure ordering promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError:false","TypeError:false","OperationError","SyntaxError","TypeError","SyntaxError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","NotSupportedError","OperationError","SyntaxError","NotSupportedError","SyntaxError","SyntaxError"]"#
    );
}
#[test]
fn crypto_subtle_supports_reports_current_moli_capabilities() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoSupportsProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const x25519 = await subtle.generateKey(
              "X25519",
              false,
              ["deriveBits", "deriveKey"]
            );
            const hkdfParams = {
              name: "HKDF",
              hash: "SHA-256",
              salt: new Uint8Array(16),
              info: new Uint8Array(0)
            };
            const pbkdf2Params = {
              name: "PBKDF2",
              hash: "SHA-256",
              salt: new Uint8Array(16),
              iterations: 1000
            };
            const x25519Params = {
              name: "X25519",
              public: x25519.publicKey
            };
            const x448 = await subtle.generateKey(
              "X448",
              false,
              ["deriveBits", "deriveKey"]
            );
            const x448Params = {
              name: "X448",
              public: x448.publicKey
            };
            const ecdh = await subtle.generateKey(
              { name: "ECDH", namedCurve: "P-256" },
              false,
              ["deriveBits", "deriveKey"]
            );
            const ecdhParams = {
              name: "ECDH",
              public: ecdh.publicKey
            };
            const typeErrorName = (callback) => {
              try {
                callback();
                return "none";
              } catch (error) {
                return error.name;
              }
            };

            // Chromium WPT: WebCryptoAPI/supports.tentative.https.any.js.
            // Moli reports the capabilities actually implemented by the
            // current backend. Symmetric algorithms, RSA, NIST EC, EdDSA, and
            // X25519/X448 are backed by the renderer-neutral WebCrypto crate.
            const failures = [];
            const expect = (label, actual, expected = true) => {
              if (actual !== expected) {
                failures.push(`${label}:${actual}`);
              }
            };
            const expectTypeError = (label, callback) => {
              expect(label, typeErrorName(callback), "TypeError");
            };

            expect("static method exists", typeof SubtleCrypto.supports === "function");
            expect("prototype method absent", typeof subtle.supports === "undefined");
            expect("SHA-256 digest", SubtleCrypto.supports("digest", "SHA-256"));
            expect("SHA-512 digest object", SubtleCrypto.supports("digest", { name: "SHA-512" }));
            expect("HMAC digest rejected", SubtleCrypto.supports("digest", "HMAC"), false);
            expect("SHA generateKey rejected", SubtleCrypto.supports("generateKey", "SHA-256"), false);
            expect("SHA sign rejected", SubtleCrypto.supports("sign", "SHA-256"), false);
            expect("AES-GCM generateKey", SubtleCrypto.supports("generateKey", { name: "AES-GCM", length: 256 }));
            expect("AES-GCM 192-bit generateKey", SubtleCrypto.supports("generateKey", { name: "AES-GCM", length: 192 }));
            expect("AES-GCM invalid length", SubtleCrypto.supports("generateKey", { name: "AES-GCM", length: 100 }), false);
            expect("AES-GCM string keygen rejected", SubtleCrypto.supports("generateKey", "AES-GCM"), false);
            expect("AES-CBC importKey", SubtleCrypto.supports("importKey", "AES-CBC"));
            expect("AES-KW exportKey", SubtleCrypto.supports("exportKey", "AES-KW"));
            expect("AES-GCM encrypt", SubtleCrypto.supports("encrypt", { name: "AES-GCM", iv: new Uint8Array(12) }));
            expect("AES-CBC decrypt", SubtleCrypto.supports("decrypt", { name: "AES-CBC", iv: new Uint8Array(16) }));
            expect("AES-KW wrapKey", SubtleCrypto.supports("wrapKey", "AES-KW"));
            expect("AES-GCM sign rejected", SubtleCrypto.supports("sign", "AES-GCM"), false);
            expect("AES-GCM verify rejected", SubtleCrypto.supports("verify", "AES-GCM"), false);
            expect("AES-GCM digest rejected", SubtleCrypto.supports("digest", "AES-GCM"), false);
            expect("HMAC generateKey", SubtleCrypto.supports("generateKey", { name: "HMAC", hash: "SHA-256" }));
            expect("HMAC string keygen rejected", SubtleCrypto.supports("generateKey", "HMAC"), false);
            expect("HMAC missing hash rejected", SubtleCrypto.supports("generateKey", { name: "HMAC" }), false);
            expect("HMAC invalid hash rejected", SubtleCrypto.supports("generateKey", { name: "HMAC", hash: "MD5" }), false);
            expect("HMAC undefined length rejected", SubtleCrypto.supports("generateKey", {
              name: "HMAC",
              hash: "SHA-256",
              length: undefined
            }), false);
            expect("HMAC SHA-1 short keygen", SubtleCrypto.supports("generateKey", {
              name: "HMAC",
              hash: "SHA-1",
              length: 40
            }));
            expect("HMAC too long keygen rejected", SubtleCrypto.supports("generateKey", {
              name: "HMAC",
              hash: "SHA-256",
              length: 65537
            }), false);
            expect("HMAC sign", SubtleCrypto.supports("sign", { name: "HMAC" }));
            expect("HMAC verify", SubtleCrypto.supports("verify", "HMAC"));
            expect("HMAC encrypt rejected", SubtleCrypto.supports("encrypt", "HMAC"), false);
            expect("HMAC decrypt rejected", SubtleCrypto.supports("decrypt", "HMAC"), false);
            expect("HKDF importKey", SubtleCrypto.supports("importKey", "HKDF"));
            expect("HKDF deriveBits 256", SubtleCrypto.supports("deriveBits", hkdfParams, 256));
            expect("HKDF deriveBits 0", SubtleCrypto.supports("deriveBits", hkdfParams, 0));
            expect("HKDF deriveBits max", SubtleCrypto.supports("deriveBits", hkdfParams, 65280));
            expect("HKDF deriveBits missing length", SubtleCrypto.supports("deriveBits", hkdfParams), false);
            expect("HKDF deriveBits non-byte length", SubtleCrypto.supports("deriveBits", hkdfParams, 15), false);
            expect("HKDF deriveBits too long", SubtleCrypto.supports("deriveBits", hkdfParams, 65288), false);
            expect("HKDF deriveKey AES-GCM", SubtleCrypto.supports("deriveKey", hkdfParams, { name: "AES-GCM", length: 128 }));
            expect("HKDF deriveKey AES-GCM 192-bit", SubtleCrypto.supports("deriveKey", hkdfParams, { name: "AES-GCM", length: 192 }));
            expect("HKDF deriveKey HMAC", SubtleCrypto.supports("deriveKey", hkdfParams, { name: "HMAC", hash: "SHA-256" }));
            expect("HKDF deriveKey too long HMAC", SubtleCrypto.supports("deriveKey", hkdfParams, { name: "HMAC", hash: "SHA-256", length: 65537 }), false);
            expect("HKDF deriveKey short AES rejected", SubtleCrypto.supports("deriveKey", hkdfParams, { name: "AES-GCM", length: 64 }), false);
            expect("PBKDF2 deriveBits 256", SubtleCrypto.supports("deriveBits", pbkdf2Params, 256));
            expect("PBKDF2 deriveBits 0", SubtleCrypto.supports("deriveBits", pbkdf2Params, 0));
            expect("PBKDF2 deriveBits non-byte length", SubtleCrypto.supports("deriveBits", pbkdf2Params, 44), false);
            expect("PBKDF2 deriveBits too long", SubtleCrypto.supports("deriveBits", pbkdf2Params, 8388616), false);
            expect("PBKDF2 deriveBits too many iterations", SubtleCrypto.supports(
              "deriveBits",
              { name: "PBKDF2", hash: "SHA-256", salt: new Uint8Array(16), iterations: 1000001 },
              256
            ), false);
            expect("X25519 generateKey", SubtleCrypto.supports("generateKey", "X25519"));
            expect("X25519 importKey", SubtleCrypto.supports("importKey", "X25519"));
            expect("X25519 deriveBits omitted length", SubtleCrypto.supports("deriveBits", x25519Params));
            expect("X25519 deriveBits 128", SubtleCrypto.supports("deriveBits", x25519Params, 128));
            expect("X25519 deriveBits non-byte length", SubtleCrypto.supports("deriveBits", x25519Params, 245));
            expect("X25519 deriveBits too long", SubtleCrypto.supports("deriveBits", x25519Params, 264), false);
            expect("X25519 deriveKey default HMAC rejected", SubtleCrypto.supports("deriveKey", x25519Params, { name: "HMAC", hash: "SHA-256" }), false);
            expect("X25519 deriveKey HMAC", SubtleCrypto.supports("deriveKey", x25519Params, { name: "HMAC", hash: "SHA-256", length: 256 }));
            expect("X25519 deriveKey too long HMAC", SubtleCrypto.supports("deriveKey", x25519Params, { name: "HMAC", hash: "SHA-256", length: 257 }), false);
            expect("X25519 deriveKey non-byte HMAC", SubtleCrypto.supports("deriveKey", x25519Params, { name: "HMAC", hash: "SHA-1", length: 19 }));
            expect("HKDF deriveKey non-byte HMAC rejected", SubtleCrypto.supports("deriveKey", hkdfParams, { name: "HMAC", hash: "SHA-1", length: 19 }), false);
            expect("X25519 deriveKey HKDF", SubtleCrypto.supports("deriveKey", x25519Params, "HKDF"));
            expect("HKDF deriveKey HKDF rejected", SubtleCrypto.supports("deriveKey", hkdfParams, "HKDF"), false);
            expect("X25519 getPublicKey", SubtleCrypto.supports("getPublicKey", "X25519"));
            expect("X448 generateKey", SubtleCrypto.supports("generateKey", "X448"));
            expect("X448 deriveBits omitted length", SubtleCrypto.supports("deriveBits", x448Params));
            expect("X448 deriveBits 257", SubtleCrypto.supports("deriveBits", x448Params, 257));
            expect("X448 deriveBits too long", SubtleCrypto.supports("deriveBits", x448Params, 456), false);
            expect("X448 deriveKey default HMAC rejected", SubtleCrypto.supports("deriveKey", x448Params, { name: "HMAC", hash: "SHA-256" }), false);
            expect("X448 deriveKey HMAC", SubtleCrypto.supports("deriveKey", x448Params, { name: "HMAC", hash: "SHA-256", length: 256 }));
            expect("X448 deriveKey HKDF", SubtleCrypto.supports("deriveKey", x448Params, "HKDF"));
            expect("X448 getPublicKey", SubtleCrypto.supports("getPublicKey", "X448"));
            expect("RSA-PSS string keygen rejected", SubtleCrypto.supports("generateKey", "RSA-PSS"), false);
            expect("RSA-PSS generateKey", SubtleCrypto.supports("generateKey", {
              name: "RSA-PSS",
              modulusLength: 2048,
              publicExponent: new Uint8Array([1, 0, 1]),
              hash: "SHA-256"
            }));
            expect("RSA-OAEP generateKey", SubtleCrypto.supports("generateKey", {
              name: "RSA-OAEP",
              modulusLength: 2048,
              publicExponent: new Uint8Array([1, 0, 1]),
              hash: "SHA-256"
            }));
            expect("RSA-OAEP encrypt", SubtleCrypto.supports("encrypt", { name: "RSA-OAEP", label: new Uint8Array(0) }));
            expect("RSA-OAEP wrapKey", SubtleCrypto.supports("wrapKey", { name: "RSA-OAEP" }));
            expect("RSASSA generateKey", SubtleCrypto.supports("generateKey", {
              name: "RSASSA-PKCS1-v1_5",
              modulusLength: 2048,
              publicExponent: new Uint8Array([1, 0, 1]),
              hash: "SHA-256"
            }));
            expect("RSASSA sign", SubtleCrypto.supports("sign", { name: "RSASSA-PKCS1-v1_5" }));
            expect("ECDH string keygen rejected", SubtleCrypto.supports("generateKey", "ECDH"), false);
            expect("ECDH generateKey", SubtleCrypto.supports("generateKey", { name: "ECDH", namedCurve: "P-256" }));
            expect("ECDH deriveBits omitted length", SubtleCrypto.supports("deriveBits", ecdhParams));
            expect("ECDH deriveBits 129", SubtleCrypto.supports("deriveBits", ecdhParams, 129));
            expect("ECDH deriveBits too long", SubtleCrypto.supports("deriveBits", ecdhParams, 264), false);
            expect("ECDH deriveKey HMAC", SubtleCrypto.supports("deriveKey", ecdhParams, { name: "HMAC", hash: "SHA-256", length: 256 }));
            expect("ECDH deriveKey HKDF", SubtleCrypto.supports("deriveKey", ecdhParams, "HKDF"));
            expect("ECDSA generateKey", SubtleCrypto.supports("generateKey", { name: "ECDSA", namedCurve: "P-256" }));
            expect("ECDSA sign", SubtleCrypto.supports("sign", { name: "ECDSA", hash: "SHA-256" }));
            expect("ECDSA encrypt rejected", SubtleCrypto.supports("encrypt", "ECDSA"), false);
            expect("ECDSA decrypt rejected", SubtleCrypto.supports("decrypt", "ECDSA"), false);
            expect("ECDSA digest rejected", SubtleCrypto.supports("digest", "ECDSA"), false);
            expect("Ed25519 generateKey", SubtleCrypto.supports("generateKey", "Ed25519"));
            expect("Ed25519 sign", SubtleCrypto.supports("sign", { name: "Ed25519" }));
            expect("Ed448 generateKey", SubtleCrypto.supports("generateKey", "Ed448"));
            expect("Ed448 verify", SubtleCrypto.supports("verify", { name: "Ed448" }));
            expect("invalid operation", SubtleCrypto.supports("invalidOperation", "AES-GCM"), false);
            expect("empty operation", SubtleCrypto.supports("", "AES-GCM"), false);
            expect("case-sensitive operation", SubtleCrypto.supports("GENERATEKEY", "AES-GCM"), false);
            expect("invalid algorithm", SubtleCrypto.supports("generateKey", "InvalidAlgorithm"), false);
            expect("empty algorithm", SubtleCrypto.supports("generateKey", ""), false);
            expectTypeError("missing operation", () => SubtleCrypto.supports());
            expectTypeError("missing algorithm", () => SubtleCrypto.supports("digest"));

            globalThis.__cryptoSupportsProbe = failures.length === 0 ? ["ok"] : failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle supports probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoSupportsProbe)")
        .expect("crypto subtle supports promise chain should settle");

    assert_eq!(result, r#"["ok"]"#);
}
#[test]
fn crypto_subtle_key_format_errors_follow_webcrypto_boundaries() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKeyFormatErrorProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const rejection = (promise) => promise.then(
              () => "resolved",
              (error) => `${error.name}:${error.message}:${error instanceof DOMException}`
            );
            const publicRaw = new Uint8Array([
              28, 242, 177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250, 17,
              84, 216, 62, 152, 235, 100, 234, 81, 250, 229, 179, 48, 124, 254, 151, 6
            ]);
            const pkcs8 = new Uint8Array([
              48, 46, 2, 1, 0, 48, 5, 6, 3, 43, 101, 110, 4, 34, 4, 32,
              200, 131, 142, 118, 208, 87, 223, 183, 216, 201, 90, 105, 225,
              56, 22, 10, 221, 99, 115, 253, 113, 164, 210, 118, 187, 86,
              227, 168, 27, 100, 255, 97
            ]);
            const missingX = {
              kty: "OKP",
              crv: "X25519",
              d: "yIOOdtBX37fYyVpp4TgWCt1jc_1xpNJ2u1bjqBtk_2E"
            };
            const mismatchedPair = {
              kty: "OKP",
              crv: "X25519",
              x: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
              d: "yIOOdtBX37fYyVpp4TgWCt1jc_1xpNJ2u1bjqBtk_2E"
            };

            const hmac = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign", "verify"]
            );
            const signOnly = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign"]
            );
            const aesKey = await subtle.importKey(
              "raw",
              new Uint8Array(16),
              "AES-GCM",
              true,
              ["encrypt"]
            );
            const aesKw = await subtle.importKey(
              "raw",
              new Uint8Array(16),
              "AES-KW",
              true,
              ["wrapKey"]
            );
            const publicKey = await subtle.importKey("raw", publicRaw, "X25519", true, []);
            const privateKey = await subtle.importKey(
              "pkcs8",
              pkcs8,
              "X25519",
              true,
              ["deriveBits"]
            );
            const signature = await subtle.sign("HMAC", signOnly, new Uint8Array([1, 2, 3]));

            globalThis.__cryptoKeyFormatErrorProbe = await Promise.all([
              rejection(subtle.exportKey("raw", hmac)),
              rejection(subtle.exportKey("spki", signOnly)),
              rejection(subtle.exportKey("pkcs8", signOnly)),
              rejection(subtle.exportKey("spki", aesKey)),
              rejection(subtle.exportKey("pkcs8", aesKey)),
              rejection(subtle.exportKey("raw", privateKey)),
              rejection(subtle.exportKey("pkcs8", publicKey)),
              rejection(subtle.exportKey("spki", privateKey)),
              // Chromium's wrapKey front-end checks extractability, then the
              // backend exports the target key before encryption. Unsupported
              // target export formats therefore keep exportKey's
              // NotSupportedError, while X25519 wrong-type exports stay
              // InvalidAccessError.
              rejection(subtle.wrapKey("spki", signOnly, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("pkcs8", aesKey, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("raw-private", publicKey, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("raw-secret", publicKey, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("raw-public", privateKey, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("raw", privateKey, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("spki", privateKey, aesKw, "AES-KW")),
              rejection(subtle.wrapKey("pkcs8", publicKey, aesKw, "AES-KW")),
              rejection(subtle.importKey("raw", publicRaw, "X25519", true, ["deriveBits"])),
              rejection(subtle.importKey("pkcs8", pkcs8, "X25519", true, [])),
              rejection(subtle.importKey("spki", new Uint8Array([1, 2, 3]), "X25519", true, [])),
              rejection(subtle.importKey("jwk", missingX, "X25519", true, ["deriveBits"])),
              rejection(subtle.importKey("jwk", mismatchedPair, "X25519", true, ["deriveBits"])),
              rejection(subtle.verify("HMAC", signOnly, signature, new Uint8Array([1, 2, 3]))),
              rejection(subtle.deriveBits({ name: "X25519", public: publicKey }, publicKey, 128)),
              rejection(subtle.deriveBits({ name: "X25519", public: publicKey }, privateKey, 264))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle key format error probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKeyFormatErrorProbe)")
        .expect("crypto subtle key format error promise chain should settle");

    assert_eq!(
        result,
        r#"["InvalidAccessError:InvalidAccessError:true","NotSupportedError:NotSupportedError:true","NotSupportedError:NotSupportedError:true","NotSupportedError:NotSupportedError:true","NotSupportedError:NotSupportedError:true","InvalidAccessError:InvalidAccessError:true","InvalidAccessError:InvalidAccessError:true","InvalidAccessError:InvalidAccessError:true","NotSupportedError:NotSupportedError:true","NotSupportedError:NotSupportedError:true","NotSupportedError:NotSupportedError:true","NotSupportedError:NotSupportedError:true","InvalidAccessError:InvalidAccessError:true","InvalidAccessError:InvalidAccessError:true","InvalidAccessError:InvalidAccessError:true","InvalidAccessError:InvalidAccessError:true","SyntaxError:SyntaxError:true","SyntaxError:SyntaxError:true","DataError:DataError:true","DataError:DataError:true","DataError:DataError:true","InvalidAccessError:InvalidAccessError:true","InvalidAccessError:InvalidAccessError:true","OperationError:OperationError:true"]"#
    );
}
#[test]
fn crypto_subtle_rejects_plain_objects_as_crypto_keys() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoPlainObjectKeyProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const bytes = new Uint8Array(16);
            bytes.forEach((_, index) => { bytes[index] = index + 1; });
            const data = new Uint8Array([1, 2, 3]);
            const hmac = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign", "verify"]
            );
            const aesKw = await subtle.importKey(
              "raw",
              bytes,
              "AES-KW",
              true,
              ["wrapKey", "unwrapKey"]
            );
            const hkdf = {
              name: "HKDF",
              hash: "SHA-256",
              salt: new Uint8Array([9]),
              info: new Uint8Array([8])
            };
            const x25519 = await subtle.generateKey(
              "X25519",
              true,
              ["deriveBits", "deriveKey"]
            );

            // WebIDL CryptoKey conversions reject ordinary objects before the
            // operation-specific InvalidAccessError path. These cases are
            // derived from Chromium/WPT typed-key boundaries, including
            // cfrg_curves_bits.js and cfrg_curves_keys.js public-key checks.
            globalThis.__cryptoPlainObjectKeyProbe = await Promise.all([
              rejectionName(subtle.sign("HMAC", {}, data)),
              rejectionName(subtle.verify("HMAC", {}, new Uint8Array(32), data)),
              rejectionName(subtle.getPublicKey({}, [])),
              rejectionName(subtle.exportKey("raw", {})),
              rejectionName(subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, {}, data)),
              rejectionName(subtle.wrapKey("raw", {}, aesKw, "AES-KW")),
              rejectionName(subtle.wrapKey("raw", hmac, {}, "AES-KW")),
              rejectionName(subtle.unwrapKey(
                "raw",
                new Uint8Array(16),
                {},
                "AES-KW",
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveBits(hkdf, {}, 128)),
              rejectionName(subtle.deriveKey(
                hkdf,
                {},
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveBits(
                { name: "X25519", public: {} },
                x25519.privateKey,
                128
              )),
              rejectionName(subtle.deriveBits(
                { name: "X25519", public: x25519.publicKey },
                {},
                128
              )),
              rejectionName(subtle.deriveKey(
                { name: "X25519", public: {} },
                x25519.privateKey,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "X25519", public: x25519.publicKey },
                {},
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              ))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle plain object CryptoKey probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoPlainObjectKeyProbe)")
        .expect("crypto subtle plain object CryptoKey promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
}
#[test]
fn crypto_subtle_unimplemented_operations_still_normalize_arguments() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoUnsupportedOperationProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const bytes = new Uint8Array(16);
            bytes.forEach((_, index) => { bytes[index] = index + 1; });
            const data = new Uint8Array([1, 2, 3]);

            const aesCbcEncrypt = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["encrypt"]
            );
            const aesCbcDecrypt = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["decrypt"]
            );
            const aesKw = await subtle.importKey(
              "raw",
              bytes,
              "AES-KW",
              true,
              ["wrapKey", "unwrapKey"]
            );
            const aesCbcWrap = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["wrapKey"]
            );
            const aesCbcUnwrap = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["unwrapKey"]
            );
            const aesCtrWrap = await subtle.importKey(
              "raw",
              bytes,
              "AES-CTR",
              true,
              ["wrapKey"]
            );
            const aesCtrUnwrap = await subtle.importKey(
              "raw",
              bytes,
              "AES-CTR",
              true,
              ["unwrapKey"]
            );
            const hmacExtractable = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              true,
              ["sign"]
            );
            const hmacHidden = await subtle.generateKey(
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign"]
            );
            const unwrappedAesCbcAlgorithm = { name: "AES-CBC" };

            globalThis.__cryptoUnsupportedOperationProbe = await Promise.all([
              rejectionName(subtle.encrypt()),
              // Chromium legacy AES-CBC/AES-CTR/AES-GCM failure cases:
              // crypto/subtle/aes-cbc/failures.html,
              // crypto/subtle/aes-ctr/failures.html, and
              // crypto/subtle/aes-gcm/failures.html.
              rejectionName(subtle.encrypt({ name: "AES-CBC", iv: null }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-CBC" }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(0) }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-CTR", counter: new Uint8Array(16), length: 256 }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-CTR", counter: new Uint8Array(16), length: 0 }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-GCM", iv: new Uint8Array(12), additionalData: "5" }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-GCM", iv: new Uint8Array(12), tagLength: 130 }, aesCbcEncrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, hmacExtractable, data)),
              rejectionName(subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, aesCbcDecrypt, data)),
              rejectionName(subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, aesCbcEncrypt, data)),
              rejectionName(subtle.decrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, aesCbcEncrypt, data)),
              // Chromium legacy tests:
              // crypto/subtle/wrapKey-badParameters.html and
              // crypto/subtle/unwrapKey-badParameters.html.
              rejectionName(subtle.wrapKey("raw", 1, aesKw, "AES-KW")),
              rejectionName(subtle.wrapKey("raw", hmacExtractable, "", "AES-KW")),
              // AlgorithmIdentifier scalar values normalize as DOMString
              // names; unsupported names are NotSupportedError, while missing
              // dictionary members stay TypeError.
              rejectionName(subtle.wrapKey("raw", hmacExtractable, aesKw, undefined)),
              rejectionName(subtle.wrapKey("bogus", hmacExtractable, aesKw, "AES-KW")),
              rejectionName(subtle.wrapKey("raw", hmacExtractable, aesKw, { name: "SHA-1" })),
              rejectionName(subtle.wrapKey("raw", hmacExtractable, aesCbcWrap, { name: "AES-CTR", counter: new Uint8Array(16), length: 0 })),
              rejectionName(subtle.wrapKey("raw", hmacExtractable, hmacExtractable, "AES-KW")),
              rejectionName(subtle.wrapKey("raw", hmacExtractable, aesCtrWrap, { name: "AES-CTR", counter: new Uint8Array(16), length: 0 })),
              rejectionName(subtle.wrapKey("raw", hmacHidden, aesKw, "AES-KW")),
              rejectionName(subtle.wrapKey("raw", hmacExtractable, aesKw, "AES-KW")),
              rejectionName(subtle.unwrapKey("raw", null, aesKw, "AES-KW", unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", {}, aesKw, "AES-KW", unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), "hi", "AES-KW", unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, "AES-KW", null, true, 9)),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, null, unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, "AES-KW", 3, true, ["sign"])),
              rejectionName(subtle.unwrapKey("bad-format", new Uint8Array([1]), aesKw, "AES-KW", unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, { name: "SHA-1" }, unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesCbcUnwrap, { name: "AES-CBC" }, unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesCbcUnwrap, { name: "AES-CTR", counter: new Uint8Array(16), length: 0 }, unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesCtrUnwrap, { name: "AES-CTR", counter: new Uint8Array(16), length: 0 }, unwrappedAesCbcAlgorithm, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, "AES-KW", unwrappedAesCbcAlgorithm, true)),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, "AES-KW", { name: "NOPE" }, true, ["sign"])),
              rejectionName(subtle.unwrapKey("raw", new Uint8Array([1]), aesKw, "AES-KW", { name: "HMAC", hash: "SHA-256" }, true, ["sign"]))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle unsupported operation probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoUnsupportedOperationProbe)")
        .expect("crypto subtle unsupported operation promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","TypeError","TypeError","OperationError","TypeError","OperationError","TypeError","OperationError","InvalidAccessError","InvalidAccessError","resolved","InvalidAccessError","TypeError","TypeError","NotSupportedError","TypeError","NotSupportedError","InvalidAccessError","InvalidAccessError","OperationError","InvalidAccessError","resolved","TypeError","TypeError","TypeError","TypeError","NotSupportedError","NotSupportedError","TypeError","NotSupportedError","TypeError","InvalidAccessError","OperationError","TypeError","NotSupportedError","OperationError"]"#
    );
}
#[test]
fn crypto_subtle_import_key_product_resource_limits_reject_large_key_data() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoImportResourceLimitProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const tooLargeBytes = new Uint8Array(65537);
            const tooLargeMember = "A".repeat(65537);
            const tooManyKeyOps = Array.from({ length: 65 }, () => "sign");
            let rsaAlgorithmTouched = false;
            let jwkAlgorithmTouched = false;
            const rsaAlgorithm = {
              get name() {
                rsaAlgorithmTouched = true;
                return "RSA-OAEP";
              },
              get hash() {
                return "SHA-256";
              }
            };
            const hmacAlgorithm = {
              get name() {
                jwkAlgorithmTouched = true;
                return "HMAC";
              },
              get hash() {
                return "SHA-256";
              }
            };

            globalThis.__cryptoImportResourceLimitProbe = [
              await rejectionName(subtle.importKey(
                "spki",
                tooLargeBytes,
                rsaAlgorithm,
                true,
                ["encrypt"]
              )),
              String(rsaAlgorithmTouched),
              await rejectionName(subtle.importKey(
                "jwk",
                { kty: "oct", k: tooLargeMember },
                hmacAlgorithm,
                true,
                ["sign"]
              )),
              await rejectionName(subtle.importKey(
                "jwk",
                { kty: "oct", k: "AQ", key_ops: tooManyKeyOps },
                hmacAlgorithm,
                true,
                ["sign"]
              )),
              String(jwkAlgorithmTouched)
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle importKey resource-limit probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoImportResourceLimitProbe)")
        .expect("crypto subtle importKey resource-limit promise chain should settle");

    assert_eq!(
        result,
        r#"["OperationError","false","OperationError","OperationError","false"]"#
    );
}
#[test]
fn crypto_subtle_non_container_resource_limits_reject_large_inputs() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoNonContainerResourceLimitProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const tooLargeRaw = new Uint8Array(1048577);
            const tooLargeOperation = new Uint8Array(16777217);
            const tooLargeLabel = new Uint8Array(65537);
            const rawAlgorithmTouched = [];
            let digestAlgorithmTouched = false;
            let signAlgorithmTouched = false;
            let verifyAlgorithmTouched = false;
            const rawAlgorithm = (format) => ({
              get name() {
                rawAlgorithmTouched.push(format);
                return "HMAC";
              },
              get hash() {
                return "SHA-256";
              }
            });
            const digestAlgorithm = {
              get name() {
                digestAlgorithmTouched = true;
                return "SHA-256";
              }
            };
            const signAlgorithm = {
              get name() {
                signAlgorithmTouched = true;
                return "HMAC";
              }
            };
            const verifyAlgorithm = {
              get name() {
                verifyAlgorithmTouched = true;
                return "HMAC";
              }
            };
            const hmacKey = await subtle.importKey(
              "raw",
              new Uint8Array([1]),
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign", "verify"]
            );
            const hkdfKey = await subtle.importKey(
              "raw",
              new Uint8Array([2]),
              "HKDF",
              false,
              ["deriveBits"]
            );

            globalThis.__cryptoNonContainerResourceLimitProbe = [
              await rejectionName(subtle.importKey(
                "raw",
                tooLargeRaw,
                rawAlgorithm("raw"),
                false,
                ["sign"]
              )),
              await rejectionName(subtle.importKey(
                "raw-public",
                tooLargeRaw,
                rawAlgorithm("raw-public"),
                false,
                ["sign"]
              )),
              await rejectionName(subtle.importKey(
                "raw-private",
                tooLargeRaw,
                rawAlgorithm("raw-private"),
                false,
                ["sign"]
              )),
              await rejectionName(subtle.importKey(
                "raw-seed",
                tooLargeRaw,
                rawAlgorithm("raw-seed"),
                false,
                ["sign"]
              )),
              await rejectionName(subtle.importKey(
                "raw-secret",
                tooLargeRaw,
                rawAlgorithm("raw-secret"),
                false,
                ["sign"]
              )),
              rawAlgorithmTouched.join(","),
              await rejectionName(subtle.digest(
                digestAlgorithm,
                tooLargeOperation
              )),
              String(digestAlgorithmTouched),
              await rejectionName(subtle.sign(
                signAlgorithm,
                hmacKey,
                tooLargeOperation
              )),
              String(signAlgorithmTouched),
              await rejectionName(subtle.verify(
                verifyAlgorithm,
                hmacKey,
                tooLargeOperation,
                new Uint8Array()
              )),
              String(verifyAlgorithmTouched),
              await rejectionName(subtle.deriveBits(
                { name: "HKDF", hash: "SHA-256", salt: tooLargeRaw, info: new Uint8Array() },
                hkdfKey,
                128
              )),
              await rejectionName(subtle.encrypt(
                { name: "RSA-OAEP", label: tooLargeLabel },
                hmacKey,
                new Uint8Array()
              )),
              String(SubtleCrypto.supports(
                "deriveBits",
                { name: "HKDF", hash: "SHA-256", salt: tooLargeRaw, info: new Uint8Array() },
                128
              )),
              String(SubtleCrypto.supports(
                "encrypt",
                { name: "RSA-OAEP", label: tooLargeLabel }
              ))
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle non-container resource-limit probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoNonContainerResourceLimitProbe)")
        .expect("crypto subtle non-container resource-limit promise chain should settle");

    assert_eq!(
        result,
        r#"["OperationError","OperationError","OperationError","OperationError","OperationError","","OperationError","true","OperationError","true","OperationError","true","OperationError","OperationError","false","false"]"#
    );
}
#[test]
fn crypto_subtle_backend_failures_use_stable_domexception_messages() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoBackendFailureMessageProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionShape = (promise) => promise.then(
              () => "resolved",
              (error) => `${error.name}:${error.message}:${error instanceof DOMException}`
            );

            const aesKey = await subtle.generateKey(
              { name: "AES-GCM", length: 128 },
              false,
              ["encrypt", "decrypt"]
            );
            const iv = new Uint8Array(12);
            const ciphertext = new Uint8Array(
              await subtle.encrypt({ name: "AES-GCM", iv }, aesKey, new Uint8Array([1, 2, 3]))
            );
            ciphertext[ciphertext.length - 1] ^= 1;

            const rsaPair = await subtle.generateKey(
              {
                name: "RSA-OAEP",
                modulusLength: 1024,
                publicExponent: new Uint8Array([1, 0, 1]),
                hash: "SHA-256"
              },
              true,
              ["encrypt", "decrypt"]
            );
            const malformedRsaCiphertext = new Uint8Array(128);

            const aesKwKey = await subtle.generateKey(
              { name: "AES-KW", length: 128 },
              true,
              ["wrapKey", "unwrapKey"]
            );
            const malformedWrappedKey = new Uint8Array([1, 2, 3]);

            const pkcs8 = new Uint8Array([
              48, 46, 2, 1, 0, 48, 5, 6, 3, 43, 101, 110, 4, 34, 4, 32,
              200, 131, 142, 118, 208, 87, 223, 183, 216, 201, 90, 105, 225,
              56, 22, 10, 221, 99, 115, 253, 113, 164, 210, 118, 187, 86,
              227, 168, 27, 100, 255, 97
            ]);
            const smallOrderSpki = new Uint8Array([
              48, 42, 48, 5, 6, 3, 43, 101, 110, 3, 33, 0, 0, 0, 0, 0, 0, 0,
              0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
              0, 0, 0, 0, 0
            ]);
            const x25519Private = await subtle.importKey(
              "pkcs8",
              pkcs8,
              "X25519",
              false,
              ["deriveBits"]
            );
            const smallOrderPublic = await subtle.importKey("spki", smallOrderSpki, "X25519", true, []);

            globalThis.__cryptoBackendFailureMessageProbe = await Promise.all([
              rejectionShape(subtle.decrypt({ name: "AES-GCM", iv }, aesKey, ciphertext)),
              rejectionShape(subtle.decrypt("RSA-OAEP", rsaPair.privateKey, malformedRsaCiphertext)),
              rejectionShape(subtle.unwrapKey(
                "raw",
                malformedWrappedKey,
                aesKwKey,
                "AES-KW",
                "AES-GCM",
                true,
                ["encrypt"]
              )),
              rejectionShape(subtle.deriveBits(
                { name: "X25519", public: smallOrderPublic },
                x25519Private,
                256
              ))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle backend failure message probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoBackendFailureMessageProbe)")
        .expect("crypto subtle backend failure message promise chain should settle");

    assert_eq!(
        result,
        r#"["OperationError:OperationError:true","OperationError:OperationError:true","OperationError:OperationError:true","OperationError:OperationError:true"]"#
    );
}
#[test]
fn crypto_subtle_unwrap_key_normalizes_target_import_algorithm_first() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoUnwrapTargetAlgorithmProbe = [];
          (async () => {
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const bytes = new Uint8Array(16);
            bytes.forEach((_, index) => { bytes[index] = index + 1; });
            const wrapped = new Uint8Array(16);
            const unwrapAlgorithm = { name: "AES-CBC", iv: new Uint8Array(16) };
            const aesCbcUnwrap = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["unwrapKey"]
            );
            const aesCbcDecryptOnly = await subtle.importKey(
              "raw",
              bytes,
              "AES-CBC",
              true,
              ["decrypt"]
            );
            const orderingEvents = [];
            const usageIterable = {
              [Symbol.iterator]() {
                orderingEvents.push("keyUsages");
                return ["sign"][Symbol.iterator]();
              }
            };
            const orderingResult = await rejectionName(subtle.unwrapKey(
              "raw",
              wrapped,
              aesCbcUnwrap,
              {
                get name() {
                  orderingEvents.push("unwrap-name");
                  return "AES-CBC";
                },
                iv: new Uint8Array(16)
              },
              {
                get name() {
                  orderingEvents.push("target-name");
                  return "HMAC";
                },
                get hash() {
                  orderingEvents.push("target-hash");
                  return "SHA-256";
                }
              },
              true,
              usageIterable
            ));
            const invalidWrappedEvents = [];
            const invalidWrappedResult = await rejectionName(subtle.unwrapKey(
              "raw",
              null,
              aesCbcUnwrap,
              unwrapAlgorithm,
              { name: "HMAC", hash: "SHA-256" },
              true,
              {
                [Symbol.iterator]() {
                  invalidWrappedEvents.push("keyUsages");
                  return ["sign"][Symbol.iterator]();
                }
              }
            ));

            globalThis.__cryptoUnwrapTargetAlgorithmProbe = await Promise.all([
              // Chromium parses keyUsages before normalizing either algorithm.
              rejectionName(subtle.unwrapKey(
                "raw",
                wrapped,
                aesCbcUnwrap,
                { name: "SHA-1" },
                { name: "HMAC" },
                true,
                9
              )),
              // The target algorithm is normalized as an importKey algorithm
              // before unwrapping-key capability checks.
              rejectionName(subtle.unwrapKey(
                "raw",
                wrapped,
                aesCbcDecryptOnly,
                unwrapAlgorithm,
                { name: "HMAC" },
                true,
                ["sign"]
              )),
              rejectionName(subtle.unwrapKey(
                "raw",
                wrapped,
                aesCbcDecryptOnly,
                unwrapAlgorithm,
                { name: "NOPE" },
                true,
                ["sign"]
              )),
              rejectionName(subtle.unwrapKey(
                "raw",
                wrapped,
                aesCbcDecryptOnly,
                unwrapAlgorithm,
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
              )),
              rejectionName(subtle.unwrapKey(
                "raw",
                wrapped,
                aesCbcUnwrap,
                unwrapAlgorithm,
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
              )),
              Promise.resolve(orderingResult + ":" + orderingEvents.join(",")),
              Promise.resolve(invalidWrappedResult + ":" + invalidWrappedEvents.join(","))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle unwrap target algorithm probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoUnwrapTargetAlgorithmProbe)")
        .expect("crypto subtle unwrap target algorithm promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","TypeError","NotSupportedError","InvalidAccessError","OperationError","OperationError:keyUsages,unwrap-name,target-name,target-hash","TypeError:"]"#
    );
}
#[test]
fn crypto_subtle_legacy_no_backend_error_edges_match_chromium() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoLegacyNoBackendErrorFailures = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const failures = [];
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            // Chromium legacy tests:
            // crypto/subtle/aes-ctr/generateKey-failures.html and the
            // keyUsages cases from crypto/subtle/aes-cbc/generateKey-failures.html.
            // These are front-end parameter errors and do not require an AES
            // implementation behind the generated-key path.
            const generateErrors = await Promise.all([
              rejectionName(subtle.generateKey({ name: "aes-ctr" }, true, ["encrypt", "decrypt"])),
              rejectionName(subtle.generateKey({ name: "aes-ctr", length: 70000 }, true, ["encrypt", "decrypt"])),
              rejectionName(subtle.generateKey({ name: "aes-ctr", length: -3 }, true, ["encrypt", "decrypt"])),
              rejectionName(subtle.generateKey({ name: "aes-ctr", length: -Infinity }, true, ["encrypt", "decrypt"])),
              rejectionName(subtle.generateKey({ name: "aes-cbc", length: 1024 }, true, -1)),
              rejectionName(subtle.generateKey({ name: "aes-cbc", length: 1024 }, true, null)),
              rejectionName(subtle.generateKey({ name: "aes-cbc", length: 1024 }, true, ["boo"]))
            ]);
            if (generateErrors.join(",") !== "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError") {
              failures.push("generate-errors:" + generateErrors.join(","));
            }

            // Chromium legacy test: crypto/subtle/aes-cbc/invalid-length.html.
            // The 176-bit raw key is rejected during AES key import, before any
            // encrypt/decrypt backend could be involved.
            const invalidLength = await rejectionName(subtle.importKey(
              "raw",
              new Uint8Array([
                0x8e, 0x73, 0xb0, 0xf7, 0xda, 0x0e, 0x64, 0x52,
                0xc8, 0x10, 0xf3, 0x2b, 0x80, 0x90, 0x79, 0xe5,
                0x62, 0xf8, 0xea, 0xd2, 0x52, 0x2c
              ]),
              { name: "aes-cbc" },
              true,
              ["encrypt", "decrypt"]
            ));
            if (invalidLength !== "DataError") {
              failures.push("aes-cbc-invalid-length:" + invalidLength);
            }

            // Chromium legacy test: crypto/subtle/aes-cbc/wrong-key-class.html.
            // A key-algorithm mismatch is observable before the local AES-CBC
            // encrypt backend is reached.
            const hmacKey = await subtle.importKey(
              "raw",
              new Uint8Array([0x61]),
              { name: "HMAC", hash: { name: "sha-1" } },
              true,
              ["sign", "verify"]
            );
            const wrongKeyClass = await rejectionName(subtle.encrypt(
              { name: "aes-cbc", iv: new Uint8Array(16) },
              hmacKey,
              new Uint8Array(64)
            ));
            if (wrongKeyClass !== "InvalidAccessError") {
              failures.push("aes-cbc-wrong-key-class:" + wrongKeyClass);
            }

            // Chromium legacy test: crypto/subtle/hkdf/exportKey.html.
            // HKDF keys are intentionally non-extractable; exportKey fails at
            // the common extractability gate, not at algorithm dispatch.
            const hkdfKey = await subtle.importKey(
              "raw",
              new Uint8Array([
                0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b,
                0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b,
                0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b
              ]),
              { name: "HKDF" },
              false,
              ["deriveKey", "deriveBits"]
            );
            const hkdfExport = await rejectionName(subtle.exportKey("raw", hkdfKey));
            if (hkdfExport !== "InvalidAccessError") {
              failures.push("hkdf-export:" + hkdfExport);
            }

            globalThis.__cryptoLegacyNoBackendErrorFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle legacy no-backend error probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoLegacyNoBackendErrorFailures)")
        .expect("crypto subtle legacy no-backend error promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_usage_reflection_matches_chromium_legacy() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoUsageReflectionFailures = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const failures = [];
            const aesBytes = new Uint8Array([
              0x8e, 0x73, 0xb0, 0xf7, 0xda, 0x0e, 0x64, 0x52,
              0xc8, 0x10, 0xf3, 0x2b, 0x80, 0x90, 0x79, 0xe5
            ]);
            const hmacBytes = new Uint8Array([
              0x6a, 0x18, 0xe4, 0x9f, 0xef, 0xf7, 0xf3, 0xb7,
              0xe0, 0x9e, 0xc8, 0x9b, 0x7f, 0x6d, 0xea, 0xb2,
              0xf6, 0xa1, 0x8e, 0x49, 0xfe, 0xff, 0x7f, 0x3b,
              0x7e, 0x09, 0xec, 0x89, 0xb7, 0xf6, 0xde, 0xab
            ]);

            // Chromium legacy tests:
            // crypto/subtle/cryptokey-interface-is-visible.html and
            // crypto/subtle/importKey-normalize-usages.html.
            const duplicateUsageKey = await subtle.importKey(
              "raw",
              aesBytes,
              { name: "aes-cbc" },
              true,
              ["decrypt", "decrypt", "encrypt", "wrapKey", "encrypt", "encrypt"]
            );
            if (!(duplicateUsageKey instanceof CryptoKey)) {
              failures.push("crypto-key-interface");
            }
            if (duplicateUsageKey.usages.join(",") !== "encrypt,decrypt,wrapKey") {
              failures.push("normalized-usages:" + duplicateUsageKey.usages.join(","));
            }

            // Chromium legacy test: crypto/subtle/jwk-export-use-values.html.
            const aesUsageCases = [
              [["encrypt"], "encrypt"],
              [["decrypt"], "decrypt"],
              [["encrypt", "decrypt"], "encrypt,decrypt"],
              [["wrapKey"], "wrapKey"],
              [["unwrapKey"], "unwrapKey"],
              [["wrapKey", "unwrapKey"], "wrapKey,unwrapKey"],
              [["encrypt", "decrypt", "wrapKey"], "encrypt,decrypt,wrapKey"],
              [["encrypt", "decrypt", "wrapKey", "unwrapKey"], "encrypt,decrypt,wrapKey,unwrapKey"]
            ];
            for (const [usages, expectedKeyOps] of aesUsageCases) {
              const key = await subtle.importKey(
                "raw",
                aesBytes,
                { name: "AES-CBC" },
                true,
                usages
              );
              const jwk = await subtle.exportKey("jwk", key);
              if (
                jwk.use !== undefined ||
                jwk.key_ops.join(",") !== expectedKeyOps ||
                jwk.alg !== "A128CBC" ||
                jwk.ext !== true
              ) {
                failures.push(`aes:${usages.join("+")}:${jwk.use}:${jwk.key_ops.join(",")}:${jwk.alg}:${jwk.ext}`);
              }
            }

            const hmacUsageCases = [
              [["sign"], "sign"],
              [["verify"], "verify"],
              [["sign", "verify"], "sign,verify"]
            ];
            for (const [usages, expectedKeyOps] of hmacUsageCases) {
              const key = await subtle.importKey(
                "raw",
                hmacBytes,
                { name: "hmac", hash: { name: "sha-256" } },
                true,
                usages
              );
              const jwk = await subtle.exportKey("jwk", key);
              if (
                jwk.use !== undefined ||
                jwk.key_ops.join(",") !== expectedKeyOps ||
                jwk.alg !== "HS256" ||
                jwk.ext !== true
              ) {
                failures.push(`hmac:${usages.join("+")}:${jwk.use}:${jwk.key_ops.join(",")}:${jwk.alg}:${jwk.ext}`);
              }
            }

            globalThis.__cryptoUsageReflectionFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle usage reflection probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoUsageReflectionFailures)")
        .expect("crypto subtle usage reflection promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_legacy_vectors_and_usage_edges_match_chromium() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoLegacyVectorFailures = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const failures = [];
            const sameBytes = (left, right) => {
              const a = new Uint8Array(left);
              const b = new Uint8Array(right);
              return a.length === b.length && a.every((value, index) => value === b[index]);
            };
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            // Chromium legacy test: crypto/subtle/digest-arraybuffer.html.
            const digest = await subtle.digest(
              { name: "sha-256" },
              new Uint8Array([0]).buffer
            );
            if (!sameBytes(digest, [
              0x6e, 0x34, 0x0b, 0x9c, 0xff, 0xb3, 0x7a, 0x98,
              0x9c, 0xa5, 0x44, 0xe6, 0xbb, 0x78, 0x0a, 0x2c,
              0x78, 0x90, 0x1d, 0x3f, 0xb3, 0x37, 0x38, 0x76,
              0x85, 0x11, 0xa3, 0x06, 0x17, 0xaf, 0xa0, 0x1d
            ])) {
              failures.push("digest-arraybuffer");
            }

            // Chromium legacy test: crypto/subtle/hmac/import-jwk.html.
            const hmacKey = await subtle.importKey(
              "jwk",
              {
                kty: "oct",
                alg: "HS256",
                use: "sig",
                ext: false,
                k: "ahjkn-_387fgnsibf23qsvahjkn-_387fgnsibf23qs"
              },
              { name: "HMAC", hash: { name: "SHA-256" } },
              false,
              ["sign", "verify"]
            );
            const signature = await subtle.sign(
              hmacKey.algorithm,
              hmacKey,
              new Uint8Array([0x66, 0x6f, 0x6f])
            );
            const hmacVerified = await subtle.verify(
              hmacKey.algorithm,
              hmacKey,
              signature,
              new Uint8Array([0x66, 0x6f, 0x6f])
            );
            if (
              hmacKey.type !== "secret" ||
              hmacKey.extractable !== false ||
              hmacKey.algorithm.name !== "HMAC" ||
              hmacKey.algorithm.length !== 256 ||
              hmacKey.usages.join(",") !== "sign,verify" ||
              !hmacVerified ||
              !sameBytes(signature, [
                0xe0, 0x37, 0x36, 0xfe, 0x09, 0x88, 0x92, 0xb2,
                0xa2, 0xda, 0x77, 0x81, 0x24, 0x31, 0xf7, 0xc0,
                0x14, 0xd3, 0x2e, 0x2f, 0xd6, 0x9f, 0x3b, 0xcf,
                0xf8, 0x83, 0xac, 0x92, 0x3a, 0x8f, 0xa2, 0xda
              ])
            ) {
              failures.push("hmac-import-jwk");
            }
            // Chromium legacy test: crypto/subtle/hmac/export-key.html.
            // This keeps the exact historical HMAC JWK fixture covered for
            // key-format validation, raw/JWK export shape, and extractability.
            const hmacExportJwk = {
              kty: "oct",
              k: "ahjkn-_387fgnsibf23qsvahjkn-_387fgnsibf23qs"
            };
            const extractableHmacKey = await subtle.importKey(
              "jwk",
              hmacExportJwk,
              { name: "HMAC", hash: { name: "SHA-256" } },
              true,
              ["sign", "verify"]
            );
            const hmacExportFormatErrors = await Promise.all([
              rejectionName(subtle.exportKey(null, extractableHmacKey)),
              rejectionName(subtle.exportKey(undefined, extractableHmacKey)),
              rejectionName(subtle.exportKey({}, extractableHmacKey)),
              rejectionName(subtle.exportKey("", extractableHmacKey)),
              rejectionName(subtle.exportKey("foobar", extractableHmacKey))
            ]);
            if (hmacExportFormatErrors.join(",") !== "TypeError,TypeError,TypeError,TypeError,TypeError") {
              failures.push("hmac-export-key:formats:" + hmacExportFormatErrors.join(","));
            }
            const exportedHmacRaw = await subtle.exportKey("raw", extractableHmacKey);
            const exportedHmacJwk = await subtle.exportKey("jwk", extractableHmacKey);
            if (
              !sameBytes(exportedHmacRaw, [
                0x6a, 0x18, 0xe4, 0x9f, 0xef, 0xf7, 0xf3, 0xb7,
                0xe0, 0x9e, 0xc8, 0x9b, 0x7f, 0x6d, 0xea, 0xb2,
                0xf6, 0xa1, 0x8e, 0x49, 0xfe, 0xff, 0x7f, 0x3b,
                0x7e, 0x09, 0xec, 0x89, 0xb7, 0xf6, 0xde, 0xab
              ]) ||
              exportedHmacJwk.kty !== "oct" ||
              exportedHmacJwk.k !== hmacExportJwk.k ||
              exportedHmacJwk.alg !== "HS256" ||
              exportedHmacJwk.ext !== true ||
              exportedHmacJwk.use !== undefined ||
              exportedHmacJwk.key_ops.join(",") !== "sign,verify"
            ) {
              failures.push("hmac-export-key:shape");
            }
            const unextractableHmacKey = await subtle.importKey(
              "jwk",
              hmacExportJwk,
              { name: "HMAC", hash: { name: "SHA-256" } },
              false,
              ["sign", "verify"]
            );
            const hmacUnextractableErrors = await Promise.all([
              rejectionName(subtle.exportKey("raw", unextractableHmacKey)),
              rejectionName(subtle.exportKey("jwk", unextractableHmacKey))
            ]);
            if (hmacUnextractableErrors.join(",") !== "InvalidAccessError,InvalidAccessError") {
              failures.push("hmac-export-key:unextractable:" + hmacUnextractableErrors.join(","));
            }

            // Chromium legacy test: crypto/subtle/aes-key-usages.html.
            const aesBytes = new Uint8Array([
              0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6,
              0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c
            ]);
            const missingEncrypt = await subtle.importKey(
              "raw",
              aesBytes,
              "AES-CBC",
              false,
              ["decrypt", "wrapKey", "unwrapKey"]
            );
            const missingDecrypt = await subtle.importKey(
              "raw",
              aesBytes,
              "AES-CBC",
              false,
              ["encrypt", "wrapKey", "unwrapKey"]
            );
            const aesErrors = await Promise.all([
              rejectionName(subtle.encrypt(
                { name: "AES-CBC", iv: new Uint8Array(16) },
                missingEncrypt,
                new Uint8Array([0x68, 0x65, 0x6c, 0x6c, 0x6f])
              )),
              rejectionName(subtle.decrypt(
                { name: "AES-CBC", iv: new Uint8Array(16) },
                missingDecrypt,
                new Uint8Array([0x68, 0x65, 0x6c, 0x6c, 0x6f])
              ))
            ]);
            if (aesErrors.join(",") !== "InvalidAccessError,InvalidAccessError") {
              failures.push("aes-key-usages:" + aesErrors.join(","));
            }

            globalThis.__cryptoLegacyVectorFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle legacy vector probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoLegacyVectorFailures)")
        .expect("crypto subtle legacy vector promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_kdf_derive_key_failures_match_chromium_boundaries() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKdfDeriveKeyFailureProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const source = new Uint8Array([112, 97, 115, 115, 119, 111, 114, 100]);
            const salt = new Uint8Array([115, 97, 108, 116]);
            const info = new Uint8Array([105, 110, 102, 111]);
            const pbkdf2 = await subtle.importKey(
              "raw",
              source,
              "PBKDF2",
              false,
              ["deriveKey"]
            );
            const hkdf = await subtle.importKey(
              "raw",
              source,
              "HKDF",
              false,
              ["deriveKey"]
            );
            const hkdfDeriveBitsOnly = await subtle.importKey(
              "raw",
              source,
              "HKDF",
              false,
              ["deriveBits"]
            );
            const hmac = await subtle.importKey(
              "raw",
              source,
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign"]
            );
            const pbkdf2Params = {
              name: "PBKDF2",
              salt,
              iterations: 1,
              hash: "SHA-1"
            };
            const hkdfParams = {
              name: "HKDF",
              salt,
              info,
              hash: "SHA-256"
            };
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            // Chromium legacy test:
            // crypto/subtle/pbkdf2/deriveKey-failures.html.
            globalThis.__cryptoKdfDeriveKeyFailureProbe = await Promise.all([
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "AES-CBC" },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "AES-CBC", length: 120 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "AES-CBC", length: 192 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "AES-CBC", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "HMAC", hash: "SHA-1", length: 0 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "HMAC", hash: "SHA-1", length: 65537 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "ECDH", namedCurve: "P-256" },
                true,
                ["deriveBits"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                pbkdf2,
                { name: "RSA-OAEP", hash: "SHA-1" },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "PBKDF2", iterations: 1, hash: "SHA-1" },
                pbkdf2,
                { name: "AES-CBC", length: 128 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "PBKDF2", salt, hash: "SHA-1" },
                pbkdf2,
                { name: "AES-CBC", length: 128 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "PBKDF2", salt, iterations: 1, hash: "SHA256" },
                pbkdf2,
                { name: "AES-CBC", length: 128 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "PBKDF2", salt, iterations: 0, hash: "SHA-1" },
                pbkdf2,
                { name: "AES-CBC", length: 128 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                pbkdf2Params,
                hmac,
                { name: "AES-CBC", length: 128 },
                true,
                ["encrypt"]
              )),
              rejectionName(subtle.deriveKey(
                hkdfParams,
                hkdfDeriveBitsOnly,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "HKDF", info, hash: "SHA-256" },
                hkdf,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "HKDF", salt, hash: "SHA-256" },
                hkdf,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                { name: "HKDF", salt, info, hash: "SHA256" },
                hkdf,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              )),
              rejectionName(subtle.deriveKey(
                hkdfParams,
                hkdf,
                { name: "HMAC", hash: "SHA-256", length: 0 },
                true,
                ["sign"]
              ))
            ]);
          })();
        })()
        "#,
    )
    .expect("crypto subtle KDF deriveKey failure probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKdfDeriveKeyFailureProbe)")
        .expect("crypto subtle KDF deriveKey failure promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","OperationError","resolved","SyntaxError","TypeError","TypeError","NotSupportedError","NotSupportedError","TypeError","TypeError","NotSupportedError","OperationError","InvalidAccessError","InvalidAccessError","TypeError","TypeError","NotSupportedError","TypeError"]"#
    );
}
#[test]
fn crypto_subtle_derive_key_normalizes_source_before_target() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoDeriveKeyOrderProbe = ["pending"];
          (async () => {
            const failures = [];
            const subtle = crypto.subtle;
            const source = new Uint8Array([112, 97, 115, 115, 119, 111, 114, 100]);
            const salt = new Uint8Array([115, 97, 108, 116]);
            const pbkdf2 = await subtle.importKey(
              "raw",
              source,
              "PBKDF2",
              false,
              ["deriveKey"]
            );
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            let sourceGetterForUsage = false;
            let targetGetterForUsage = false;
            const invalidUsage = await rejectionName(subtle.deriveKey(
              {
                get name() { sourceGetterForUsage = true; return "PBKDF2"; },
                salt,
                iterations: 1,
                hash: "SHA-1"
              },
              pbkdf2,
              {
                get name() { targetGetterForUsage = true; return "AES-CBC"; },
                length: 128
              },
              true,
              ["ENCRYPT"]
            ));
            if (invalidUsage !== "TypeError" || sourceGetterForUsage || targetGetterForUsage) {
              failures.push("usage-order:" + invalidUsage + ":" + sourceGetterForUsage + ":" + targetGetterForUsage);
            }

            let targetGetterForSourceName = false;
            const invalidSourceName = await rejectionName(subtle.deriveKey(
              { name: "NOPE" },
              pbkdf2,
              {
                get name() { targetGetterForSourceName = true; return "AES-CBC"; },
                length: 128
              },
              true,
              ["encrypt"]
            ));
            if (invalidSourceName !== "NotSupportedError" || targetGetterForSourceName) {
              failures.push("source-name-order:" + invalidSourceName + ":" + targetGetterForSourceName);
            }

            let targetGetterForSourceParams = false;
            const missingSourceSalt = await rejectionName(subtle.deriveKey(
              { name: "PBKDF2", iterations: 1, hash: "SHA-1" },
              pbkdf2,
              {
                get name() { targetGetterForSourceParams = true; return "AES-CBC"; },
                length: 128
              },
              true,
              ["encrypt"]
            ));
            if (missingSourceSalt !== "TypeError" || targetGetterForSourceParams) {
              failures.push("source-params-order:" + missingSourceSalt + ":" + targetGetterForSourceParams);
            }

            globalThis.__cryptoDeriveKeyOrderProbe = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle deriveKey ordering probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoDeriveKeyOrderProbe)")
        .expect("crypto subtle deriveKey ordering promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_kdf_hash_member_errors_match_chromium() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKdfHashMemberProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const source = new Uint8Array([112, 97, 115, 115]);
            const salt = new Uint8Array([115, 97, 108, 116]);
            const info = new Uint8Array([105, 110, 102, 111]);
            const hkdf = await subtle.importKey("raw", source, "HKDF", false, ["deriveBits"]);
            const pbkdf2 = await subtle.importKey("raw", source, "PBKDF2", false, ["deriveBits"]);
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );
            const events = [];
            const hkdfMissingHash = {
              get name() {
                events.push("hkdf:name");
                return "HKDF";
              },
              get salt() {
                events.push("hkdf:salt");
                return salt;
              },
              get info() {
                events.push("hkdf:info");
                return info;
              }
            };
            const pbkdf2MissingHash = {
              get name() {
                events.push("pbkdf2:name");
                return "PBKDF2";
              },
              get salt() {
                events.push("pbkdf2:salt");
                return salt;
              },
              get iterations() {
                events.push("pbkdf2:iterations");
                return 1;
              }
            };

            // Blink treats the required HashAlgorithmIdentifier member shape as
            // WebIDL input: missing/undefined/object-without-name is TypeError.
            // A syntactically valid but unsupported digest name is
            // NotSupportedError.
            globalThis.__cryptoKdfHashMemberProbe = [
              await rejectionName(subtle.deriveBits(hkdfMissingHash, hkdf, 128)),
              events.join(","),
              await rejectionName(subtle.deriveBits(
                { name: "HKDF", hash: undefined, salt, info },
                hkdf,
                128
              )),
              await rejectionName(subtle.deriveBits(
                { name: "HKDF", hash: {}, salt, info },
                hkdf,
                128
              )),
              await rejectionName(subtle.deriveBits(
                { name: "HKDF", hash: "HMAC", salt, info },
                hkdf,
                128
              )),
              await rejectionName(subtle.deriveBits(pbkdf2MissingHash, pbkdf2, 128)),
              events.join(","),
              await rejectionName(subtle.deriveBits(
                { name: "PBKDF2", salt, iterations: 1, hash: undefined },
                pbkdf2,
                128
              )),
              await rejectionName(subtle.deriveBits(
                { name: "PBKDF2", salt, iterations: 1, hash: {} },
                pbkdf2,
                128
              )),
              await rejectionName(subtle.deriveBits(
                { name: "PBKDF2", salt, iterations: 1, hash: "SHA256" },
                pbkdf2,
                128
              ))
            ];
          })();
        })()
        "#,
    )
    .expect("crypto subtle KDF hash-member probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKdfHashMemberProbe)")
        .expect("crypto subtle KDF hash-member promise chain should settle");

    assert_eq!(
        result,
        r#"["TypeError","hkdf:name","TypeError","TypeError","NotSupportedError","TypeError","hkdf:name,pbkdf2:name,pbkdf2:salt,pbkdf2:iterations","TypeError","TypeError","NotSupportedError"]"#
    );
}
#[test]
fn crypto_subtle_derive_key_normalizes_target_before_usage_compatibility() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoDeriveKeyTargetUsageOrderProbe = ["pending"];
          (async () => {
            const failures = [];
            const subtle = crypto.subtle;
            const source = new Uint8Array([112, 97, 115, 115, 119, 111, 114, 100]);
            const salt = new Uint8Array([115, 97, 108, 116]);
            const baseKey = await subtle.importKey(
              "raw",
              source,
              "PBKDF2",
              false,
              ["deriveKey"]
            );
            const sourceAlgorithm = {
              name: "PBKDF2",
              salt,
              iterations: 1,
              hash: "SHA-256"
            };
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            const aesEvents = [];
            const aesError = await rejectionName(subtle.deriveKey(
              sourceAlgorithm,
              baseKey,
              {
                get name() {
                  aesEvents.push("name");
                  return "AES-CBC";
                },
                get length() {
                  aesEvents.push("length");
                  return 128;
                }
              },
              true,
              ["sign"]
            ));
            if (
              aesError !== "SyntaxError" ||
              !aesEvents.includes("name") ||
              !aesEvents.includes("length") ||
              aesEvents.indexOf("length") < aesEvents.indexOf("name")
            ) {
              failures.push("aes:" + aesError + ":" + aesEvents.join(","));
            }

            const missingLengthError = await rejectionName(subtle.deriveKey(
              sourceAlgorithm,
              baseKey,
              { name: "AES-CBC" },
              true,
              ["sign"]
            ));
            if (missingLengthError !== "TypeError") {
              failures.push("aes-missing-length:" + missingLengthError);
            }

            const hmacEvents = [];
            const hmacError = await rejectionName(subtle.deriveKey(
              sourceAlgorithm,
              baseKey,
              {
                get name() {
                  hmacEvents.push("name");
                  return "HMAC";
                },
                get hash() {
                  hmacEvents.push("hash");
                  return "SHA-256";
                },
                get length() {
                  hmacEvents.push("length");
                  return 128;
                }
              },
              true,
              ["encrypt"]
            ));
            if (
              hmacError !== "SyntaxError" ||
              !hmacEvents.includes("name") ||
              !hmacEvents.includes("hash") ||
              !hmacEvents.includes("length") ||
              hmacEvents.indexOf("hash") < hmacEvents.indexOf("name") ||
              hmacEvents.indexOf("length") < hmacEvents.indexOf("name")
            ) {
              failures.push("hmac:" + hmacError + ":" + hmacEvents.join(","));
            }

            globalThis.__cryptoDeriveKeyTargetUsageOrderProbe = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle deriveKey target usage-order probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoDeriveKeyTargetUsageOrderProbe)")
        .expect("crypto subtle deriveKey target usage-order promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_derive_checks_base_key_before_algorithm_getters() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoDeriveBaseKeyOrderProbe = ["pending"];
          (async () => {
            const failures = [];
            const subtle = crypto.subtle;
            const rejectionName = (promise) => promise.then(
              () => "resolved",
              (error) => error.name
            );

            let deriveBitsSourceGetter = false;
            const deriveBitsError = await rejectionName(subtle.deriveBits(
              {
                get name() { deriveBitsSourceGetter = true; return "PBKDF2"; },
                salt: new Uint8Array([1]),
                iterations: 1,
                hash: "SHA-256"
              },
              {},
              8
            ));
            if (deriveBitsError !== "TypeError" || deriveBitsSourceGetter) {
              failures.push(`deriveBits:${deriveBitsError}:${deriveBitsSourceGetter}`);
            }

            let deriveKeySourceGetter = false;
            let deriveKeyTargetGetter = false;
            const deriveKeyError = await rejectionName(subtle.deriveKey(
              {
                get name() { deriveKeySourceGetter = true; return "PBKDF2"; },
                salt: new Uint8Array([1]),
                iterations: 1,
                hash: "SHA-256"
              },
              {},
              {
                get name() { deriveKeyTargetGetter = true; return "AES-CBC"; },
                length: 128
              },
              true,
              ["encrypt"]
            ));
            if (deriveKeyError !== "TypeError" || deriveKeySourceGetter || deriveKeyTargetGetter) {
              failures.push(
                `deriveKey:${deriveKeyError}:${deriveKeySourceGetter}:${deriveKeyTargetGetter}`
              );
            }

            let invalidUsageSourceGetter = false;
            const invalidUsageError = await rejectionName(subtle.deriveKey(
              {
                get name() { invalidUsageSourceGetter = true; return "PBKDF2"; },
                salt: new Uint8Array([1]),
                iterations: 1,
                hash: "SHA-256"
              },
              {},
              { name: "AES-CBC", length: 128 },
              true,
              ["ENCRYPT"]
            ));
            if (invalidUsageError !== "TypeError" || invalidUsageSourceGetter) {
              failures.push(`invalid-usage:${invalidUsageError}:${invalidUsageSourceGetter}`);
            }

            globalThis.__cryptoDeriveBaseKeyOrderProbe = failures;
          })();
        })()
        "#,
    )
    .expect("crypto subtle derive baseKey ordering probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoDeriveBaseKeyOrderProbe)")
        .expect("crypto subtle derive baseKey ordering promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_subtle_derive_bits_converts_length_before_algorithm_getter() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoDeriveBitsLengthOrderProbe = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const baseKey = await subtle.importKey(
              "raw",
              new Uint8Array([1]),
              "HKDF",
              false,
              ["deriveBits"]
            );
            const events = [];
            const algorithm = {
              get name() {
                events.push("name");
                return "HKDF";
              },
              salt: new Uint8Array([1]),
              info: new Uint8Array([2]),
              hash: "SHA-256"
            };
            const length = {
              valueOf() {
                events.push("length");
                return 8;
              }
            };
            try {
              await subtle.deriveBits(algorithm, baseKey, length);
              events.push("resolved");
            } catch (error) {
              events.push(error.name);
            }
            globalThis.__cryptoDeriveBitsLengthOrderProbe = events;
          })();
        })()
        "#,
    )
    .expect("crypto subtle deriveBits length-order probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoDeriveBitsLengthOrderProbe)")
        .expect("crypto subtle deriveBits length-order promise chain should settle");

    assert_eq!(result, r#"["length","name","resolved"]"#);
}
