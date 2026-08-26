use super::*;

#[tokio::test]
async fn worker_postmessage_to_parent() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"postMessage("hello from worker");"#.into(),
        "test://postmessage".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""hello from worker""#);
}

#[tokio::test]
async fn worker_postmessage_object() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"postMessage({ x: 1, y: "two" });"#.into(),
        "test://postmessage_obj".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let json = expect_post_json(msg);
    assert!(
        json.contains(r#""x":1"#) || json.contains(r#""x": 1"#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""y":"two""#) || json.contains(r#""y": "two""#),
        "json: {json}"
    );
}

#[tokio::test]
async fn worker_resource_owner_uses_context_slot_not_isolate_slot() {
    ensure_v8();
    let handle = spawn_worker(
        r#"postMessage("ready");"#.into(),
        "test://owner-slot".into(),
    );

    let diagnostics = timeout(TIMEOUT, handle.resource_owner_slot_diagnostics())
        .await
        .expect("timed out waiting for worker resource-owner slot diagnostics")
        .expect("worker resource-owner slot diagnostics should complete");

    assert!(
        diagnostics.context_slot_has_owner,
        "worker ResourceOwnerId must be installed on the worker global context"
    );
    assert!(
        diagnostics.current_owner_matches_context,
        "current_resource_owner_id must read the worker owner from the current context slot"
    );
    assert!(
        !diagnostics.isolate_slot_has_owner,
        "worker ResourceOwnerId must not be written to the V8 isolate slot"
    );
    assert!(
        !diagnostics.opfs_owner_state_materialized,
        "an unrelated worker must not eagerly allocate OPFS owner state"
    );
    assert_eq!(
        diagnostics.storage_constructor_materializations, 0,
        "an unrelated worker must not materialize storage constructors"
    );
    assert!(
        !diagnostics.storage_manager_materialized,
        "an unrelated worker must not eagerly create navigator.storage"
    );
    assert!(
        !diagnostics.storage_bucket_manager_materialized,
        "an unrelated worker must not eagerly create navigator.storageBuckets"
    );
}

