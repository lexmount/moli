use super::*;

use crate::context_bootstrap::WebCryptoTaskResult;
use crate::page_task_queue::{
    MainDocumentMetaRefreshNavigationTask, PageInternalLoadingTargetEffect,
    PageOwnedInternalLoadingTask,
};

#[tokio::test(flavor = "current_thread")]
async fn synchronous_child_script_registers_webcrypto_to_child_window_and_retires_on_detach() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "owner-bound-crypto-frame";
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__ownerBoundCryptoFrame = frame;
  void frame.contentWindow.Function;
  const script = frame.contentDocument.createElement("script");
  script.textContent = `
    globalThis.__ownerBoundCryptoTask = crypto.subtle
      .importKey(
        "raw",
        new Uint8Array([1, 2, 3]),
        "PBKDF2",
        false,
        ["deriveBits"]
      )
      .then(key => crypto.subtle.deriveBits(
        {
          name: "PBKDF2",
          salt: new Uint8Array([4, 5, 6]),
          iterations: 1000,
          hash: "SHA-256"
        },
        key,
        256
      ));
  `;
  frame.contentDocument.body.appendChild(script);
  return "scheduled";
})()
"#,
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("owner-bound-crypto-frame")
            .expect("child browsing context");
        let child_owner = page_vm
            .vm()
            .current_child_document_task_owner(child_handle)
            .expect("child document owner");
        let pending = page_vm.vm().pending_webcrypto_execution_contexts_for_test();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].0,
            crate::native_bridge::WindowExecutionContextOwner::Frame(child_owner.local_window_id),
            "WebCrypto registration in a child realm must bind its child LocalWindow"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                )
                .is_none(),
            "a dynamic inline classic script must not publish an asynchronous DocumentScriptReady task"
        );
        run_expected_child_realm_materialization_for_wait(
            &mut page_vm,
            "child WebCrypto script realm publication",
        )
        .await;

        page_vm.vm_mut().eval("document.open(); 'replaced'")?;
        assert!(
            !page_vm.vm().has_pending_webcrypto_tasks(),
            "child LocalWindow retirement must remove its pending WebCrypto resolver"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child WebCrypto owner should be tested through synchronous inline execution");
}

