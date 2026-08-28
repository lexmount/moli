use super::*;

#[test]
fn crypto_random_uuid_returns_rfc4122_v4_string() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const failures = [];
              const iterations = 256;
              const seen = new Set();
              const namespaceFormat = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/;
              for (let i = 0; i < iterations; i++) {
                const value = crypto.randomUUID();
                if (typeof value !== "string" || value.length !== 36) {
                  failures.push("shape:" + i);
                  continue;
                }
                if (seen.has(value)) {
                  failures.push("collision:" + value);
                }
                seen.add(value);
                if (!namespaceFormat.test(value)) {
                  failures.push("namespace:" + value);
                }
                const version = parseInt(value.split("-")[2].slice(0, 2), 16) & 0b11110000;
                if (version !== 0b01000000) {
                  failures.push("version:" + value);
                }
                const variant = parseInt(value.split("-")[3].slice(0, 2), 16) & 0b11000000;
                if (variant !== 0b10000000) {
                  failures.push("variant:" + value);
                }
              }
              return JSON.stringify(failures);
            })()
            "#,
        )
        .expect("crypto.randomUUID should evaluate");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_prototype_members_reject_wrong_receiver() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const subtleGetter = Object.getOwnPropertyDescriptor(Crypto.prototype, "subtle").get;
              const subtleDescriptor = Object.getOwnPropertyDescriptor(Crypto.prototype, "subtle");
              const typeErrorName = (fn) => {
                try {
                  fn();
                  return "resolved";
                } catch (error) {
                  return error.name;
                }
              };
              const bytes = new Uint8Array(1);
              const returned = Crypto.prototype.getRandomValues.call(crypto, bytes);
              const uuid = Crypto.prototype.randomUUID.call(crypto);

              return [
                subtleGetter.call(crypto) === crypto.subtle,
                [
                  typeof subtleDescriptor.get,
                  subtleDescriptor.get.name,
                  subtleDescriptor.get.length,
                  typeof subtleDescriptor.set,
                  subtleDescriptor.enumerable,
                  subtleDescriptor.configurable,
                  Object.prototype.hasOwnProperty.call(crypto, "subtle")
                ].join(":"),
                returned === bytes,
                bytes.length,
                typeof uuid,
                typeErrorName(() => subtleGetter.call({})),
                typeErrorName(() => Crypto.prototype.getRandomValues.call({}, new Uint8Array(1))),
                typeErrorName(() => Crypto.prototype.randomUUID.call({}))
              ].join("|");
            })()
            "#,
        )
        .expect("crypto receiver brand probe should evaluate");

    assert_eq!(
        result,
        "true|function:get subtle:0:undefined:true:true:false|true|1|string|TypeError|TypeError|TypeError"
    );
}