#[tokio::test]
async fn worker_crypto_subtle_hmac_surface_matches_webcrypto_any_tests() {
    ensure_v8();
    // Chromium's WebCrypto WPT coverage is mostly `any.js`; keep a worker
    // smoke test so future runtime-state changes do not accidentally make the
    // worker global diverge from the window WebCrypto surface.
    let mut handle = spawn_worker(
        r#"
        (async () => {
            const failures = [];
            const typeErrorName = (fn) => {
                try {
                    fn();
                    return "resolved";
                } catch (error) {
                    return error.name;
                }
            };
            if (!(crypto instanceof Crypto)) {
                failures.push("crypto-instance");
            }
            if (!(crypto.subtle instanceof SubtleCrypto)) {
                failures.push("subtle-instance");
            }
            if (typeof CryptoKey !== "function") {
                failures.push("cryptokey-interface");
            }
            if (typeof SubtleCrypto.supports !== "function") {
                failures.push("supports-static");
            }
            const workerCryptoDescriptor = Object.getOwnPropertyDescriptor(
                WorkerGlobalScope.prototype,
                "crypto"
            );
            if (Object.prototype.hasOwnProperty.call(self, "crypto")) {
                failures.push("crypto-own-property");
            }
            if (typeof workerCryptoDescriptor?.get !== "function" || workerCryptoDescriptor.enumerable !== true) {
                failures.push("worker-crypto-descriptor");
            }
            if (crypto !== self.crypto || crypto !== workerCryptoDescriptor.get.call(self)) {
                failures.push("crypto-sameobject");
            }
            if (typeErrorName(() => workerCryptoDescriptor.get.call(WorkerGlobalScope.prototype)) !== "TypeError") {
                failures.push("worker-crypto-prototype-brand");
            }
            if (typeErrorName(() => Crypto.prototype.randomUUID.call({})) !== "TypeError") {
                failures.push("randomUUID-brand");
            }
            const key = await crypto.subtle.generateKey(
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign", "verify"]
            );
            const algorithmA = key.algorithm;
            const algorithmB = key.algorithm;
            const usagesA = key.usages;
            const usagesB = key.usages;
            if (algorithmA !== algorithmB || algorithmA.hash !== algorithmB.hash) {
                failures.push("algorithm-cached-wrapper");
            }
            if (usagesA !== usagesB) {
                failures.push("usages-cached-wrapper");
            }
            if (algorithmA.name !== "HMAC" || algorithmA.hash.name !== "SHA-256") {
                failures.push("algorithm-shape");
            }
            if (usagesA.join(",") !== "sign,verify") {
                failures.push("usages-shape");
            }
            const data = new TextEncoder().encode("worker hmac");
            const signature = await crypto.subtle.sign("HMAC", key, data);
            if (!await crypto.subtle.verify("HMAC", key, signature, data)) {
                failures.push("hmac-verify");
            }
            const uuid = crypto.randomUUID();
            const namespaceFormat = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/;
            const version = parseInt(uuid.split("-")[2].slice(0, 2), 16) & 0b11110000;
            const variant = parseInt(uuid.split("-")[3].slice(0, 2), 16) & 0b11000000;
            if (!namespaceFormat.test(uuid) || version !== 0b01000000 || variant !== 0b10000000) {
                failures.push("randomUUID-shape");
            }
            postMessage(failures);
        })().catch((error) => {
            postMessage(["error:" + error.name + ":" + error.message]);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-hmac-surface.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let payload = expect_post_json(msg);
    handle.terminate_and_join();
    assert_eq!(payload, "[]");
}

#[tokio::test]
async fn worker_crypto_subtle_derive_bits_pbkdf2_uses_async_lane_and_matches_vector() {
    ensure_v8();
    // Worker WebCrypto deriveBits routes heavy KDF work through the worker-owned
    // async completion lane (spawn_blocking + completion channel) instead of the
    // synchronous callback fallback. Assert the lane resolves the well-known
    // RFC-style PBKDF2-HMAC-SHA256 vector so the off-loop path stays correct.
    let mut handle = spawn_worker(
        r#"
        (async () => {
            const enc = new TextEncoder();
            const baseKey = await crypto.subtle.importKey(
                "raw",
                enc.encode("password"),
                { name: "PBKDF2" },
                false,
                ["deriveBits"]
            );
            const bits = await crypto.subtle.deriveBits(
                {
                    name: "PBKDF2",
                    salt: enc.encode("salt"),
                    iterations: 1,
                    hash: "SHA-256"
                },
                baseKey,
                256
            );
            const hex = Array.from(new Uint8Array(bits))
                .map((b) => b.toString(16).padStart(2, "0"))
                .join("");
            postMessage(hex);
        })().catch((error) => {
            postMessage("error:" + error.name + ":" + error.message);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-derive-bits-pbkdf2.js".into(),
    );

    let payload = recv_post_json(&mut handle).await;
    assert_eq!(
        payload,
        r#""120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b""#,
    );
}

#[tokio::test]
async fn worker_crypto_subtle_derive_bits_does_not_block_event_loop() {
    ensure_v8();
    // The point of the worker async lane is that heavy crypto runs on the
    // blocking pool, so the worker event loop keeps servicing other tasks while
    // a deriveBits is in flight. A concurrently scheduled timer must be able to
    // fire before the (intentionally expensive) derivation resolves.
    let mut handle = spawn_worker(
        r#"
        let timerFiredBeforeDerive = false;
        let deriveResolved = false;
        setTimeout(() => {
            timerFiredBeforeDerive = !deriveResolved;
        }, 0);
        (async () => {
            const enc = new TextEncoder();
            const baseKey = await crypto.subtle.importKey(
                "raw",
                enc.encode("password"),
                { name: "PBKDF2" },
                false,
                ["deriveBits"]
            );
            await crypto.subtle.deriveBits(
                {
                    name: "PBKDF2",
                    salt: enc.encode("salt"),
                    iterations: 1000000,
                    hash: "SHA-256"
                },
                baseKey,
                256
            );
            deriveResolved = true;
            postMessage(timerFiredBeforeDerive ? "timer-first" : "derive-first");
        })().catch((error) => {
            postMessage("error:" + error.name + ":" + error.message);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-derive-bits-nonblocking.js".into(),
    );

    let payload = recv_post_json(&mut handle).await;
    assert_eq!(payload, r#""timer-first""#);
}

#[tokio::test]
async fn worker_crypto_subtle_derive_bits_processes_incoming_message_while_pending() {
    ensure_v8();
    // Beyond timers, the worker must keep handling parent postMessage traffic
    // while a heavy derivation is in flight. Start a 1,000,000-iteration
    // deriveBits, then post a message from the parent; the worker should
    // respond to that message before the derivation resolves.
    let mut handle = spawn_worker(
        r#"
        let deriveResolved = false;
        let pingHandledBeforeDerive = null;
        onmessage = (event) => {
            if (event.data === "ping") {
                pingHandledBeforeDerive = !deriveResolved;
                postMessage(pingHandledBeforeDerive ? "ping-first" : "ping-after-derive");
            }
        };
        (async () => {
            const enc = new TextEncoder();
            const baseKey = await crypto.subtle.importKey(
                "raw", enc.encode("password"), { name: "PBKDF2" }, false, ["deriveBits"]
            );
            await crypto.subtle.deriveBits(
                { name: "PBKDF2", salt: enc.encode("salt"), iterations: 1000000, hash: "SHA-256" },
                baseKey, 256
            );
            deriveResolved = true;
        })().catch((error) => {
            postMessage("error:" + error.name + ":" + error.message);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-derive-bits-message-responsive.js".into(),
    );

    // Give the worker a moment to install onmessage and start the derivation,
    // then send the ping. The derivation is intentionally long enough that the
    // message must be serviced off the critical path.
    handle.post_message(serialize_test_string("ping"));
    let payload = recv_post_json(&mut handle).await;
    // The responsiveness contract ends while PBKDF2 is still in flight.
    // Terminate and join that live worker explicitly so this test also proves
    // teardown drops a late WebCrypto completion without racing process-wide
    // V8 shutdown.
    handle.terminate_and_join();
    assert_eq!(payload, r#""ping-first""#);
}

#[tokio::test]
async fn worker_crypto_get_random_values_quota_exceeded_error_matches_wpt_any() {
    ensure_v8();
    // WPT's getRandomValues.any.js uses assert_throws_quotaexceedederror(),
    // which checks the modern QuotaExceededError constructor/prototype shape
    // in the worker realm, not just DOMException.name/code.
    let mut handle = spawn_worker(
        r#"
        try {
            crypto.getRandomValues(new Uint8Array(65537));
            postMessage("no-throw");
        } catch (error) {
            postMessage([
                error.name,
                error.code,
                error.constructor === QuotaExceededError,
                error instanceof QuotaExceededError,
                error instanceof DOMException,
                Object.getPrototypeOf(QuotaExceededError.prototype) === DOMException.prototype
            ].join("|"));
        }
        close();
        "#
        .into(),
        "https://worker-crypto.test/get-random-values-quota.js".into(),
    );

    let payload = recv_post_json(&mut handle).await;
    assert_eq!(payload, r#""QuotaExceededError|22|true|true|true|true""#);
}

#[tokio::test]
async fn worker_structured_clone_global_clones_plain_data_for_wpt_vectors() {
    ensure_v8();
    // WebCrypto's ECDSA WPT vector helper uses structuredClone() while running
    // in both window and worker globals. Keep the worker surface wired to the
    // same structured-clone backend as postMessage/history state.
    let mut handle = spawn_worker(
        r#"
        const source = {
            nested: { value: 7 },
            bytes: new Uint8Array([1, 2, 3]).buffer
        };
        const clone = structuredClone(source);
        source.nested.value = 9;
        new Uint8Array(source.bytes)[0] = 99;
        postMessage([
            typeof structuredClone,
            clone !== source,
            clone.nested !== source.nested,
            clone.nested.value,
            Array.from(new Uint8Array(clone.bytes)).join(",")
        ].join("|"));
        close();
        "#
        .into(),
        "https://worker-crypto.test/structured-clone-global.js".into(),
    );

    let payload = recv_post_json(&mut handle).await;
    assert_eq!(payload, r#""function|true|true|7|1,2,3""#);
}

#[tokio::test]
async fn worker_crypto_subtle_hmac_parallel_operations_match_chromium_worker_intent() {
    ensure_v8();
    // Chromium legacy test: crypto/subtle/worker-subtle-crypto-concurrent.html.
    // Chromium's original worker stress case mixes HMAC, RSA, and AES-GCM. In
    // this no-new-deps branch Moli only has vetted HMAC primitives, so
    // keep the same concurrency shape but scope it to supported HMAC work.
    const WORKER_SOURCE: &str = r#"
        const hexBytes = (value) => value.length === 0
            ? new Uint8Array([])
            : new Uint8Array(value.match(/../g).map((byte) => parseInt(byte, 16)));
        const sameBytes = (left, right) => {
            const a = new Uint8Array(left);
            const b = new Uint8Array(right);
            return a.length === b.length && a.every((value, index) => value === b[index]);
        };
        const vector = {
            hash: "SHA-256",
            key: "9779d9120642797f1747025d5b22b7ac607cab08e1758f2f3a46c8be1e25c53b8c6a8f58ffefa176",
            message: "b1689c2591eaf3c9e66070f8a77954ffb81749f1b00346f9dfe0b2ee905dcc288baf4a92de3f4001dd9f44c468c3d07d6c6ee82faceafc97c2fc0fc0601719d2dcd0aa2aec92d1b0ae933c65eb06a03c9c935c2bad0459810241347ab87e9f11adb30415424c6c7f5f22a003b8ab8de54f6ded0e3ab9245fa79568451dfa258e",
            mac: "769f00d3e6a6cc1fb426a14a4f76c6462e6149726e0dee0ec0cf97a16605ac8b"
        };
        async function runHmacRound(index) {
            const failures = [];
            const keyBytes = hexBytes(vector.key);
            const messageBytes = hexBytes(vector.message);
            const macBytes = hexBytes(vector.mac);
            const key = await crypto.subtle.importKey(
                "raw",
                keyBytes,
                { name: "HMAC", hash: { name: vector.hash } },
                true,
                ["verify", "sign"]
            );
            if (
                key.type !== "secret" ||
                key.extractable !== true ||
                key.algorithm.name !== "HMAC" ||
                key.algorithm.hash.name !== vector.hash ||
                key.algorithm.length !== keyBytes.byteLength * 8 ||
                key.usages.join(",") !== "sign,verify"
            ) {
                failures.push(`shape:${index}`);
            }
            const [signature, verified, truncatedVerified, raw, jwk, generated] = await Promise.all([
                crypto.subtle.sign("HMAC", key, messageBytes),
                crypto.subtle.verify("HMAC", key, macBytes, messageBytes),
                crypto.subtle.verify("HMAC", key, macBytes.slice(0, macBytes.byteLength - 1), messageBytes),
                crypto.subtle.exportKey("raw", key),
                crypto.subtle.exportKey("jwk", key),
                crypto.subtle.generateKey({ name: "HMAC", hash: "SHA-1", length: 40 }, true, ["sign"])
            ]);
            if (!sameBytes(signature, macBytes)) failures.push(`sign:${index}`);
            if (verified !== true) failures.push(`verify:${index}`);
            if (truncatedVerified !== false) failures.push(`truncated:${index}`);
            if (!sameBytes(raw, keyBytes)) failures.push(`raw:${index}`);
            if (
                jwk.kty !== "oct" ||
                jwk.alg !== "HS256" ||
                jwk.ext !== true ||
                jwk.key_ops.join(",") !== "sign,verify" ||
                typeof jwk.k !== "string"
            ) {
                failures.push(`jwk:${index}`);
            }
            if (
                generated.type !== "secret" ||
                generated.algorithm.name !== "HMAC" ||
                generated.algorithm.hash.name !== "SHA-1" ||
                generated.algorithm.length !== 40 ||
                generated.usages.join(",") !== "sign"
            ) {
                failures.push(`generated:${index}`);
            }
            return failures;
        }
        (async () => {
            const results = await Promise.all(Array.from({ length: 8 }, (_, index) => runHmacRound(index)));
            postMessage([].concat(...results));
        })().catch((error) => {
            postMessage(["error:" + error.name + ":" + error.message]);
        });
    "#;
    let handles = (0..4)
        .map(|index| {
            spawn_worker(
                WORKER_SOURCE.into(),
                format!("https://worker-crypto.test/subtle-hmac-parallel-{index}.js"),
            )
        })
        .collect::<Vec<_>>();

    for mut handle in handles {
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(expect_post_json(msg), "[]");
        handle.terminate_and_join();
    }
}

#[tokio::test]
async fn worker_crypto_subtle_chacha20_poly1305_matches_tentative_any_intent() {
    ensure_v8();
    // The current ChaCha20-Poly1305 WebCrypto surface is still based on
    // tentative WPT, but it is exposed in both window and worker globals. Keep
    // a compact worker smoke for key import/export, AEAD, and wrapping.
    let mut handle = spawn_worker(
        r#"
        const sameBytes = (left, right) => {
            const a = new Uint8Array(left);
            const b = new Uint8Array(right);
            return a.length === b.length && a.every((value, index) => value === b[index]);
        };
        (async () => {
            const failures = [];
            const subtle = crypto.subtle;
            const keyBytes = new Uint8Array([
                0x60, 0x3d, 0xeb, 0x10, 0x15, 0xca, 0x71, 0xbe,
                0x2b, 0x73, 0xae, 0xf0, 0x85, 0x7d, 0x77, 0x81,
                0x1f, 0x35, 0x2c, 0x07, 0x3b, 0x61, 0x08, 0xd7,
                0x2d, 0x98, 0x10, 0xa3, 0x09, 0x14, 0xdf, 0xf4
            ]);
            const iv = new Uint8Array([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x4a, 0x00, 0x00, 0x00, 0x00
            ]);
            const additionalData = new TextEncoder().encode("worker chacha aad");
            const data = new TextEncoder().encode("worker chacha plaintext");
            const key = await subtle.importKey(
                "raw-secret",
                keyBytes,
                "ChaCha20-Poly1305",
                true,
                ["encrypt", "decrypt", "wrapKey", "unwrapKey"]
            );
            if (
                key.type !== "secret" ||
                key.extractable !== true ||
                key.algorithm.name !== "ChaCha20-Poly1305" ||
                "length" in key.algorithm ||
                key.usages.join(",") !== "encrypt,decrypt,wrapKey,unwrapKey"
            ) {
                failures.push("shape");
            }
            if (!sameBytes(await subtle.exportKey("raw-secret", key), keyBytes)) {
                failures.push("raw-secret-export");
            }
            const algorithm = {
                name: "ChaCha20-Poly1305",
                iv,
                additionalData,
                tagLength: 128
            };
            const ciphertext = await subtle.encrypt(algorithm, key, data);
            if (!sameBytes(await subtle.decrypt(algorithm, key, ciphertext), data)) {
                failures.push("roundtrip");
            }

            const hmac = await subtle.importKey(
                "raw",
                new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]),
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
            );
            const wrapped = await subtle.wrapKey("raw", hmac, key, algorithm);
            const unwrapped = await subtle.unwrapKey(
                "raw",
                wrapped,
                key,
                algorithm,
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
            );
            if (!sameBytes(await subtle.exportKey("raw", hmac), await subtle.exportKey("raw", unwrapped))) {
                failures.push("wrap-unwrap");
            }

            postMessage(failures);
        })().catch((error) => {
            postMessage(["error:" + error.name + ":" + error.message]);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-chacha20-poly1305.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "[]");
}

#[tokio::test]
async fn worker_postmessage_accepts_cryptokey_from_secure_parent() {
    ensure_v8();
    // Chromium WPT: webmessaging/resources/post-cryptokey-to-opener.html uses
    // CryptoKey as a postMessage host object. This test exercises the same
    // structured-clone boundary in the parent -> dedicated-worker direction and
    // verifies that the deserialized key still drives SubtleCrypto operations.
    let mut handle = spawn_worker(
        r#"
        onmessage = async event => {
            const failures = [];
            const key = event.data && event.data.key;
            const data = new Uint8Array([1, 2, 3, 4]);
            if (!(key instanceof CryptoKey)) failures.push("instance");
            if (key.extraProperty !== undefined) failures.push("expando");
            if (
                key.type !== "secret" ||
                key.extractable !== true ||
                key.algorithm.name !== "HMAC" ||
                key.algorithm.hash.name !== "SHA-256" ||
                key.algorithm.length !== 32 ||
                key.usages.join(",") !== "sign,verify"
            ) {
                failures.push("shape");
            }
            const signature = await crypto.subtle.sign("HMAC", key, data);
            if (!await crypto.subtle.verify("HMAC", key, signature, data)) {
                failures.push("verify");
            }
            postMessage([
                ...failures,
                new Uint8Array(signature).byteLength
            ]);
        };
        "#
        .into(),
        "https://worker-crypto.test/receive-cryptokey.js".into(),
    );

    let payload = serialize_test_crypto_value(
        r#"
        (async () => {
            const key = await crypto.subtle.importKey(
                "raw",
                new Uint8Array([0x30, 0x11, 0x22, 0x33]),
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign", "verify"]
            );
            key.extraProperty = "source-only";
            key.algorithm.name = "AES-GCM";
            key.usages.push("encrypt");
            return { key };
        })()
        "#,
    );
    handle.post_message(payload);

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "[32]");
}

#[tokio::test]
async fn worker_postmessage_cryptokey_to_nonsecure_worker_fires_messageerror() {
    ensure_v8();
    // Chromium WPT: webmessaging/postMessage_CryptoKey_insecure.sub.html.
    // A receiver that does not expose secure-context-only `CryptoKey` cannot
    // deserialize the host object; the observable delivery result is
    // `messageerror`, not a successful `message` with missing data.
    let mut handle = spawn_worker(
        r#"
        onmessage = event => {
            postMessage(["message", event.type, event.data === undefined]);
        };
        onmessageerror = event => {
            postMessage([
                "messageerror",
                event.type,
                event.data === null,
                String("CryptoKey" in globalThis),
                typeof crypto.subtle
            ]);
        };
        "#
        .into(),
        "http://example.test/receive-cryptokey-insecure.js".into(),
    );

    let payload = serialize_test_crypto_value(
        r#"
        (async () => ({
            key: await crypto.subtle.importKey(
                "raw",
                new Uint8Array([0x30, 0x11, 0x22, 0x33]),
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
            )
        }))()
        "#,
    );
    handle.post_message(payload);

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"["messageerror","messageerror",true,"false","undefined"]"#
    );
}

#[tokio::test]
async fn worker_crypto_subtle_kdf_and_x25519_derivation_match_wpt_any_tests() {
    ensure_v8();
    // Chromium WPT marks WebCrypto derivation tests as `any.js`, so supported
    // derivation paths must work from dedicated workers as well as window
    // globals. Keep this worker test compact and vector-driven to cover the
    // same backend paths without duplicating the full window matrix.
    let mut handle = spawn_worker(
        r#"
        const sameBytes = (left, right) => {
            const a = new Uint8Array(left);
            const b = new Uint8Array(right);
            return a.length === b.length && a.every((value, index) => value === b[index]);
        };
        (async () => {
            const failures = [];
            const subtle = crypto.subtle;

            // Chromium legacy test:
            // crypto/subtle/hkdf/deriveBits-rfc5869-test-vectors.html.
            const hkdfInput = new Uint8Array([
                0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b,
                0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b,
                0x0b, 0x0b, 0x0b, 0x0b, 0x0b, 0x0b
            ]);
            const hkdfSalt = new Uint8Array([
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
                0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c
            ]);
            const hkdfInfo = new Uint8Array([
                0xf0, 0xf1, 0xf2, 0xf3, 0xf4,
                0xf5, 0xf6, 0xf7, 0xf8, 0xf9
            ]);
            const hkdfExpected = new Uint8Array([
                0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a,
                0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36, 0x2f, 0x2a,
                0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c,
                0x5d, 0xb0, 0x2d, 0x56, 0xec, 0xc4, 0xc5, 0xbf,
                0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18,
                0x58, 0x65
            ]);
            const hkdfKey = await subtle.importKey("raw", hkdfInput, "HKDF", false, ["deriveBits", "deriveKey"]);
            const hkdfParams = { name: "HKDF", hash: "SHA-256", salt: hkdfSalt, info: hkdfInfo };
            const hkdfBits = await subtle.deriveBits(hkdfParams, hkdfKey, hkdfExpected.byteLength * 8);
            if (!sameBytes(hkdfBits, hkdfExpected)) {
                failures.push("hkdf:deriveBits");
            }
            const hkdfHmac = await subtle.deriveKey(
                hkdfParams,
                hkdfKey,
                { name: "HMAC", hash: "SHA-256", length: 128 },
                true,
                ["sign"]
            );
            if (!sameBytes(await subtle.exportKey("raw", hkdfHmac), hkdfExpected.slice(0, 16))) {
                failures.push("hkdf:deriveKey:hmac");
            }

            // Chromium legacy test:
            // crypto/subtle/pbkdf2/deriveBits-rfc6070-test-vectors.html.
            const pbkdf2Password = new Uint8Array([112, 97, 115, 115, 119, 111, 114, 100]);
            const pbkdf2Salt = new Uint8Array([115, 97, 108, 116]);
            const pbkdf2Expected = new Uint8Array([
                0x0c, 0x60, 0xc8, 0x0f, 0x96, 0x1f, 0x0e, 0x71,
                0xf3, 0xa9, 0xb5, 0x24, 0xaf, 0x60, 0x12, 0x06,
                0x2f, 0xe0, 0x37, 0xa6
            ]);
            const pbkdf2Key = await subtle.importKey("raw", pbkdf2Password, "PBKDF2", false, ["deriveBits", "deriveKey"]);
            const pbkdf2Params = { name: "PBKDF2", hash: "SHA-1", salt: pbkdf2Salt, iterations: 1 };
            const pbkdf2Bits = await subtle.deriveBits(pbkdf2Params, pbkdf2Key, pbkdf2Expected.byteLength * 8);
            if (!sameBytes(pbkdf2Bits, pbkdf2Expected)) {
                failures.push("pbkdf2:deriveBits");
            }
            const pbkdf2Aes = await subtle.deriveKey(
                pbkdf2Params,
                pbkdf2Key,
                { name: "AES-CBC", length: 128 },
                true,
                ["encrypt"]
            );
            if (!sameBytes(await subtle.exportKey("raw", pbkdf2Aes), pbkdf2Expected.slice(0, 16))) {
                failures.push("pbkdf2:deriveKey:aes");
            }

            // Chromium WPT:
            // WebCryptoAPI/derive_bits_keys/cfrg_curves_bits_fixtures.js.
            const x25519Private = new Uint8Array([
                48, 46, 2, 1, 0, 48, 5, 6, 3, 43, 101, 110, 4, 34, 4, 32,
                200, 131, 142, 118, 208, 87, 223, 183, 216, 201, 90, 105,
                225, 56, 22, 10, 221, 99, 115, 253, 113, 164, 210, 118,
                187, 86, 227, 168, 27, 100, 255, 97
            ]);
            const x25519Public = new Uint8Array([
                48, 42, 48, 5, 6, 3, 43, 101, 110, 3, 33, 0, 28, 242,
                177, 230, 2, 46, 197, 55, 55, 30, 215, 245, 62, 84, 250,
                17, 84, 216, 62, 152, 235, 100, 234, 81, 250, 229, 179,
                48, 124, 254, 151, 6
            ]);
            const x25519Expected = new Uint8Array([
                39, 104, 64, 157, 250, 185, 158, 194, 59, 140, 137, 185,
                63, 245, 136, 2, 149, 247, 97, 118, 8, 143, 137, 228,
                61, 254, 190, 126, 161, 149, 0, 8
            ]);
            const x25519PrivateKey = await subtle.importKey("pkcs8", x25519Private, "X25519", false, ["deriveBits"]);
            const x25519PublicKey = await subtle.importKey("spki", x25519Public, "X25519", false, []);
            const x25519Bits = await subtle.deriveBits(
                { name: "X25519", public: x25519PublicKey },
                x25519PrivateKey,
                256
            );
            if (!sameBytes(x25519Bits, x25519Expected)) {
                failures.push("x25519:deriveBits");
            }

            postMessage(failures);
        })().catch((error) => {
            postMessage(["error:" + error.name + ":" + error.message]);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-kdf-x25519-derivation.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let payload = expect_post_json(msg);
    handle.terminate_and_join();
    assert_eq!(payload, "[]");
}

#[tokio::test]
async fn worker_crypto_subtle_asymmetric_and_wrap_operations_match_any_global_intent() {
    ensure_v8();
    // The upstream WebCrypto suites exercise RSA, EC, EdDSA, and wrap/unwrap
    // as `any.js` tests. Keep a compact worker-global smoke so worker runtime
    // wiring cannot regress to only the HMAC/KDF subset while window coverage
    // stays green.
    let mut handle = spawn_worker(
        r#"
        const sameBytes = (left, right) => {
            const a = new Uint8Array(left);
            const b = new Uint8Array(right);
            return a.length === b.length && a.every((value, index) => value === b[index]);
        };
        (async () => {
            const failures = [];
            const subtle = crypto.subtle;
            const data = new TextEncoder().encode("worker asymmetric webcrypto");
            const raw128 = new Uint8Array([
                0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6,
                0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c
            ]);

            const aesKw = await subtle.importKey(
                "raw",
                raw128,
                "AES-KW",
                false,
                ["wrapKey", "unwrapKey"]
            );
            const hmac = await subtle.importKey(
                "raw",
                new Uint8Array([
                    1, 2, 3, 4, 5, 6, 7, 8,
                    9, 10, 11, 12, 13, 14, 15, 16
                ]),
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
            );
            const wrapped = await subtle.wrapKey("raw", hmac, aesKw, "AES-KW");
            const unwrapped = await subtle.unwrapKey(
                "raw",
                wrapped,
                aesKw,
                "AES-KW",
                { name: "HMAC", hash: "SHA-256" },
                true,
                ["sign"]
            );
            if (!sameBytes(await subtle.exportKey("raw", hmac), await subtle.exportKey("raw", unwrapped))) {
                failures.push("aes-kw-wrap-unwrap");
            }

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
            const rsaCiphertext = await subtle.encrypt("RSA-OAEP", rsaOaep.publicKey, data);
            if (!sameBytes(await subtle.decrypt("RSA-OAEP", rsaOaep.privateKey, rsaCiphertext), data)) {
                failures.push("rsa-oaep");
            }

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
            if (!await subtle.verify({ name: "ECDSA", hash: "SHA-256" }, ecdsa.publicKey, ecdsaSignature, data)) {
                failures.push("ecdsa");
            }

            const ed25519 = await subtle.generateKey("Ed25519", true, ["sign", "verify"]);
            const ed25519Signature = await subtle.sign("Ed25519", ed25519.privateKey, data);
            if (!await subtle.verify("Ed25519", ed25519.publicKey, ed25519Signature, data)) {
                failures.push("ed25519");
            }

            postMessage(failures);
        })().catch((error) => {
            postMessage(["error:" + error.name + ":" + error.message]);
        });
        "#
        .into(),
        "https://worker-crypto.test/subtle-asymmetric-and-wrap.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let payload = expect_post_json(msg);
    handle.terminate_and_join();
    assert_eq!(payload, "[]");
}

#[tokio::test]
async fn worker_crypto_subtle_hidden_in_nonsecure_context_matches_wpt_historical() {
    ensure_v8();
    // Chromium WPT: WebCryptoAPI/historical.any.js plus webcrypto.idl.
    // Dedicated workers created from non-secure origins keep `crypto` and
    // getRandomValues(), but must not expose the secure-context-only
    // randomUUID/SubtleCrypto/CryptoKey surface.
    let mut handle = spawn_worker(
        r#"
        postMessage([
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
        ]);
        "#
        .into(),
        "http://example.test/non-secure-worker.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"["true","function","false","undefined","false","false","undefined","false","undefined","false","undefined"]"#
    );
}

#[tokio::test]
async fn worker_postmessage_arraybuffer_to_parent() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const bytes = new Uint8Array([7, 8, 9]);
        postMessage(bytes.buffer);
        "#
        .into(),
        "test://postmessage_arraybuffer".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Post(payload) => {
            let description = inspect_payload(
                &payload,
                r#"
                (() => [
                    __wire.constructor.name,
                    __wire.byteLength,
                    Array.from(new Uint8Array(__wire)).join(',')
                ].join('|'))()
                "#,
            );
            assert_eq!(description, "ArrayBuffer|3|7,8,9");
        }
        other => panic!("expected Post, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_postmessage_image_data_to_parent_preserves_interface_and_pixels() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const imageData = new ImageData(new Uint8ClampedArray([1, 2, 3, 4]), 1, 1);
        imageData.data[0] = 128;
        postMessage(imageData);
        "#
        .into(),
        "test://postmessage_imagedata".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Post(payload) => {
            let description = inspect_payload_with_image_data(
                &payload,
                r#"
                (() => [
                    '' + __wire,
                    __wire.width,
                    __wire.height,
                    __wire.colorSpace,
                    Array.from(__wire.data).join(',')
                ].join('|'))()
                "#,
            );
            assert_eq!(description, "[object ImageData]|1|1|srgb|128,2,3,4");
        }
        other => panic!("expected Post, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_broadcast_channel_delivers_structured_data() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const receiver = new BroadcastChannel("plain-worker");
        const sender = new BroadcastChannel("plain-worker");
        receiver.onmessage = event => {
            postMessage([
                event.data.marker,
                event.data.nested.text,
                event.data.list.join(','),
                event.origin,
                event.ports.length,
                receiver.name
            ].join('|'));
        };
        sender.postMessage({
            marker: 23,
            nested: { text: "worker" },
            list: [3, 4]
        });
        "#
        .into(),
        "https://worker.example/broadcast".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""23|worker|3,4|https://worker.example|0|plain-worker""#
    );
}

#[tokio::test]
async fn worker_broadcast_channel_postmessage_preserves_webassembly_module() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const receiver = new BroadcastChannel("wasm-module-worker");
        const sender = new BroadcastChannel("wasm-module-worker");
        const module = new WebAssembly.Module(
            new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
        );
        receiver.onmessage = event => {
            const instance = new WebAssembly.Instance(event.data.module, {});
            postMessage([
                event instanceof MessageEvent,
                event.data.module instanceof WebAssembly.Module,
                event.data.module === module,
                Object.keys(instance.exports).length,
                event.data.label,
                event.origin,
                event.ports.length,
                receiver.name
            ].join('|'));
        };
        sender.postMessage({ label: "wasm", module });
        "#
        .into(),
        "https://worker.example/broadcast-wasm-module".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""true|true|false|0|wasm|https://worker.example|0|wasm-module-worker""#
    );
}

#[tokio::test]
async fn worker_broadcast_channel_postmessage_after_self_close_is_ignored() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const receiver = new BroadcastChannel("closed-worker-postmessage");
        receiver.onmessage = () => postMessage("unexpected");
        const sender = new BroadcastChannel("closed-worker-postmessage");
        close();
        sender.postMessage("leak");
        postMessage("done");
        "#
        .into(),
        "https://worker.example/broadcast-close-after-create".into(),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""done""#);
    let result = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(
        result.is_none(),
        "closed worker BroadcastChannel should not deliver after self.close(): {result:?}"
    );
}

#[tokio::test]
async fn worker_broadcast_channel_created_after_self_close_is_detached() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        close();
        const receiver = new BroadcastChannel("closed-worker-create");
        receiver.onmessage = () => postMessage("unexpected");
        const sender = new BroadcastChannel("closed-worker-create");
        sender.postMessage("leak");
        postMessage("done:" + receiver.name + ":" + sender.name);
        "#
        .into(),
        "https://worker.example/broadcast-close-before-create".into(),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""done:closed-worker-create:closed-worker-create""#
    );
    let result = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(
        result.is_none(),
        "BroadcastChannel created after self.close() should stay detached: {result:?}"
    );
}

#[tokio::test]
async fn data_url_workers_do_not_share_broadcast_channel_by_null_origin() {
    ensure_v8();
    let registry = crate::broadcast_channel_runtime::new_broadcast_channel_registry();
    let top_level_site = Some("https://app.example".to_owned());

    let receiver_source = r#"
        const channel = new BroadcastChannel("opaque-worker");
        let sawMessage = false;
        channel.onmessage = event => {
            sawMessage = true;
            postMessage("unexpected:" + event.data + ":" + event.origin);
        };
        onmessage = event => {
            if (event.data === "report") {
                postMessage(sawMessage ? "receiver-received" : "receiver-silent");
            }
        };
        postMessage("receiver-ready");
    "#;
    let mut receiver =
        crate::worker::thread::spawn_worker_with_request_client_and_kind_network_policy_and_broadcast_channel_registry(
            receiver_source.into(),
            worker_data_url(receiver_source),
            worker_test_request_client(),
            WorkerScriptKind::Classic,
            crate::worker::handle::WorkerNetworkPolicy::default(),
            registry.clone(),
            top_level_site.clone(),
            None,
        );

    let sender_source = r#"
        const channel = new BroadcastChannel("opaque-worker");
        onmessage = event => {
            if (event.data === "go") {
                channel.postMessage("leak");
                postMessage("sender-sent");
            }
        };
        postMessage("sender-ready");
    "#;
    let mut sender =
        crate::worker::thread::spawn_worker_with_request_client_and_kind_network_policy_and_broadcast_channel_registry(
            sender_source.into(),
            worker_data_url(sender_source),
            worker_test_request_client(),
            WorkerScriptKind::Classic,
            crate::worker::handle::WorkerNetworkPolicy::default(),
            registry,
            top_level_site,
            None,
        );

    assert_eq!(recv_post_json(&mut receiver).await, r#""receiver-ready""#);
    assert_eq!(recv_post_json(&mut sender).await, r#""sender-ready""#);

    sender.post_message(serialize_test_string("go"));
    assert_eq!(recv_post_json(&mut sender).await, r#""sender-sent""#);

    receiver.post_message(serialize_test_string("report"));
    assert_eq!(recv_post_json(&mut receiver).await, r#""receiver-silent""#);

    sender.terminate();
    receiver.terminate();
}

#[tokio::test]
async fn nested_worker_broadcast_channel_inherits_parent_storage_key() {
    ensure_v8();
    let registry = crate::broadcast_channel_runtime::new_broadcast_channel_registry();
    let browser_context_runtime =
        crate::runtime::RendererBrowserContextRuntime::new_with_registries_for_test(
            crate::message_port_runtime::new_message_port_registry(),
            registry,
        );
    let creator_storage_key = moli_storage_key::MoliStorageKey::new(
        "null".to_owned(),
        "https://top.example".to_owned(),
        Some(moli_storage_key::OpaqueOriginNonce::new(19)),
        moli_storage_key::StoragePartitionRelation::Unknown,
    );

    let source = r#"
        const parentChannel = new BroadcastChannel("nested-worker-owner-key");
        parentChannel.onmessage = event => {
            postMessage("parent:" + event.data + ":" + event.origin);
            close();
        };
        const childSource = `
            const childChannel = new BroadcastChannel("nested-worker-owner-key");
            childChannel.onmessage = event => {
                childChannel.postMessage("child-saw:" + event.origin);
            };
            postMessage("ready");
        `;
        const childUrl = URL.createObjectURL(
            new Blob([childSource], { type: "text/javascript" })
        );
        const child = new Worker(childUrl);
        child.onmessage = event => {
            if (event.data === "ready") {
                parentChannel.postMessage("ping");
            }
        };
        child.onerror = event => {
            postMessage("child-error:" + event.message);
            close();
        };
    "#;
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(source.into(), "blob:null/parent-worker".into())
            .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
            .with_storage_key_top_level_site(Some("https://ignored.example".to_owned()))
            .with_creator_storage_key(creator_storage_key),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""parent:child-saw:null:null""#
    );
    handle.terminate();
}

#[tokio::test]
async fn worker_postmessage_wasm_module_to_parent_preserves_module() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
        postMessage(new WebAssembly.Module(bytes));
        "#
        .into(),
        "test://postmessage_wasm_module".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Post(payload) => {
            assert!(payload.metadata.contains_wasm_module);
            assert!(payload.metadata.origin_check_required);
            assert!(payload.metadata.locked_to_sender_agent_cluster);
            let description = inspect_payload(
                &payload,
                r#"
                (() => {
                    const instance = new WebAssembly.Instance(__wire, {});
                    return [
                        __wire instanceof WebAssembly.Module,
                        Object.keys(instance.exports).length
                    ].join('|');
                })()
                "#,
            );
            assert_eq!(description, "true|0");
        }
        other => panic!("expected Post, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_postmessage_wasm_module_from_parent_preserves_module() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onmessage = function(event) {
            const instance = new WebAssembly.Instance(event.data, {});
            postMessage([
                event.data instanceof WebAssembly.Module,
                Object.keys(instance.exports).length
            ].join('|'));
        };
        "#
        .into(),
        "test://postmessage_wasm_module_from_parent".into(),
    );

    let payload = serialize_test_post_message_value(
        "new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))",
    );
    assert!(payload.metadata.contains_wasm_module);
    assert!(payload.metadata.origin_check_required);
    assert!(payload.metadata.locked_to_sender_agent_cluster);
    handle.post_message(payload);

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""true|0""#);
}

#[tokio::test]
async fn worker_notification_data_rejects_webassembly_module_for_storage() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const createModule = () => new WebAssembly.Module(
            new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
        );
        const probe = (value) => {
            try {
                new Notification("title", { data: value });
                return "ok";
            } catch (error) {
                return error && error.name;
            }
        };

        let before = false;
        let after = false;
        const interleaved = probe([
            { get x() { before = true; return 1; } },
            createModule(),
            { get x() { after = true; return 2; } }
        ]);
        const plain = new Notification("plain", { data: { answer: 42 } });
        const missing = new Notification("missing");
        const explicitUndefined = new Notification(
            "explicit",
            { data: undefined }
        );
        const constructorActions = (() => {
            try {
                new Notification("actions", {
                    actions: [{ action: "reply", title: "Reply" }]
                });
                return "ok";
            } catch (error) {
                return error && error.name;
            }
        })();

        postMessage([
            typeof Notification,
            Notification.permission,
            Notification.maxActions,
            typeof Notification.requestPermission,
            plain.title,
            plain.actions.length,
            plain.data.answer,
            missing.data === null,
            explicitUndefined.data === null,
            constructorActions,
            probe(createModule()),
            interleaved,
            before,
            after
        ].join("|"));
        "#
        .into(),
        "https://example.test/worker_notification_storage_clone.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#""function|default|2|function|plain|0|42|true|true|TypeError|DataCloneError|DataCloneError|true|false""#
    );
}

#[tokio::test]
async fn worker_notification_permission_tracks_permission_overrides() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            (async () => {
              const requested = await Notification.requestPermission();
              postMessage([Notification.permission, requested].join("|"));
            })().catch(error => {
              postMessage("error:" + error.name);
            });
            "#
            .into(),
            "https://example.test/worker-notification-permission.js".into(),
        )
        .with_network_policy(WorkerNetworkPolicy {
            permission_overrides: vec![crate::protocol_types::PermissionOverrideRegistration {
                permission: serde_json::Value::String("notifications".to_owned()),
                setting: "granted".to_owned(),
                origin: None,
                embedded_origin: None,
            }],
            ..WorkerNetworkPolicy::default()
        }),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""granted|granted""#);
}

#[tokio::test]
async fn worker_postmessage_arraybuffer_transfer_to_parent_detaches_sender() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const buffer = new Uint8Array([7, 8, 9]).buffer;
        postMessage(buffer, [buffer]);
        postMessage(buffer.byteLength);
        "#
        .into(),
        "test://postmessage_arraybuffer_transfer".into(),
    );

    let first = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match first {
        WorkerToParentMessage::Post(payload) => {
            let description = inspect_payload(
                &payload,
                r#"
                (() => [
                    __wire.constructor.name,
                    __wire.byteLength,
                    Array.from(new Uint8Array(__wire)).join(',')
                ].join('|'))()
                "#,
            );
            assert_eq!(description, "ArrayBuffer|3|7,8,9");
        }
        other => panic!("expected Post, got {other:?}"),
    }

    let second = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(second), "0");
}

#[tokio::test]
async fn worker_postmessage_transfer_rejects_invalid_entry() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            postMessage("nope", [new Uint8Array([1])]);
            postMessage("unexpected");
        } catch (error) {
            postMessage(error.name);
        }
        "#
        .into(),
        "test://postmessage_transfer_invalid".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""DataCloneError""#);
}

#[tokio::test]
async fn worker_postmessage_rejects_wasm_memory_buffer_transfer_with_type_error() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            const buffer = new WebAssembly.Memory({ initial: 1 }).buffer;
            postMessage("nope", [buffer]);
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error && error.name,
                dataClone: error instanceof DOMException && error.name === "DataCloneError",
            });
        }
        "#
        .into(),
        "test://postmessage_wasm_memory_buffer_transfer".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","dataClone":false}"#
    );
}