#[test]
fn webcrypto_task_rejects_a_real_page_vm_replacement_identity_collision() {
    run_page_vm_large_stack_async_test(
        "webcrypto-real-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><body>replacement</body>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let retired_producer = page_vm
                        .vm_mut()
                        .register_pending_webcrypto_task_producer_for_executor_test()?;
                    let retired_owner = retired_producer.owner();
                    assert_eq!(
                        retired_owner.root_document(),
                        page_vm.document_lifecycle.identity().document
                    );
                    retired_producer
                        .send(Ok(WebCryptoTaskResult::Bool(false)))
                        .expect("retired WebCrypto task should enter the stable Page source");

                    let replacement_url = format!("{base_url}/replacement.html");
                    page_vm
                        .vm_mut()
                        .eval(&format!("location.href = {replacement_url:?}; 'queued'"))?;
                    let mut pending_document_lifecycle_turn = None;
                    let navigation = page_vm
                        .follow_pending_location_navigation_one_turn_async(
                            &mut pending_document_lifecycle_turn,
                            PageVmInitStage::Load,
                        )
                        .await?;
                    assert!(matches!(
                        navigation,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                            | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                ..
                            }
                    ));

                    let current_producer = page_vm
                        .vm_mut()
                        .register_pending_webcrypto_task_producer_for_executor_test()?;
                    let current_owner = current_producer.owner();
                    assert_eq!(
                        retired_owner.task(),
                        current_owner.task(),
                        "fresh PageVm counters should naturally reuse the first WebCrypto task id and transport generation"
                    );
                    assert_eq!(
                        retired_owner.execution_context(),
                        current_owner.execution_context(),
                        "fresh PageVm counters should naturally reuse the top Window/realm identity"
                    );
                    assert_ne!(
                        retired_owner.root_document(),
                        current_owner.root_document(),
                        "the stable Page queue must namespace identical local owners by root Document"
                    );
                    assert_eq!(
                        current_owner.root_document(),
                        page_vm.document_lifecycle.identity().document
                    );
                    current_producer
                        .send(Ok(WebCryptoTaskResult::Bool(true)))
                        .expect("replacement WebCrypto task should enter the same stable Page source");

                    let current_document_owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("replacement main Document owner");
                    page_vm
                        .vm()
                        .schedule_page_internal_loading_task(
                            PageOwnedInternalLoadingTask::MetaRefreshNavigation(
                            MainDocumentMetaRefreshNavigationTask::new(
                                current_document_owner,
                                0,
                                Url::parse("https://example.test/refresh").unwrap(),
                            ),
                            ),
                            Instant::now(),
                        )
                        .expect("internal-loading task should enter the stable Page source");
                    park_current_document_websocket_for_test(
                        &mut page_vm,
                        moli_websocket::Event::TextMessage {
                            socket_id: 41,
                            data: "blocked".to_owned(),
                        },
                    )
                    .await;
                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebCryptoTask, &loader)
                            .await?,
                        "retired WebCrypto task should remain runnable beside independent internal-loading and backpressured WebSocket work"
                    );

                    assert_eq!(
                        page_vm
                            .vm()
                            .current_pending_webcrypto_task_execution_context(current_owner.task()),
                        Some(current_owner.execution_context()),
                        "discarding the old completion must not remove the colliding replacement Promise"
                    );

                    assert!(
                        page_vm
                            .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebCryptoTask, &loader)
                            .await?,
                        "replacement WebCrypto task should consume the following turn"
                    );

                    assert_eq!(
                        page_vm
                            .vm()
                            .current_pending_webcrypto_task_execution_context(current_owner.task()),
                        None,
                        "the current completion must settle exactly the replacement Promise"
                    );
                    let internal_loading = page_vm
                        .run_internal_loading_body_for_test()
                        .expect("the independent internal-loading task should remain queued");
                    assert_eq!(
                        internal_loading.action.target_effect,
                        PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                            effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated,
                        },
                        "the synthetic refresh must still enforce its own post-load prerequisite"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("WebCrypto replacement should run through the typed task executor");
            server
                .await
                .expect("WebCrypto PageVm replacement server should finish");
        },
    );
}

#[test]
fn webcrypto_task_survives_document_open_within_the_same_window_realm() {
    run_page_vm_large_stack_async_test("webcrypto-document-open-owner", || async move {
        let result: anyhow::Result<()> = async {
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse("https://webcrypto-document-open.test/").unwrap();
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

            let before_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("initial main Document owner");
            let producer = page_vm
                .vm_mut()
                .register_pending_webcrypto_task_producer_for_executor_test()?;
            let owner = producer.owner();

            page_vm.vm_mut().eval(
                r#"
            document.open();
            document.write("<!doctype html><title>replacement</title>");
            document.close();
            "replaced"
            "#,
            )?;

            let after_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("replacement main Document owner");
            assert_eq!(
                after_document_owner.local_window_id, before_document_owner.local_window_id,
                "document.open must preserve the Window execution context"
            );
            assert_ne!(
                after_document_owner.document_id, before_document_owner.document_id,
                "document.open must still rotate the Document owner"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .current_pending_webcrypto_task_execution_context(owner.task()),
                Some(owner.execution_context()),
                "Window-owned WebCrypto work must remain current across document.open"
            );

            producer
                .send(Ok(WebCryptoTaskResult::Bool(true)))
                .expect("preserved WebCrypto task should enter its production Page source");
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::WebCryptoTask,
                        &loader
                    )
                    .await?,
                "preserved WebCrypto task should consume one selected Page turn"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .current_pending_webcrypto_task_execution_context(owner.task()),
                None,
                "the selected task must settle the exact preserved Promise"
            );

            Ok(())
        }
        .await;
        result.expect("document.open WebCrypto task should run through the typed executor");
    });
}