#[test]
fn crypto_operations_keep_declared_method_descriptors() {
    let mut vm = new_storage_test_vm("https://crypto-operation-descriptors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const methodDescriptor = (owner, name, expectedLength) => {
                const descriptor = Object.getOwnPropertyDescriptor(owner, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.value?.length === expectedLength,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const subtleMethods = [
                ["digest", 2],
                ["generateKey", 3],
                ["encrypt", 3],
                ["decrypt", 3],
                ["sign", 3],
                ["verify", 4],
                ["deriveBits", 2],
                ["deriveKey", 5],
                ["getPublicKey", 2],
                ["importKey", 5],
                ["exportKey", 2],
                ["wrapKey", 4],
                ["unwrapKey", 7]
              ];
              return JSON.stringify({
                crypto: [
                  methodDescriptor(Crypto.prototype, "getRandomValues", 1),
                  methodDescriptor(Crypto.prototype, "randomUUID", 0)
                ],
                subtle: subtleMethods.map(([name, length]) =>
                  methodDescriptor(SubtleCrypto.prototype, name, length)
                ),
                static: methodDescriptor(SubtleCrypto, "supports", 2),
                keys: [
                  Object.keys(Crypto.prototype).includes("getRandomValues"),
                  Object.keys(Crypto.prototype).includes("randomUUID"),
                  Object.keys(SubtleCrypto.prototype).includes("digest"),
                  Object.keys(SubtleCrypto).includes("supports")
                ],
                basicBehavior: [
                  crypto.getRandomValues(new Uint8Array(1)).length,
                  typeof crypto.randomUUID(),
                  SubtleCrypto.supports("digest", "SHA-256")
                ]
              });
            })()
            "#,
        )
        .expect("crypto operation descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"crypto":["getRandomValues:true:function:getRandomValues:1:true:true:true:true","randomUUID:true:function:randomUUID:0:true:true:true:true"],"subtle":["digest:true:function:digest:2:true:true:true:true","generateKey:true:function:generateKey:3:true:true:true:true","encrypt:true:function:encrypt:3:true:true:true:true","decrypt:true:function:decrypt:3:true:true:true:true","sign:true:function:sign:3:true:true:true:true","verify:true:function:verify:4:true:true:true:true","deriveBits:true:function:deriveBits:2:true:true:true:true","deriveKey:true:function:deriveKey:5:true:true:true:true","getPublicKey:true:function:getPublicKey:2:true:true:true:true","importKey:true:function:importKey:5:true:true:true:true","exportKey:true:function:exportKey:2:true:true:true:true","wrapKey:true:function:wrapKey:4:true:true:true:true","unwrapKey:true:function:unwrapKey:7:true:true:true:true"],"static":"supports:true:function:supports:2:true:true:true:true","keys":[true,true,true,true],"basicBehavior":[1,"string",true]}"#
    );
}