// ─── Ping-pong ──────────────────────────────────────────────────────

#[tokio::test]
async fn worker_pingpong() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onmessage = function(e) {
            postMessage("pong:" + e.data);
        };
        "#
        .into(),
        "test://pingpong".into(),
    );

    handle.post_message(serialize_test_string("ping"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""pong:ping""#);
}

#[tokio::test]
async fn worker_onmessage_receives_messageevent_instance() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onmessage = function(event) {
            postMessage({
                isMessageEvent: event instanceof MessageEvent,
                typeString: Object.prototype.toString.call(event),
                type: event.type,
                data: event.data
            });
        };
        "#
        .into(),
        "test://onmessage_event_instance".into(),
    );

    handle.post_message(serialize_test_string("ping"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"isMessageEvent":true,"typeString":"[object MessageEvent]","type":"message","data":"ping"}"#
    );
}

#[tokio::test]
async fn worker_pingpong_object_payload() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onmessage = function(e) {
            postMessage({
                nestedValue: e.data && e.data.nested && e.data.nested.value,
                listLength: Array.isArray(e.data && e.data.list) ? e.data.list.length : -1
            });
        };
        "#
        .into(),
        "test://pingpong_object".into(),
    );

    handle.post_message(serialize_test_value(
        r#"({ nested: { value: "ok" }, list: [1, 2, 3] })"#,
    ));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"nestedValue":"ok","listLength":3}"#
    );
}