#[tokio::test]
async fn crypto_subtle_digest_completes_through_owner_task() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoDigestDone = false;
                        globalThis.__webcryptoDigestResult = "pending";

                        (async () => {
                            const data = new TextEncoder().encode("owner-task-digest");
                            const promise = crypto.subtle.digest("SHA-256", data);
                            data.fill(255);
                            const bytes = new Uint8Array(await promise);
                            globalThis.__webcryptoDigestResult = Array.from(bytes)
                                .map((byte) => byte.toString(16).padStart(2, "0"))
                                .join("");
                        })().catch(error => {
                            globalThis.__webcryptoDigestResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoDigestDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoDigestDone === true)",
                    "digest WebCrypto owner task should settle",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__webcryptoDigestResult")?,
                    "a52386583e06ab1cd977656268f7c60ec186cbd22deaa6c43fc9edffe69b7aa2"
                );
                anyhow::Ok(())
            })
            .await
            .expect("digest WebCrypto owner task probe should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn crypto_subtle_asymmetric_imports_complete_through_owner_tasks() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoAsymmetricImportDone = false;
                        globalThis.__webcryptoAsymmetricImportResult = "pending";
                        const sameBytes = (left, right) => {
                            const a = new Uint8Array(left);
                            const b = new Uint8Array(right);
                            return a.length === b.length && a.every((value, index) => value === b[index]);
                        };
                        const data = new TextEncoder().encode("owner-task-import");

                        (async () => {
                            const subtle = crypto.subtle;
                            const results = [];

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
                            const x25519PrivateKey = await subtle.importKey(
                                "pkcs8",
                                x25519Private,
                                "X25519",
                                false,
                                ["deriveBits"]
                            );
                            const x25519PublicKey = await subtle.importKey(
                                "spki",
                                x25519Public,
                                "X25519",
                                true,
                                []
                            );
                            const x25519Bits = await subtle.deriveBits(
                                { name: "X25519", public: x25519PublicKey },
                                x25519PrivateKey,
                                256
                            );
                            results.push(new Uint8Array(x25519Bits).length === 32 ? "x25519" : "x25519:bad");

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
                            const rsaPublic = await subtle.importKey(
                                "spki",
                                await subtle.exportKey("spki", rsaPair.publicKey),
                                { name: "RSA-OAEP", hash: "SHA-256" },
                                true,
                                ["encrypt"]
                            );
                            const rsaPrivate = await subtle.importKey(
                                "pkcs8",
                                await subtle.exportKey("pkcs8", rsaPair.privateKey),
                                { name: "RSA-OAEP", hash: "SHA-256" },
                                true,
                                ["decrypt"]
                            );
                            const rsaCiphertext = await subtle.encrypt("RSA-OAEP", rsaPublic, data);
                            const rsaPlaintext = await subtle.decrypt("RSA-OAEP", rsaPrivate, rsaCiphertext);
                            results.push(sameBytes(rsaPlaintext, data) ? "rsa" : "rsa:bad");

                            const ecdsaPair = await subtle.generateKey(
                                { name: "ECDSA", namedCurve: "P-256" },
                                true,
                                ["sign", "verify"]
                            );
                            const ecdsaPublic = await subtle.importKey(
                                "spki",
                                await subtle.exportKey("spki", ecdsaPair.publicKey),
                                { name: "ECDSA", namedCurve: "P-256" },
                                true,
                                ["verify"]
                            );
                            const ecdsaPrivate = await subtle.importKey(
                                "pkcs8",
                                await subtle.exportKey("pkcs8", ecdsaPair.privateKey),
                                { name: "ECDSA", namedCurve: "P-256" },
                                true,
                                ["sign"]
                            );
                            const ecdsaSignature = await subtle.sign(
                                { name: "ECDSA", hash: "SHA-256" },
                                ecdsaPrivate,
                                data
                            );
                            results.push(await subtle.verify(
                                { name: "ECDSA", hash: "SHA-256" },
                                ecdsaPublic,
                                ecdsaSignature,
                                data
                            ) ? "ecdsa" : "ecdsa:bad");

                            const edPair = await subtle.generateKey("Ed25519", true, ["sign", "verify"]);
                            const edPublic = await subtle.importKey(
                                "spki",
                                await subtle.exportKey("spki", edPair.publicKey),
                                "Ed25519",
                                true,
                                ["verify"]
                            );
                            const edPrivate = await subtle.importKey(
                                "pkcs8",
                                await subtle.exportKey("pkcs8", edPair.privateKey),
                                "Ed25519",
                                true,
                                ["sign"]
                            );
                            const edSignature = await subtle.sign("Ed25519", edPrivate, data);
                            results.push(await subtle.verify("Ed25519", edPublic, edSignature, data) ? "eddsa" : "eddsa:bad");

                            globalThis.__webcryptoAsymmetricImportResult = results.join("|");
                        })().catch(error => {
                            globalThis.__webcryptoAsymmetricImportResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoAsymmetricImportDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoAsymmetricImportDone === true)",
                    "asymmetric WebCrypto import owner tasks should settle",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__webcryptoAsymmetricImportResult")?,
                    "x25519|rsa|ecdsa|eddsa"
                );
                anyhow::Ok(())
            })
            .await
            .expect("asymmetric WebCrypto import owner task probe should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn crypto_subtle_hmac_sign_verify_complete_through_owner_tasks() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoHmacSetupDone = false;
                        globalThis.__webcryptoHmacSetupResult = "pending";

                        (async () => {
                            globalThis.__webcryptoHmacKey = await crypto.subtle.importKey(
                                "raw",
                                new TextEncoder().encode("owner-task-hmac-key"),
                                { name: "HMAC", hash: "SHA-256" },
                                false,
                                ["sign", "verify"]
                            );
                            globalThis.__webcryptoHmacOriginal =
                                new TextEncoder().encode("owner-task-hmac-data");
                            globalThis.__webcryptoHmacSetupResult = "ready";
                        })().catch(error => {
                            globalThis.__webcryptoHmacSetupResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoHmacSetupDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoHmacSetupDone === true)",
                    "HMAC setup should settle",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__webcryptoHmacSetupResult")?,
                    "ready"
                );

                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoHmacSignDone = false;
                        globalThis.__webcryptoHmacSignResult = "pending";

                        (async () => {
                            const data = new Uint8Array(globalThis.__webcryptoHmacOriginal);
                            const promise = crypto.subtle.sign(
                                "HMAC",
                                globalThis.__webcryptoHmacKey,
                                data
                            );
                            data.fill(255);
                            const signature = await promise;
                            const bytes = new Uint8Array(signature);
                            globalThis.__webcryptoHmacSignature = signature;
                            globalThis.__webcryptoHmacSignResult =
                                bytes.length === 32 ? "signed" : `bad:${bytes.length}`;
                        })().catch(error => {
                            globalThis.__webcryptoHmacSignResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoHmacSignDone = true;
                        });
                    })()
                    "#,
                )?;
                assert!(
                    page_vm.vm().has_pending_webcrypto_tasks(),
                    "HMAC sign should register an owner WebCrypto task before settlement"
                );
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoHmacSignDone === true)",
                    "HMAC sign WebCrypto owner task should settle",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__webcryptoHmacSignResult")?,
                    "signed"
                );

                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoHmacVerifyDone = false;
                        globalThis.__webcryptoHmacVerifyResult = "pending";

                        (async () => {
                            const ok = await crypto.subtle.verify(
                                "HMAC",
                                globalThis.__webcryptoHmacKey,
                                globalThis.__webcryptoHmacSignature,
                                globalThis.__webcryptoHmacOriginal
                            );
                            const tampered = new Uint8Array(globalThis.__webcryptoHmacOriginal);
                            tampered[0] ^= 1;
                            const bad = await crypto.subtle.verify(
                                "HMAC",
                                globalThis.__webcryptoHmacKey,
                                globalThis.__webcryptoHmacSignature,
                                tampered
                            );
                            globalThis.__webcryptoHmacVerifyResult =
                                ok === true && bad === false ? "verified" : `${ok}:${bad}`;
                        })().catch(error => {
                            globalThis.__webcryptoHmacVerifyResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoHmacVerifyDone = true;
                        });
                    })()
                    "#,
                )?;
                assert!(
                    page_vm.vm().has_pending_webcrypto_tasks(),
                    "HMAC verify should register an owner WebCrypto task before settlement"
                );
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoHmacVerifyDone === true)",
                    "HMAC verify WebCrypto owner tasks should settle",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__webcryptoHmacVerifyResult")?,
                    "verified"
                );
                anyhow::Ok(())
            })
            .await
            .expect("HMAC WebCrypto owner task probe should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn crypto_subtle_export_key_completes_through_owner_task() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoExportDone = false;
                        globalThis.__webcryptoExportResult = "pending";

                        const hasOps = (jwk, expected) =>
                            Array.isArray(jwk.key_ops) &&
                            jwk.key_ops.join(",") === expected;
                        const hasString = (value) => typeof value === "string" && value.length > 0;

                        (async () => {
                            const subtle = crypto.subtle;
                            const results = [];

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
                            const rsaSpki = await subtle.exportKey("spki", rsaPair.publicKey);
                            const rsaPkcs8 = await subtle.exportKey("pkcs8", rsaPair.privateKey);
                            const rsaJwk = await subtle.exportKey("jwk", rsaPair.privateKey);
                            results.push(
                                rsaSpki.byteLength > 100 &&
                                rsaPkcs8.byteLength > 300 &&
                                rsaJwk.kty === "RSA" &&
                                rsaJwk.alg === "RSA-OAEP-256" &&
                                hasString(rsaJwk.n) &&
                                hasString(rsaJwk.d) &&
                                hasOps(rsaJwk, "decrypt")
                                    ? "rsa"
                                    : "rsa:bad"
                            );

                            const ecdsaPair = await subtle.generateKey(
                                { name: "ECDSA", namedCurve: "P-256" },
                                true,
                                ["sign", "verify"]
                            );
                            const ecdsaRaw = await subtle.exportKey("raw", ecdsaPair.publicKey);
                            const ecdsaJwk = await subtle.exportKey("jwk", ecdsaPair.privateKey);
                            results.push(
                                ecdsaRaw.byteLength === 65 &&
                                ecdsaJwk.kty === "EC" &&
                                ecdsaJwk.crv === "P-256" &&
                                hasString(ecdsaJwk.x) &&
                                hasString(ecdsaJwk.y) &&
                                hasString(ecdsaJwk.d) &&
                                hasOps(ecdsaJwk, "sign")
                                    ? "ec"
                                    : "ec:bad"
                            );

                            const edPair = await subtle.generateKey("Ed25519", true, ["sign", "verify"]);
                            const edSpki = await subtle.exportKey("spki", edPair.publicKey);
                            const edPkcs8 = await subtle.exportKey("pkcs8", edPair.privateKey);
                            const edJwk = await subtle.exportKey("jwk", edPair.privateKey);
                            results.push(
                                edSpki.byteLength > 30 &&
                                edPkcs8.byteLength > 30 &&
                                edJwk.kty === "OKP" &&
                                edJwk.crv === "Ed25519" &&
                                hasString(edJwk.x) &&
                                hasString(edJwk.d) &&
                                hasOps(edJwk, "sign")
                                    ? "ed25519"
                                    : "ed25519:bad"
                            );

                            const xPair = await subtle.generateKey("X25519", true, ["deriveBits"]);
                            const xRaw = await subtle.exportKey("raw-public", xPair.publicKey);
                            const xSpki = await subtle.exportKey("spki", xPair.publicKey);
                            const xPkcs8 = await subtle.exportKey("pkcs8", xPair.privateKey);
                            const xPublicJwk = await subtle.exportKey("jwk", xPair.publicKey);
                            const xPrivateJwk = await subtle.exportKey("jwk", xPair.privateKey);
                            results.push(
                                xRaw.byteLength === 32 &&
                                xSpki.byteLength > 40 &&
                                xPkcs8.byteLength > 40 &&
                                xPublicJwk.kty === "OKP" &&
                                xPublicJwk.crv === "X25519" &&
                                hasString(xPublicJwk.x) &&
                                hasOps(xPublicJwk, "") &&
                                xPrivateJwk.kty === "OKP" &&
                                xPrivateJwk.crv === "X25519" &&
                                hasString(xPrivateJwk.x) &&
                                hasString(xPrivateJwk.d) &&
                                hasOps(xPrivateJwk, "deriveBits")
                                    ? "x25519"
                                    : "x25519:bad"
                            );

                            globalThis.__webcryptoExportResult = results.join("|");
                        })().catch(error => {
                            globalThis.__webcryptoExportResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoExportDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoExportDone === true)",
                    "exportKey WebCrypto owner task should settle",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__webcryptoExportResult")?,
                    "rsa|ec|ed25519|x25519"
                );
                anyhow::Ok(())
            })
            .await
            .expect("exportKey WebCrypto owner task probe should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn crypto_subtle_get_public_key_completes_through_owner_task() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoGetPublicKeySetupDone = false;
                        globalThis.__webcryptoGetPublicKeySetupResult = "pending";

                        (async () => {
                            globalThis.__webcryptoGetPublicKeyPair =
                                await crypto.subtle.generateKey("X25519", true, ["deriveBits"]);
                            globalThis.__webcryptoGetPublicKeySetupResult = "ready";
                        })().catch(error => {
                            globalThis.__webcryptoGetPublicKeySetupResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoGetPublicKeySetupDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoGetPublicKeySetupDone === true)",
                    "getPublicKey setup key generation should settle",
                )
                .await?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__webcryptoGetPublicKeySetupResult")?,
                    "ready"
                );

                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoGetPublicKeyDone = false;
                        globalThis.__webcryptoGetPublicKeyResult = "pending";

                        const sameBytes = (left, right) => {
                            const a = new Uint8Array(left);
                            const b = new Uint8Array(right);
                            return a.length === b.length && a.every((value, index) => value === b[index]);
                        };

                        (async () => {
                            const subtle = crypto.subtle;
                            const pair = globalThis.__webcryptoGetPublicKeyPair;
                            const derivedPublic = await subtle.getPublicKey(pair.privateKey, []);
                            const derivedRaw = await subtle.exportKey("raw-public", derivedPublic);
                            const originalRaw = await subtle.exportKey("raw-public", pair.publicKey);
                            globalThis.__webcryptoGetPublicKeyResult =
                                derivedPublic.type === "public" &&
                                derivedPublic.extractable === true &&
                                derivedPublic.algorithm.name === "X25519" &&
                                derivedPublic.usages.length === 0 &&
                                sameBytes(derivedRaw, originalRaw)
                                    ? "x25519"
                                    : "x25519:bad";
                        })().catch(error => {
                            globalThis.__webcryptoGetPublicKeyResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoGetPublicKeyDone = true;
                        });
                    })()
                    "#,
                )?;
                assert!(
                    page_vm.vm().has_pending_webcrypto_tasks(),
                    "getPublicKey should register an owner WebCrypto task before settlement"
                );
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoGetPublicKeyDone === true)",
                    "getPublicKey WebCrypto owner task should settle",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__webcryptoGetPublicKeyResult")?,
                    "x25519"
                );
                anyhow::Ok(())
            })
            .await
            .expect("getPublicKey WebCrypto owner task probe should run on owner lane");
    })
    .await;
}