#[test]
fn crypto_get_random_values_matches_chromium_type_boundaries() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              // Chromium WPT: WebCryptoAPI/getRandomValues.any.js.
              // Moli rejects SharedArrayBuffer-backed views at the
              // BufferSource boundary before filling bytes, which is also
              // observable through WebIDL's [AllowShared] absence.
              const failures = [];
              const thrown = (fn) => {
                try {
                  fn();
                  return "resolved";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              };
              const expect = (label, actual, expected) => {
                if (actual !== expected) {
                  failures.push(`${label}:${actual}`);
                }
              };

              const floatArrayNames = ["Float32Array", "Float64Array"];
              if (typeof Float16Array === "function") {
                floatArrayNames.unshift("Float16Array");
              }
              for (const name of floatArrayNames) {
                const ctor = globalThis[name];
                expect(
                  `${name}:normal`,
                  thrown(() => crypto.getRandomValues(new ctor(6))),
                  "TypeMismatchError:17"
                );
                const tooLong = 65536 / ctor.BYTES_PER_ELEMENT + 1;
                expect(
                  `${name}:quota-order`,
                  thrown(() => crypto.getRandomValues(new ctor(tooLong))),
                  "TypeMismatchError:17"
                );
              }

              expect(
                "DataView:normal",
                thrown(() => crypto.getRandomValues(new DataView(new ArrayBuffer(6)))),
                "TypeMismatchError:17"
              );
              expect(
                "DataView:quota-order",
                thrown(() => crypto.getRandomValues(new DataView(new ArrayBuffer(65537)))),
                "TypeMismatchError:17"
              );
              expect(
                "non-view",
                thrown(() => crypto.getRandomValues({})),
                "TypeError:undefined"
              );

              const integerArrayNames = [
                "Int8Array",
                "Int16Array",
                "Int32Array",
                "BigInt64Array",
                "Uint8Array",
                "Uint8ClampedArray",
                "Uint16Array",
                "Uint32Array",
                "BigUint64Array",
              ];
              for (const name of integerArrayNames) {
                const ctor = globalThis[name];
                if (typeof ctor !== "function") {
                  failures.push(`${name}:missing`);
                  continue;
                }

                const view = new ctor(8);
                const returned = crypto.getRandomValues(view);
                if (returned !== view || returned.constructor !== ctor) {
                  failures.push(`${name}:return`);
                }

                const tooLong = 65536 / ctor.BYTES_PER_ELEMENT + 1;
                expect(
                  `${name}:quota`,
                  thrown(() => crypto.getRandomValues(new ctor(tooLong))),
                  "QuotaExceededError:22"
                );

                const empty = new ctor(0);
                if (crypto.getRandomValues(empty).length !== 0) {
                  failures.push(`${name}:zero-length`);
                }

                class Buffer extends ctor {}
                const subclass = new Buffer(256);
                const subclassReturned = crypto.getRandomValues(subclass);
                if (subclassReturned !== subclass || subclassReturned.constructor !== Buffer) {
                  failures.push(`${name}:subclass`);
                }
              }

              if (typeof SharedArrayBuffer === "function") {
                expect(
                  "shared-buffer",
                  thrown(() => crypto.getRandomValues(new Uint8Array(new SharedArrayBuffer(8)))),
                  "TypeError:undefined"
                );
              }

              return JSON.stringify(failures);
            })()
            "#,
        )
        .expect("crypto.getRandomValues boundary probe should evaluate");

    assert_eq!(result, "[]");
}
#[test]
fn crypto_secure_context_exposure_matches_wpt_historical_and_idl() {
    let mut non_secure_vm = new_storage_test_vm("http://example.test/");
    // Chromium WPT: WebCryptoAPI/historical.any.js plus interfaces/webcrypto.idl.
    // randomUUID(), subtle, SubtleCrypto, and CryptoKey are secure-context-only;
    // getRandomValues() remains available on ordinary HTTP origins.
    let non_secure = non_secure_vm
        .eval(
            r#"
            JSON.stringify([
              String(crypto instanceof Crypto),
              typeof crypto.getRandomValues,
              String("randomUUID" in crypto),
              typeof crypto.randomUUID,
              String("randomUUID" in Crypto.prototype),
              String("subtle" in crypto),
              typeof crypto.subtle,
              String("SubtleCrypto" in globalThis),
              typeof globalThis.SubtleCrypto,
              String("CryptoKey" in globalThis),
              typeof globalThis.CryptoKey
            ])
            "#,
        )
        .expect("non-secure WebCrypto historical probe should evaluate");
    assert_eq!(
        non_secure,
        r#"["true","function","false","undefined","false","false","undefined","false","undefined","false","undefined"]"#
    );

    let mut loopback_vm = new_storage_test_vm("http://127.0.0.1:38080/");
    let loopback = loopback_vm
        .eval(r#"String(crypto.subtle instanceof SubtleCrypto && typeof CryptoKey === "function" && typeof crypto.randomUUID === "function")"#)
        .expect("loopback WebCrypto secure-context probe should evaluate");
    assert_eq!(loopback, "true");

    let mut localhost_vm = new_storage_test_vm("http://app.localhost/");
    let localhost = localhost_vm
        .eval(r#"String(crypto.subtle instanceof SubtleCrypto && typeof CryptoKey === "function" && typeof crypto.randomUUID === "function")"#)
        .expect("localhost WebCrypto secure-context probe should evaluate");
    assert_eq!(localhost, "true");
}
#[test]
fn crypto_secure_context_inherited_child_frames_use_creator_origin() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const mount = document.body || document.documentElement || document;
              const blank = document.createElement("iframe");
              mount.appendChild(blank);

              const srcdoc = document.createElement("iframe");
              srcdoc.srcdoc = "<!doctype html><p>child</p>";
              mount.appendChild(srcdoc);

              const surfaces = (win) => [
                typeof win.crypto.randomUUID,
                String(win.crypto.subtle instanceof win.SubtleCrypto),
                String(typeof win.CryptoKey === "function")
              ].join(",");

              return [surfaces(blank.contentWindow), surfaces(srcdoc.contentWindow)].join("|");
            })()
            "#,
        )
        .expect("inherited child frame secure-context probe should evaluate");

    assert_eq!(result, "function,true,true|function,true,true");
}
#[test]
fn crypto_objects_expose_webcrypto_tags_and_illegal_constructors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoTagProbe = [];
          const thrownName = (fn) => {
            try {
              fn();
              return "resolved";
            } catch (error) {
              return error.name;
            }
          };
          crypto.subtle.generateKey(
            { name: "HMAC", hash: "SHA-256" },
            true,
            ["sign"]
          ).then((key) => {
            globalThis.__cryptoTagProbe = [
              String(crypto),
              Object.prototype.toString.call(crypto),
              crypto[Symbol.toStringTag],
              String(crypto.subtle),
              Object.prototype.toString.call(crypto.subtle),
              crypto.subtle[Symbol.toStringTag],
              String(key),
              Object.prototype.toString.call(key),
              key[Symbol.toStringTag],
              String(key.constructor === CryptoKey),
              thrownName(() => new Crypto()),
              thrownName(() => new SubtleCrypto()),
              thrownName(() => new CryptoKey())
            ];
          });
        })()
        "#,
    )
    .expect("crypto tag probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoTagProbe)")
        .expect("crypto tag promise chain should settle");

    assert_eq!(
        result,
        r#"["[object Crypto]","[object Crypto]","Crypto","[object SubtleCrypto]","[object SubtleCrypto]","SubtleCrypto","[object CryptoKey]","[object CryptoKey]","CryptoKey","true","TypeError","TypeError","TypeError"]"#
    );
}
#[test]
fn crypto_key_attributes_use_internal_slots() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKeySlotProbe = [];
          // Chromium implementation reference:
          // third_party/blink/renderer/modules/crypto/crypto_key.cc.
          // The visible algorithm/usages objects are cached per key, but
          // cryptographic operations and structured cloning keep using the
          // immutable internal slots.
          crypto.subtle.generateKey(
            { name: "HMAC", hash: "SHA-256" },
            true,
            ["sign", "verify"]
          ).then(async (key) => {
            const typeErrorName = (fn) => {
              try {
                fn();
                return "resolved";
              } catch (error) {
                return error.name;
              }
            };
            const descriptor = (name) => {
              const d = Object.getOwnPropertyDescriptor(CryptoKey.prototype, name);
              return [
                d.enumerable,
                d.configurable,
                typeof d.get,
                d.get.name,
                d.get.length,
                typeof d.set,
                typeErrorName(() => d.get.call({})),
                Object.prototype.hasOwnProperty.call(key, name)
              ].join(":");
            };
            const algorithm = key.algorithm;
            const usages = key.usages;
            key.type = "public";
            key.extractable = false;
            key.algorithm = { name: "AES-GCM" };
            key.usages = ["verify"];
            algorithm.name = "AES-GCM";
            algorithm.hash.name = "SHA-1";
            algorithm.length = 1;
            usages[0] = "verify";
            usages.push("verify");

            const signature = await crypto.subtle.sign(
              "HMAC",
              key,
              new TextEncoder().encode("slot integrity")
            );
            const verified = await crypto.subtle.verify(
              "HMAC",
              key,
              signature,
              new TextEncoder().encode("slot integrity")
            );
            const jwk = await crypto.subtle.exportKey("jwk", key);
            const clone = structuredClone(key);
            globalThis.__cryptoKeySlotProbe = [
              String(key instanceof CryptoKey),
              String(key.constructor === CryptoKey),
              Object.getOwnPropertyNames(key).join(","),
              descriptor("type"),
              descriptor("extractable"),
              descriptor("algorithm"),
              descriptor("usages"),
              String(key.algorithm === key.algorithm),
              String(key.usages === key.usages),
              [key.type, key.extractable, key.algorithm.name, key.algorithm.hash.name, key.algorithm.length].join(":"),
              key.usages.join(","),
              String(verified),
              [jwk.alg, jwk.key_ops.join(",")].join(":"),
              [clone.type, clone.extractable, clone.algorithm.name, clone.algorithm.hash.name, clone.algorithm.length].join(":"),
              clone.usages.join(",")
            ];
          });
        })()
        "#,
    )
    .expect("crypto key internal slot probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKeySlotProbe)")
        .expect("crypto key internal slot promise should settle");

    assert_eq!(
        result,
        r#"["true","true","","true:true:function:get type:0:undefined:TypeError:false","true:true:function:get extractable:0:undefined:TypeError:false","true:true:function:get algorithm:0:undefined:TypeError:false","true:true:function:get usages:0:undefined:TypeError:false","true","true","secret:true:AES-GCM:SHA-1:1","verify,verify,verify","true","HS256:sign,verify","secret:true:HMAC:SHA-256:512","sign,verify"]"#
    );
}
#[test]
fn crypto_key_structured_clone_preserves_internal_slots() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoKeyCloneProbe = [];
          const sameBytes = (left, right) => {
            const a = new Uint8Array(left);
            const b = new Uint8Array(right);
            return a.length === b.length && a.every((value, index) => value === b[index]);
          };
          const rejectName = promise => promise.then(
            () => "resolved",
            error => error.name
          );
          Promise.resolve().then(async () => {
            // Chromium WebCrypto cloneKey tests expect CryptoKey structured
            // cloning to preserve the key's internal slots while dropping page
            // script expando properties.
            const hmac = await crypto.subtle.importKey(
              "raw",
              new Uint8Array([0x30, 0x11, 0x22, 0x33]),
              { name: "HMAC", hash: "SHA-256" },
              false,
              ["sign", "verify"]
            );
            hmac.extraProperty = "hi";
            const hmacClone = structuredClone(hmac);
            const data = new TextEncoder().encode("cloned key");
            const signature = await crypto.subtle.sign("HMAC", hmac, data);
            const verified = await crypto.subtle.verify("HMAC", hmacClone, signature, data);
            const hmacExport = await rejectName(crypto.subtle.exportKey("raw", hmacClone));

            const aesBytes = new Uint8Array([
              0x30, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
              0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff
            ]);
            const aes = await crypto.subtle.importKey(
              "raw",
              aesBytes,
              "AES-GCM",
              true,
              ["decrypt", "encrypt", "wrapKey"]
            );
            const aesClone = structuredClone(aes);
            const aesRoundTrip = sameBytes(await crypto.subtle.exportKey("raw", aesClone), aesBytes);

            const hkdf = await crypto.subtle.importKey(
              "raw",
              new Uint8Array([1, 2, 3, 4]),
              "HKDF",
              false,
              ["deriveBits"]
            );
            const hkdfClone = structuredClone(hkdf);
            const hkdfParams = {
              name: "HKDF",
              hash: "SHA-256",
              salt: new Uint8Array([5, 6, 7, 8]),
              info: new Uint8Array([9, 10])
            };
            const hkdfSame = sameBytes(
              await crypto.subtle.deriveBits(hkdfParams, hkdf, 128),
              await crypto.subtle.deriveBits(hkdfParams, hkdfClone, 128)
            );

            globalThis.__cryptoKeyCloneProbe = [
              String(hmacClone instanceof CryptoKey),
              String(hmacClone !== hmac),
              String(hmacClone.extraProperty),
              [hmacClone.type, hmacClone.extractable, hmacClone.algorithm.name, hmacClone.algorithm.hash.name, hmacClone.algorithm.length].join(":"),
              hmacClone.usages.join(","),
              String(verified),
              hmacExport,
              [aesClone.algorithm.name, aesClone.algorithm.length, aesClone.usages.join(","), aesRoundTrip].join(":"),
              [hkdfClone.algorithm.name, hkdfClone.extractable, hkdfClone.usages.join(","), hkdfSame].join(":")
            ];
          });
        })()
        "#,
    )
    .expect("crypto key structured clone probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoKeyCloneProbe)")
        .expect("crypto key structured clone promise should settle");

    assert_eq!(
        result,
        r#"["true","true","undefined","secret:false:HMAC:SHA-256:32","sign,verify","true","InvalidAccessError","AES-GCM:128:encrypt,decrypt,wrapKey:true","HKDF:false:deriveBits:true"]"#
    );
}
#[test]
fn crypto_key_structured_clone_matches_chromium_symmetric_matrix() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
          globalThis.__cryptoSymmetricCloneFailures = ["pending"];
          (async () => {
            const subtle = crypto.subtle;
            const failures = [];
            const hexBytes = (hex) => {
              const bytes = new Uint8Array(hex.length / 2);
              for (let index = 0; index < bytes.length; index += 1) {
                bytes[index] = parseInt(hex.slice(index * 2, index * 2 + 2), 16);
              }
              return bytes;
            };
            const sameBytes = (left, right) => {
              const a = new Uint8Array(left);
              const b = new Uint8Array(right);
              return a.length === b.length && a.every((value, index) => value === b[index]);
            };
            const expect = (condition, label) => {
              if (!condition) failures.push(label);
            };
            const operationBytes = async (algorithmName, key, importBytes) => {
              if (algorithmName.startsWith("AES-")) {
                return null;
              }
              if (algorithmName === "HMAC") {
                if (key.usages.includes("sign")) {
                  return subtle.sign("HMAC", key, new Uint8Array([1, 2, 3]));
                }
                const signer = await subtle.importKey(
                  "raw",
                  importBytes,
                  { name: "HMAC", hash: key.algorithm.hash.name },
                  false,
                  ["sign"]
                );
                const signature = await subtle.sign("HMAC", signer, new Uint8Array([1, 2, 3]));
                return new Uint8Array([await subtle.verify("HMAC", key, signature, new Uint8Array([1, 2, 3])) ? 1 : 0]);
              }
              if (key.usages.includes("deriveBits")) {
                const params = algorithmName === "HKDF"
                  ? { name: "HKDF", hash: "SHA-256", salt: new Uint8Array([4, 5]), info: new Uint8Array([6]) }
                  : { name: "PBKDF2", hash: "SHA-256", salt: new Uint8Array([4, 5]), iterations: 2 };
                return subtle.deriveBits(params, key, 128);
              }
              const params = algorithmName === "HKDF"
                ? { name: "HKDF", hash: "SHA-256", salt: new Uint8Array([4, 5]), info: new Uint8Array([6]) }
                : { name: "PBKDF2", hash: "SHA-256", salt: new Uint8Array([4, 5]), iterations: 2 };
              const derived = await subtle.deriveKey(
                params,
                key,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
              );
              return subtle.exportKey("raw", derived);
            };
            const checkClone = async ({ algorithmName, importAlgorithm, extractable, usages, keyDataHex, hasLength }) => {
              const importBytes = hexBytes(keyDataHex);
              const imported = await subtle.importKey(
                "raw",
                importBytes,
                importAlgorithm,
                extractable,
                usages
              );
              imported.extraProperty = "hi";
              const cloned = structuredClone(imported);
              const label = `${algorithmName}:${keyDataHex || "empty"}:${extractable}:${usages.join("+")}`;
              expect(imported.extraProperty === "hi", `${label}:source-expando`);
              expect(cloned !== imported, `${label}:identity`);
              expect(cloned.extraProperty === undefined, `${label}:clone-expando`);
              expect(cloned.type === "secret", `${label}:type`);
              expect(cloned.extractable === extractable, `${label}:extractable`);
              expect(cloned.algorithm.name === algorithmName, `${label}:algorithm`);
              if (hasLength) {
                expect(cloned.algorithm.length === importBytes.byteLength * 8, `${label}:length`);
              }
              if (algorithmName === "HMAC") {
                expect(cloned.algorithm.hash.name === importAlgorithm.hash.name, `${label}:hash`);
              }
              expect(cloned.usages.join(",") === imported.usages.join(","), `${label}:usages`);
              if (extractable) {
                expect(sameBytes(await subtle.exportKey("raw", cloned), importBytes), `${label}:export`);
              }
              const importedOperation = await operationBytes(algorithmName, imported, importBytes);
              if (importedOperation !== null) {
                expect(
                  sameBytes(
                    importedOperation,
                    await operationBytes(algorithmName, cloned, importBytes)
                  ),
                  `${label}:operation`
                );
              }
            };

            // Chromium legacy tests crypto/subtle/*/cloneKey.html cover a
            // matrix of symmetric key sizes, usages, and extractability. Keep
            // the local port compact, but cover every supported symmetric key
            // family. The empty HKDF/PBKDF2 cases matter because crawlers can
            // persist and later clone those keys.
            const aesData = [
              "30112233445566778899aabbccddeeff",
              "00112233445546778899aabbccddeeff000102030405060708090a0b0c0d0e0f"
            ];
            const aesCases = [
              { algorithmName: "AES-CBC", usages: [["encrypt"], ["decrypt", "wrapKey"], ["encrypt", "wrapKey", "unwrapKey"]] },
              { algorithmName: "AES-CTR", usages: [["wrapKey", "unwrapKey"]] },
              { algorithmName: "AES-GCM", usages: [["encrypt"], ["decrypt", "wrapKey"], ["encrypt", "wrapKey", "unwrapKey"]] },
              { algorithmName: "AES-KW", usages: [["wrapKey", "unwrapKey"]] }
            ];
            for (const { algorithmName, usages: usageCases } of aesCases) {
              for (const extractable of [true, false]) {
                for (const usages of usageCases) {
                  for (const keyDataHex of aesData) {
                    await checkClone({
                      algorithmName,
                      importAlgorithm: { name: algorithmName },
                      extractable,
                      usages,
                      keyDataHex,
                      hasLength: true
                    });
                  }
                }
              }
            }

            const hmacData = ["30", "0011223344554677", "30112233445566778899aabbccddeeff"];
            for (const hash of ["SHA-1", "SHA-256", "SHA-512"]) {
              for (const extractable of [true, false]) {
                for (const usages of [["sign"], ["verify"], ["sign", "verify"]]) {
                  for (const keyDataHex of hmacData) {
                    await checkClone({
                      algorithmName: "HMAC",
                      importAlgorithm: { name: "HMAC", hash: { name: hash } },
                      extractable,
                      usages,
                      keyDataHex,
                      hasLength: true
                    });
                  }
                }
              }
            }

            const kdfData = ["", "30", "30112233445566778899aabbccddeeff"];
            for (const algorithmName of ["HKDF", "PBKDF2"]) {
              for (const usages of [["deriveBits"], ["deriveKey"], ["deriveKey", "deriveBits"]]) {
                for (const keyDataHex of kdfData) {
                  await checkClone({
                    algorithmName,
                    importAlgorithm: { name: algorithmName, hash: { name: "SHA-256" } },
                    extractable: false,
                    usages,
                    keyDataHex,
                    hasLength: false
                  });
                }
              }
            }

            globalThis.__cryptoSymmetricCloneFailures = failures;
          })();
        })()
        "#,
    )
    .expect("crypto key symmetric clone matrix probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__cryptoSymmetricCloneFailures)")
        .expect("crypto key symmetric clone matrix promise chain should settle");

    assert_eq!(result, "[]");
}
#[test]
fn child_initial_about_blank_executes_dynamic_inline_script_synchronously() {
    let mut vm = new_storage_test_vm("https://webcrypto-context-detach.test/");

    let setup = vm
        .eval(
            r#"
            (() => {
              globalThis.__childDynamicScriptEvents = [];
              globalThis.__closeChildDynamicScriptFrame = (frame) => {
                __childDynamicScriptEvents.push("getter");
                frame.parentNode.removeChild(frame);
              };

              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const childBody = frame.contentDocument.body;

              const script = document.createElement("script");
              script.textContent = `
                top.__childDynamicScriptEvents.push(
                  "start:" + String(crypto.subtle instanceof SubtleCrypto)
                );
                const algorithm = { name: "SHA-256" };
                Object.defineProperty(algorithm, "name", {
                  get() {
                    top.__closeChildDynamicScriptFrame(frameElement);
                    return "SHA-256";
                  }
                });
                Promise.resolve().then(() => {
                  top.__childDynamicScriptEvents.push("microtask");
                });
                crypto.subtle.digest(algorithm, new Uint8Array());
                top.__childDynamicScriptEvents.push("after-call");
              `;
              childBody.appendChild(script);
              top.__childDynamicScriptEvents.push("after-append");
              return "append-returned";
            })()
        "#,
        )
        .expect("dynamic child script setup should evaluate");
    assert_eq!(setup, "append-returned");

    let result = vm
        .eval("JSON.stringify(globalThis.__childDynamicScriptEvents)")
        .expect("dynamic child script events should be readable");

    assert_eq!(
        result, r#"["start:true","getter","after-call","after-append","microtask"]"#,
        "child inline classic execution must finish before appendChild returns, while its Promise callback waits for the outer script turn's microtask checkpoint"
    );
}