#[tokio::test]
async fn worker_arraybuffer_round_trip_supports_dataview() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onmessage = function(event) {
            const view = new DataView(event.data);
            postMessage(new Uint8Array([view.getUint8(0) + 1, view.getUint8(1) + 1]));
        };
        "#
        .into(),
        "test://arraybuffer_round_trip".into(),
    );

    handle.post_message(serialize_test_value("new Uint8Array([40, 41]).buffer"));

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Post(payload) => {
            let description = inspect_payload(
                &payload,
                r#"
                (() => [
                    __wire.constructor.name,
                    __wire.length,
                    Array.from(__wire).join(','),
                    String(__wire.buffer instanceof ArrayBuffer)
                ].join('|'))()
                "#,
            );
            assert_eq!(description, "Uint8Array|2|41,42|true");
        }
        other => panic!("expected Post, got {other:?}"),
    }
}

// ─── Multiple messages ──────────────────────────────────────────────

#[tokio::test]
async fn worker_multiple_messages() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let count = 0;
        onmessage = function(e) {
            count++;
            postMessage(count);
        };
        "#
        .into(),
        "test://multi_msg".into(),
    );

    for expected in 1..=3 {
        handle.post_message(serialize_test_string("go"));
        let msg = timeout(TIMEOUT, handle.recv())
            .await
            .expect("timed out")
            .expect("channel closed");
        assert_eq!(expect_post_json(msg), expected.to_string());
    }
}