#[tokio::test]
async fn crypto_subtle_wrap_key_exports_target_material_through_owner_task() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        globalThis.__webcryptoWrapDone = false;
                        globalThis.__webcryptoWrapResult = "pending";

                        const sameBytes = (left, right) => {
                            const a = new Uint8Array(left);
                            const b = new Uint8Array(right);
                            return a.length === b.length && a.every((value, index) => value === b[index]);
                        };

                        (async () => {
                            const subtle = crypto.subtle;
                            const results = [];
                            const data = new TextEncoder().encode("owner-task-wrap");

                            const aesWrappingKey = await subtle.generateKey(
                                { name: "AES-GCM", length: 256 },
                                true,
                                ["wrapKey", "unwrapKey"]
                            );
                            const rsaPair = await subtle.generateKey(
                                {
                                    name: "RSA-OAEP",
                                    modulusLength: 1024,
                                    publicExponent: new Uint8Array([1, 0, 1]),
                                    hash: "SHA-256"
                                },
                                true,
                                ["encrypt", "decrypt", "wrapKey", "unwrapKey"]
                            );

                            const wrappedRsaJwk = await subtle.wrapKey(
                                "jwk",
                                rsaPair.privateKey,
                                aesWrappingKey,
                                { name: "AES-GCM", iv: new Uint8Array([9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]) }
                            );
                            const unwrappedRsa = await subtle.unwrapKey(
                                "jwk",
                                wrappedRsaJwk,
                                aesWrappingKey,
                                { name: "AES-GCM", iv: new Uint8Array([9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]) },
                                { name: "RSA-OAEP", hash: "SHA-256" },
                                true,
                                ["decrypt"]
                            );
                            const rsaCiphertext = await subtle.encrypt("RSA-OAEP", rsaPair.publicKey, data);
                            const rsaPlaintext = await subtle.decrypt("RSA-OAEP", unwrappedRsa, rsaCiphertext);
                            results.push(sameBytes(rsaPlaintext, data) ? "aesgcm-jwk" : "aesgcm-jwk:bad");

                            const aesGcmKey = await subtle.generateKey(
                                { name: "AES-GCM", length: 128 },
                                true,
                                ["encrypt", "decrypt"]
                            );
                            const wrappedAesRaw = await subtle.wrapKey(
                                "raw",
                                aesGcmKey,
                                rsaPair.publicKey,
                                { name: "RSA-OAEP", label: new Uint8Array([1, 2, 3]) }
                            );
                            const unwrappedAes = await subtle.unwrapKey(
                                "raw",
                                wrappedAesRaw,
                                rsaPair.privateKey,
                                { name: "RSA-OAEP", label: new Uint8Array([1, 2, 3]) },
                                "AES-GCM",
                                true,
                                ["encrypt", "decrypt"]
                            );
                            const iv = new Uint8Array(12);
                            const aesCiphertext = await subtle.encrypt({ name: "AES-GCM", iv }, unwrappedAes, data);
                            const aesPlaintext = await subtle.decrypt({ name: "AES-GCM", iv }, unwrappedAes, aesCiphertext);
                            results.push(sameBytes(aesPlaintext, data) ? "rsa-raw" : "rsa-raw:bad");

                            globalThis.__webcryptoWrapResult = results.join("|");
                        })().catch(error => {
                            globalThis.__webcryptoWrapResult =
                                `${error && error.name}:${error && error.message}`;
                        }).finally(() => {
                            globalThis.__webcryptoWrapDone = true;
                        });
                    })()
                    "#,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__webcryptoWrapDone === true)",
                    "wrapKey WebCrypto owner task should settle",
                )
                .await?;
                assert_eq!(
                    page_vm.vm_mut().eval("globalThis.__webcryptoWrapResult")?,
                    "aesgcm-jwk|rsa-raw"
                );
                anyhow::Ok(())
            })
            .await
            .expect("wrapKey WebCrypto owner task probe should run on owner lane");
    })
    .await;
}