// ─── Terminate ──────────────────────────────────────────────────────

#[tokio::test]
async fn worker_terminate() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onmessage = function(e) {
            postMessage("alive");
        };
        "#
        .into(),
        "test://terminate".into(),
    );

    handle.post_message(serialize_test_string("check"));
    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(matches!(msg, WorkerToParentMessage::Post(_)));

    handle.terminate();

    let result = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(
        result.is_none(),
        "expected channel to close after terminate"
    );
}

#[tokio::test]
async fn nested_worker_terminate_drops_pending_messages() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const child = new Worker('data:text/javascript,postMessage(%22queued%22);');
        setTimeout(() => {
            child.terminate();
            child.onmessage = () => postMessage("bad");
            setTimeout(() => postMessage("done"), 0);
        }, 20);
        "#
        .into(),
        "https://nested-worker-terminate.test/parent.js".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""done""#);

    handle.terminate();
}

// ─── self.close() ───────────────────────────────────────────────────

#[tokio::test]
async fn worker_self_close() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        postMessage("before_close");
        close();
        "#
        .into(),
        "test://self_close".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert!(matches!(msg, WorkerToParentMessage::Post(_)));

    let result = timeout(TIMEOUT, handle.recv()).await.expect("timed out");
    assert!(
        result.is_none(),
        "expected channel to close after self.close()"
    );
}

// ─── Error propagation ──────────────────────────────────────────────

#[tokio::test]
async fn worker_error_propagation() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        throw new Error("worker error");
        "#
        .into(),
        "test://error".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    match msg {
        WorkerToParentMessage::Error {
            message, filename, ..
        } => {
            assert!(message.contains("worker error"), "got: {message}");
            assert_eq!(filename, "test://error");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn worker_performance_now_uses_readonly_monotonic_time_origin() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const descriptor = Object.getOwnPropertyDescriptor(performance, "timeOrigin");
        const before = performance.timeOrigin;
        try { performance.timeOrigin = before + 1000000; } catch (_) {}
        const after = performance.timeOrigin;
        const first = performance.now();
        const second = performance.now();
        postMessage({
            readonly: descriptor && descriptor.writable === false,
            unchanged: after === before,
            numeric: typeof first === "number" && typeof second === "number",
            monotonic: second >= first
        });
        close();
        "#
        .into(),
        "test://worker_performance_now".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"readonly":true,"unchanged":true,"numeric":true,"monotonic":true}"#
    );
}

#[tokio::test]
async fn shared_worker_message_port_handler_can_reply_with_performance_now() {
    ensure_v8();
    let (port_wake_tx, mut port_wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let registry = crate::message_port_runtime::new_message_port_registry();

    let browser_context_runtime =
        crate::runtime::RendererBrowserContextRuntime::new_with_registries_for_test(
            registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
        );
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            addEventListener("connect", event => {
                const port = event.ports[0];
                port.onmessage = () => {
                    port.postMessage([
                        typeof workerStart,
                        typeof performance,
                        typeof performance.now,
                        performance.now()
                    ]);
                    port.close();
                };
            });
            "#
            .into(),
            "https://app.example/shared-worker-performance-now.js".into(),
        )
        .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        }),
    );

    let first = connect_shared_worker_test_port_and_read_now(
        &mut handle,
        &registry,
        &port_wake_tx,
        &mut port_wake_rx,
    )
    .await;
    let second = connect_shared_worker_test_port_and_read_now(
        &mut handle,
        &registry,
        &port_wake_tx,
        &mut port_wake_rx,
    )
    .await;
    assert!(
        second >= first,
        "reused SharedWorker connection should compute a fresh performance.now() value"
    );
}

#[tokio::test]
async fn shared_worker_navigator_exposes_canonical_user_agent_data() {
    ensure_v8();
    const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36";
    let (port_wake_tx, mut port_wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let registry = crate::message_port_runtime::new_message_port_registry();
    let browser_context_runtime =
        crate::runtime::RendererBrowserContextRuntime::new_with_registries_for_test(
            registry.clone(),
            crate::broadcast_channel_runtime::new_broadcast_channel_registry(),
        );
    let storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let mut config = FetchConfig::default();
    config.set_user_agent(USER_AGENT);
    config.push_default_request_header("Accept-Language", "de-DE,de;q=0.8");
    let loader = ResourceRequestClient::new(&config).expect("shared worker loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            addEventListener("connect", event => {
                const port = event.ports[0];
                const uaData = navigator.userAgentData;
                Promise.all([
                    uaData.getHighEntropyValues([]),
                    uaData.getHighEntropyValues(["architecture"])
                ]).then(([empty, selected]) => {
                    port.postMessage({
                        constructorType: typeof NavigatorUAData,
                        dataType: typeof uaData,
                        instance: uaData instanceof NavigatorUAData,
                        sameObject: uaData === navigator.userAgentData,
                        userAgent: navigator.userAgent,
                        languages: Array.from(navigator.languages),
                        brands: uaData.brands,
                        emptyKeys: Object.keys(empty),
                        selectedKeys: Object.keys(selected),
                        architecture: selected.architecture
                    });
                    port.close();
                });
                port.start();
            });
            "#
            .into(),
            "https://app.example/shared-worker-navigator.js".into(),
        )
        .with_request_client(loader)
        .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "shared".to_owned(),
            storage_key,
        }),
    );

    let (client_port_id, _) =
        connect_shared_worker_test_port(&mut handle, &registry, &port_wake_tx);
    assert_eq!(
        read_message_port_value(
            &registry,
            &mut port_wake_rx,
            client_port_id,
            "JSON.stringify(__wire)",
        )
        .await,
        r#"{"constructorType":"function","dataType":"object","instance":true,"sameObject":true,"userAgent":"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.1.2.3 Safari/537.36","languages":["de-DE","de"],"brands":[{"brand":"Chromium","version":"146"},{"brand":"Not-A.Brand","version":"24"},{"brand":"Google Chrome","version":"146"}],"emptyKeys":["brands","mobile","platform"],"selectedKeys":["architecture","brands","mobile","platform"],"architecture":"x86"}"#
    );
}

#[tokio::test]
async fn data_url_shared_workers_do_not_share_broadcast_channel_by_constructor_key() {
    ensure_v8();
    let (port_wake_tx, mut port_wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let message_port_registry = crate::message_port_runtime::new_message_port_registry();
    let broadcast_channel_registry =
        crate::broadcast_channel_runtime::new_broadcast_channel_registry();
    let browser_context_runtime =
        crate::runtime::RendererBrowserContextRuntime::new_with_registries_for_test(
            message_port_registry.clone(),
            broadcast_channel_registry,
        );
    let constructor_storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let worker_source = r#"
        addEventListener("connect", event => {
            const port = event.ports[0];
            const channel = new BroadcastChannel("data-url-shared-worker-opaque-bc");
            const seen = [];
            channel.onmessage = event => {
                seen.push(event.data + ":" + event.origin);
            };
            port.onmessage = event => {
                if (event.data === "send") {
                    channel.postMessage("leak");
                    port.postMessage("sent");
                } else if (event.data === "report") {
                    port.postMessage(seen.length === 0 ? "silent" : "received:" + seen.join("|"));
                }
            };
            port.start();
        });
    "#;

    let mut first = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(worker_source.into(), worker_data_url(worker_source))
            .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
            .with_storage_key_top_level_site(Some("https://app.example".to_owned()))
            .with_global_kind(super::super::WorkerGlobalKind::Shared {
                name: "first".to_owned(),
                storage_key: constructor_storage_key.clone(),
            }),
    );
    let mut second = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(worker_source.into(), worker_data_url(worker_source))
            .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
            .with_storage_key_top_level_site(Some("https://app.example".to_owned()))
            .with_global_kind(super::super::WorkerGlobalKind::Shared {
                name: "second".to_owned(),
                storage_key: constructor_storage_key,
            }),
    );

    let (first_client_port, first_worker_port) =
        connect_shared_worker_test_port(&mut first, &message_port_registry, &port_wake_tx);
    let (second_client_port, second_worker_port) =
        connect_shared_worker_test_port(&mut second, &message_port_registry, &port_wake_tx);

    message_port_registry.enqueue_message_to_message_port(
        first_client_port,
        serialize_test_post_message_value("'send'"),
    );
    message_port_registry.wake_message_port_if_pending(first_worker_port);
    assert_eq!(
        read_message_port_string(&message_port_registry, &mut port_wake_rx, first_client_port,)
            .await,
        "sent"
    );

    message_port_registry.enqueue_message_to_message_port(
        second_client_port,
        serialize_test_post_message_value("'report'"),
    );
    message_port_registry.wake_message_port_if_pending(second_worker_port);
    assert_eq!(
        read_message_port_string(
            &message_port_registry,
            &mut port_wake_rx,
            second_client_port,
        )
        .await,
        "silent"
    );

    first.terminate();
    second.terminate();
}

#[tokio::test]
async fn blob_url_shared_worker_broadcast_channel_uses_constructor_storage_key() {
    ensure_v8();
    let (port_wake_tx, mut port_wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let message_port_registry = crate::message_port_runtime::new_message_port_registry();
    let broadcast_channel_registry =
        crate::broadcast_channel_runtime::new_broadcast_channel_registry();
    let browser_context_runtime =
        crate::runtime::RendererBrowserContextRuntime::new_with_registries_for_test(
            message_port_registry.clone(),
            broadcast_channel_registry.clone(),
        );
    let constructor_storage_key = moli_storage_key::MoliStorageKey::new(
        "https://app.example".to_owned(),
        "https://app.example".to_owned(),
        None,
        moli_storage_key::StoragePartitionRelation::FirstParty,
    );
    let (broadcast_wake_tx, mut broadcast_wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let page_channel_id = broadcast_channel_registry.create_broadcast_channel(
        constructor_storage_key.clone(),
        "blob-shared-worker-constructor-bc".to_owned(),
        crate::broadcast_channel_runtime::BroadcastChannelOwner::Worker(broadcast_wake_tx),
    );
    let worker_source = r#"
        addEventListener("connect", event => {
            const port = event.ports[0];
            const channel = new BroadcastChannel("blob-shared-worker-constructor-bc");
            port.onmessage = event => {
                if (event.data === "send") {
                    channel.postMessage("from-worker");
                    port.postMessage("posted");
                }
            };
            port.start();
        });
    "#;

    let mut worker = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            worker_source.into(),
            "blob:https://app.example/blob-shared-worker.js".into(),
        )
        .with_worker_context_runtime(browser_context_runtime.worker_context_runtime())
        .with_storage_key_top_level_site(Some("https://app.example".to_owned()))
        .with_creator_storage_key(constructor_storage_key.clone())
        .with_global_kind(super::super::WorkerGlobalKind::Shared {
            name: "blob-shared".to_owned(),
            storage_key: constructor_storage_key,
        }),
    );

    let (client_port_id, worker_port_id) =
        connect_shared_worker_test_port(&mut worker, &message_port_registry, &port_wake_tx);
    message_port_registry.enqueue_message_to_message_port(
        client_port_id,
        serialize_test_post_message_value("'send'"),
    );
    message_port_registry.wake_message_port_if_pending(worker_port_id);

    let mut saw_ack = false;
    let mut broadcast_message = None;
    let message = loop {
        match broadcast_wake_rx.try_recv() {
            Ok(crate::worker::WorkerMessage::BroadcastChannelWake(channel_id)) => {
                assert_eq!(channel_id, page_channel_id);
                match broadcast_channel_registry.take_pending_broadcast_channel_event(channel_id) {
                    Some(crate::broadcast_channel_runtime::BroadcastChannelEvent::Message(
                        payload,
                    )) => {
                        broadcast_message = Some(inspect_payload(&payload, "String(__wire)"));
                    }
                    None => panic!("BroadcastChannel woke without a pending event"),
                }
            }
            Ok(other) => panic!("unexpected BroadcastChannel owner wake: {other:?}"),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("BroadcastChannel owner wake channel closed")
            }
        }
        match port_wake_rx.try_recv() {
            Ok(crate::worker::WorkerMessage::MessagePortWake(port_id))
                if port_id == client_port_id =>
            {
                let payload = message_port_registry
                    .take_pending_message_port_message(client_port_id)
                    .expect("client port should have an ack message");
                saw_ack |= inspect_payload(&payload, "String(__wire)") == "posted";
            }
            Ok(crate::worker::WorkerMessage::MessagePortWake(_)) => {}
            Ok(other) => {
                panic!("unexpected owner wake while waiting for BroadcastChannel: {other:?}")
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("MessagePort owner wake channel closed")
            }
        }
        if saw_ack && let Some(message) = broadcast_message.take() {
            break message;
        }
        timeout(TIMEOUT, async {
            tokio::select! {
                biased;
                wake = broadcast_wake_rx.recv() => {
                    let wake = wake.expect("BroadcastChannel owner wake channel should remain open");
                    let crate::worker::WorkerMessage::BroadcastChannelWake(channel_id) = wake else {
                        panic!("unexpected BroadcastChannel owner wake: {wake:?}");
                    };
                    assert_eq!(channel_id, page_channel_id);
                    match broadcast_channel_registry.take_pending_broadcast_channel_event(channel_id) {
                        Some(crate::broadcast_channel_runtime::BroadcastChannelEvent::Message(payload)) => {
                            broadcast_message = Some(inspect_payload(&payload, "String(__wire)"));
                        }
                        None => panic!("BroadcastChannel woke without a pending event"),
                    }
                }
                wake = port_wake_rx.recv() => {
                    let wake = wake.expect("MessagePort owner wake channel should remain open");
                    let crate::worker::WorkerMessage::MessagePortWake(port_id) = wake else {
                        panic!("unexpected MessagePort owner wake: {wake:?}");
                    };
                    if port_id == client_port_id {
                        let payload = message_port_registry
                            .take_pending_message_port_message(client_port_id)
                            .expect("client port should have an ack message");
                        saw_ack |= inspect_payload(&payload, "String(__wire)") == "posted";
                    }
                }
            }
        })
        .await
        .expect("timed out waiting for BroadcastChannel or MessagePort completion");
    };
    assert_eq!(message, "from-worker");
    assert!(
        saw_ack,
        "worker should acknowledge after posting to BroadcastChannel"
    );

    worker.terminate();
}

fn connect_shared_worker_test_port(
    handle: &mut WorkerHandle,
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    owner_wake: &tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerMessage>,
) -> (crate::types::MessagePortId, crate::types::MessagePortId) {
    let (client_port_id, worker_port_id) = registry.create_entangled_message_port_pair(
        crate::message_port_runtime::MessagePortOwner::Worker(owner_wake.clone()),
    );
    registry.detach_message_port_owner_for_transfer(worker_port_id);
    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(
            worker_port_id,
        ))
        .expect("connect shared worker");
    (client_port_id, worker_port_id)
}

async fn read_message_port_string(
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    owner_wake: &mut tokio::sync::mpsc::UnboundedReceiver<crate::worker::WorkerMessage>,
    target_port_id: crate::types::MessagePortId,
) -> String {
    read_message_port_value(registry, owner_wake, target_port_id, "String(__wire)").await
}

async fn read_message_port_value(
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    owner_wake: &mut tokio::sync::mpsc::UnboundedReceiver<crate::worker::WorkerMessage>,
    target_port_id: crate::types::MessagePortId,
    expression: &str,
) -> String {
    loop {
        let wake = timeout(TIMEOUT, owner_wake.recv())
            .await
            .expect("timed out waiting for MessagePort wake")
            .expect("MessagePort owner wake channel closed");
        match wake {
            crate::worker::WorkerMessage::MessagePortWake(port_id) if port_id == target_port_id => {
                if let Some(payload) = registry.take_pending_message_port_message(target_port_id) {
                    return inspect_payload(&payload, expression);
                }
                if registry.take_pending_message_port_close(target_port_id) {
                    continue;
                }
                panic!("MessagePort woke without message or close");
            }
            crate::worker::WorkerMessage::MessagePortWake(_) => continue,
            other => panic!("expected MessagePort wake, got {other:?}"),
        }
    }
}

async fn connect_shared_worker_test_port_and_read_now(
    handle: &mut WorkerHandle,
    registry: &crate::message_port_runtime::SharedMessagePortRegistry,
    owner_wake_tx: &tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerMessage>,
    owner_wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::worker::WorkerMessage>,
) -> f64 {
    let (client_port_id, worker_port_id) = registry.create_entangled_message_port_pair(
        crate::message_port_runtime::MessagePortOwner::Worker(owner_wake_tx.clone()),
    );
    registry.detach_message_port_owner_for_transfer(worker_port_id);
    handle
        .tx
        .send(crate::worker::WorkerMessage::SharedWorkerConnect(
            worker_port_id,
        ))
        .expect("connect shared worker");
    registry
        .enqueue_message_to_message_port(client_port_id, serialize_test_post_message_value("''"));
    registry.wake_message_port_if_pending(worker_port_id);

    loop {
        let wake = match timeout(TIMEOUT, owner_wake_rx.recv()).await {
            Ok(Some(wake)) => wake,
            Ok(None) => panic!("MessagePort owner wake channel closed before reply"),
            Err(error) => {
                let mut worker_events = Vec::new();
                while let Ok(message) = handle.try_recv() {
                    worker_events.push(match message {
                        WorkerToParentMessage::Post(payload) => stringify_payload(&payload),
                        other => format!("{other:?}"),
                    });
                }
                panic!(
                    "timed out waiting for SharedWorker MessagePort reply: {error:?}; worker_events={worker_events:?}"
                );
            }
        };
        match wake {
            crate::worker::WorkerMessage::MessagePortWake(port_id) if port_id == client_port_id => {
                if let Some(payload) = registry.take_pending_message_port_message(client_port_id) {
                    assert_eq!(inspect_payload(&payload, "__wire.length"), "4");
                    assert_eq!(inspect_payload(&payload, "__wire[0]"), "undefined");
                    assert_eq!(inspect_payload(&payload, "__wire[1]"), "object");
                    assert_eq!(inspect_payload(&payload, "__wire[2]"), "function");
                    assert_eq!(inspect_payload(&payload, "__wire[3] >= 0"), "true");
                    return inspect_payload(&payload, "__wire[3]")
                        .parse()
                        .expect("performance.now payload should be numeric");
                }
                if registry.take_pending_message_port_close(client_port_id) {
                    continue;
                }
                panic!("SharedWorker client port woke without message or close");
            }
            crate::worker::WorkerMessage::MessagePortWake(_) => continue,
            other => panic!("expected MessagePort wake, got {other:?}"),
        }
    }
}

// ─── setTimeout ─────────────────────────────────────────────────────

#[tokio::test]
async fn worker_settimeout() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        setTimeout(function() {
            postMessage("delayed");
        }, 50);
        "#
        .into(),
        "test://settimeout".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""delayed""#);
}

#[tokio::test]
async fn worker_settimeout_with_args() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        setTimeout(function(a, b) {
            postMessage(a + b);
        }, 10, 3, 4);
        "#
        .into(),
        "test://settimeout_args".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), "7");
}

#[tokio::test]
async fn worker_settimeout_accepts_callable_proxy_with_worker_global_receiver() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const callback = new Proxy(function() {}, {
            apply(_target, receiver, args) {
                postMessage(`${receiver === self}:${args[0] + args[1]}`);
                close();
            }
        });
        setTimeout(callback, 0, 3, 4);
        "#
        .into(),
        "test://settimeout_callable_proxy".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""true:7""#);
}

#[tokio::test]
async fn worker_settimeout_negative_delay_clamps_to_zero() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        setTimeout(function() {
            postMessage("timer");
        }, -1);
        postMessage("scheduled");
        "#
        .into(),
        "test://settimeout_negative_delay".into(),
    );

    let first = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(first), r#""scheduled""#);

    let second = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(second), r#""timer""#);
}

// ─── requestAnimationFrame ─────────────────────────────────────────

#[tokio::test]
async fn worker_request_animation_frame_accepts_callable_proxy_with_webidl_arguments() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const callback = new Proxy(function() {}, {
            apply(_target, receiver, args) {
                postMessage({
                    undefinedReceiver: receiver === undefined,
                    argumentCount: args.length,
                    finiteTimestamp: Number.isFinite(args[0])
                });
                close();
            }
        });
        requestAnimationFrame(callback);
        "#
        .into(),
        "test://request_animation_frame_callable_proxy".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"undefinedReceiver":true,"argumentCount":1,"finiteTimestamp":true}"#
    );
}

#[tokio::test]
async fn worker_request_animation_frame_rejects_non_callable_callback() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            requestAnimationFrame({});
            postMessage("unexpected");
        } catch (error) {
            postMessage(error.name);
            close();
        }
        "#
        .into(),
        "test://request_animation_frame_non_callable".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""TypeError""#);
}

#[tokio::test]
async fn worker_cancel_animation_frame_cancels_exact_callback() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const animationFrameId = requestAnimationFrame(() => {
            postMessage("unexpected");
        });
        cancelAnimationFrame(animationFrameId);
        setTimeout(() => {
            postMessage("cancelled");
            close();
        }, 30);
        "#
        .into(),
        "test://cancel_animation_frame".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""cancelled""#);
}

#[tokio::test]
async fn worker_request_animation_frame_exception_routes_through_worker_global_onerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        onerror = function(message, _filename, _lineno, _colno, error) {
            postMessage(`${String(message).includes("frame-boom")}:${error.message}`);
            close();
            return true;
        };
        requestAnimationFrame(() => {
            throw new Error("frame-boom");
        });
        "#
        .into(),
        "test://request_animation_frame_onerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""true:frame-boom""#);
}

// ─── clearTimeout ───────────────────────────────────────────────────

#[tokio::test]
async fn worker_cleartimeout() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let id = setTimeout(function() {
            postMessage("should not fire");
        }, 50);
        clearTimeout(id);
        setTimeout(function() {
            postMessage("ok");
        }, 100);
        "#
        .into(),
        "test://cleartimeout".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""ok""#);
}

#[tokio::test]
async fn worker_cleartimeout_after_timer_is_active() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        let id = setTimeout(function() {
            postMessage("should not fire");
        }, 100);
        setTimeout(function() {
            clearTimeout(id);
            setTimeout(function() {
                postMessage("ok");
            }, 40);
        }, 10);
        "#
        .into(),
        "test://cleartimeout_active".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""ok""#);
}

#[tokio::test]
async fn worker_postmessage_requires_argument() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            postMessage();
            postMessage("unexpected");
        } catch (error) {
            postMessage({
                name: error.name,
                required: String(error.message).includes("1 argument required")
            });
        }
        "#
        .into(),
        "test://postmessage_requires_argument".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(
        expect_post_json(msg),
        r#"{"name":"TypeError","required":true}"#
    );
}

#[tokio::test]
async fn worker_postmessage_function_throws_datacloneerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            postMessage(function nope() {});
            postMessage("unexpected");
        } catch (error) {
            postMessage(error.name);
        }
        "#
        .into(),
        "test://postmessage_datacloneerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""DataCloneError""#);
}

#[tokio::test]
async fn worker_postmessage_formdata_throws_datacloneerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            postMessage(new FormData());
            postMessage("unexpected");
        } catch (error) {
            postMessage(error.name);
        }
        "#
        .into(),
        "test://postmessage_formdata_datacloneerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""DataCloneError""#);
}

#[tokio::test]
async fn worker_postmessage_workernavigator_throws_datacloneerror() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        try {
            postMessage(navigator);
            postMessage("unexpected");
        } catch (error) {
            postMessage(error.name);
        }
        "#
        .into(),
        "test://postmessage_workernavigator_datacloneerror".into(),
    );

    let msg = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    assert_eq!(expect_post_json(msg), r#""DataCloneError""#);
}
