use super::*;
use base64::Engine;

#[tokio::test]
async fn child_parser_modules_capture_import_map_integrity_at_preparation() {
    run_page_vm_async_test(async move {
        let source = "globalThis.moduleExecutions = (globalThis.moduleExecutions || 0) + 1;";
        let matching = format!(
            "sha384-{}",
            base64::engine::general_purpose::STANDARD
                .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(source.as_bytes()))
        );
        let mismatching = format!(
            "sha384-{}",
            base64::engine::general_purpose::STANDARD
                .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(b"different source"))
        );
        let import_map = json!({"integrity": {
            "/parser-bad.mjs": mismatching,
            "/implicit-good.mjs": matching,
            "/implicit-bad.mjs": mismatching,
            "/explicit-empty.mjs": mismatching,
            "/explicit-space.mjs": mismatching,
            "/explicit-good.mjs": mismatching,
            "/explicit-bad.mjs": matching,
        }});
        // Main currently supports parser-inserted child module roots. Reuse
        // the source hash/attribute matrix through that supported preparation path.
        let child = r#"<!doctype html>
<script type="importmap">IMPORT_MAP</script>
<script>
globalThis.parserResult = 'pending';
globalThis.moduleResults = [];
addEventListener('load', () => {
  parent.postMessage({parser: parserResult, results: moduleResults,
    executions: globalThis.moduleExecutions || 0}, '*');
});
</script>
<script type="module" src="/parser-bad.mjs" onload="parserResult='load'" onerror="parserResult='error'"></script>
<script type="module" src="/implicit-good.mjs" onload="moduleResults.push('/implicit-good.mjs:load')" onerror="moduleResults.push('/implicit-good.mjs:error')"></script>
<script type="module" src="/implicit-bad.mjs" onload="moduleResults.push('/implicit-bad.mjs:load')" onerror="moduleResults.push('/implicit-bad.mjs:error')"></script>
<script type="module" src="/explicit-empty.mjs" integrity="" onload="moduleResults.push('/explicit-empty.mjs:load')" onerror="moduleResults.push('/explicit-empty.mjs:error')"></script>
<script type="module" src="/explicit-space.mjs" integrity="   " onload="moduleResults.push('/explicit-space.mjs:load')" onerror="moduleResults.push('/explicit-space.mjs:error')"></script>
<script type="module" src="/explicit-good.mjs" integrity="GOOD_INTEGRITY" onload="moduleResults.push('/explicit-good.mjs:load')" onerror="moduleResults.push('/explicit-good.mjs:error')"></script>
<script type="module" src="/explicit-bad.mjs" integrity="BAD_INTEGRITY" onload="moduleResults.push('/explicit-bad.mjs:load')" onerror="moduleResults.push('/explicit-bad.mjs:error')"></script>
"#
            .replace("IMPORT_MAP", &import_map.to_string())
            .replace("GOOD_INTEGRITY", &matching)
            .replace("BAD_INTEGRITY", &mismatching);
        let mut responses = vec![("/child.html", "HTTP/1.1 200 OK", child, Duration::ZERO)];
        for path in [
            "/parser-bad.mjs", "/implicit-good.mjs", "/implicit-bad.mjs",
            "/explicit-empty.mjs", "/explicit-space.mjs", "/explicit-good.mjs",
            "/explicit-bad.mjs",
        ] {
            responses.push((path, "HTTP/1.1 200 OK", source.to_owned(), Duration::ZERO));
        }
        let (base, server) = spawn_path_response_http_server(responses).await;
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent"))?);
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            let parent_map = serde_json::to_string(&json!({"integrity": {
                "/parser-bad.mjs": matching,
                "/implicit-good.mjs": mismatching,
                "/implicit-bad.mjs": matching,
            }}).to_string())?;
            page_vm.vm_mut().eval(&format!(r#"
const map = document.createElement('script'); map.type = 'importmap';
map.textContent = {parent_map}; document.head.appendChild(map);
addEventListener('message', event => {{ globalThis.childModuleIntegrityResult = event.data; }});
const frame = document.createElement('iframe'); frame.src = '/child.html';
document.body.appendChild(frame);
"#))?;
            drive_websocket_until_done(
                &mut page_vm,
                "String(globalThis.childModuleIntegrityResult !== undefined)",
                "child module integrity preparation",
            ).await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(childModuleIntegrityResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({
                "parser": "error",
                "results": [
                    "/implicit-good.mjs:load", "/implicit-bad.mjs:error",
                    "/explicit-empty.mjs:load", "/explicit-space.mjs:load",
                    "/explicit-good.mjs:load", "/explicit-bad.mjs:error",
                ],
                "executions": 4,
            }));
            server.await.expect("each module request should reach the fixture");
            anyhow::Ok(())
        }).await?;
        anyhow::Ok(())
    }).await.expect("child module integrity must use its preparation Document and attribute presence");
}

#[tokio::test]
async fn child_parser_modules_use_their_own_import_map_integrity() {
    run_page_vm_async_test(async move {
        let source = "globalThis.moduleExecutions = (globalThis.moduleExecutions || 0) + 1;";
        let matching = format!(
            "sha384-{}",
            base64::engine::general_purpose::STANDARD
                .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(source.as_bytes()))
        );
        let mismatching = format!(
            "sha384-{}",
            base64::engine::general_purpose::STANDARD
                .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(b"different source"))
        );
        let import_map = json!({"integrity": {
            "/parser-bad.mjs": mismatching,
            "/implicit-good.mjs": matching,
            "/implicit-bad.mjs": mismatching,
            "/explicit-empty.mjs": mismatching,
            "/explicit-space.mjs": mismatching,
            "/explicit-good.mjs": mismatching,
            "/explicit-bad.mjs": matching,
        }});
        let mut child = format!(
            r#"<!doctype html><script type="importmap">{import_map}</script><script>globalThis.results = {{}};</script>"#
        );
        for (path, explicit_integrity) in [
            ("/parser-bad.mjs", None),
            ("/implicit-good.mjs", None),
            ("/implicit-bad.mjs", None),
            ("/explicit-empty.mjs", Some("")),
            ("/explicit-space.mjs", Some("   ")),
            ("/explicit-good.mjs", Some(matching.as_str())),
            ("/explicit-bad.mjs", Some(mismatching.as_str())),
        ] {
            let integrity = explicit_integrity
                .map(|value| format!(r#" integrity="{value}""#))
                .unwrap_or_default();
            child.push_str(&format!(
                r#"<script type="module" src="{path}"{integrity} onload="results['{path}']='load'" onerror="results['{path}']='error'"></script>"#
            ));
        }
        child.push_str(
            r#"<script>addEventListener('load', () => parent.postMessage({results, executions:globalThis.moduleExecutions || 0}, '*'));</script>"#,
        );
        let mut responses = vec![("/child.html", "HTTP/1.1 200 OK", child, Duration::ZERO)];
        for path in [
            "/parser-bad.mjs", "/implicit-good.mjs", "/implicit-bad.mjs",
            "/explicit-empty.mjs", "/explicit-space.mjs", "/explicit-good.mjs",
            "/explicit-bad.mjs",
        ] {
            responses.push((path, "HTTP/1.1 200 OK", source.to_owned(), Duration::ZERO));
        }
        let (base, server) = spawn_path_response_http_server(responses).await;
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent"))?);
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            let parent_map = serde_json::to_string(&json!({"integrity": {
                "/parser-bad.mjs": matching,
                "/implicit-good.mjs": mismatching,
                "/implicit-bad.mjs": matching,
            }}).to_string())?;
            page_vm.vm_mut().eval(&format!(r#"
const map = document.createElement('script'); map.type = 'importmap';
map.textContent = {parent_map}; document.head.appendChild(map);
addEventListener('message', event => {{ globalThis.childModuleIntegrityResult = event.data; }});
const frame = document.createElement('iframe'); frame.src = '/child.html';
document.body.appendChild(frame);
"#))?;
            drive_websocket_until_done(
                &mut page_vm,
                "String(globalThis.childModuleIntegrityResult !== undefined)",
                "child module integrity preparation",
            ).await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(childModuleIntegrityResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({
                "results": {
                    "/parser-bad.mjs": "error",
                    "/implicit-good.mjs": "load",
                    "/implicit-bad.mjs": "error",
                    "/explicit-empty.mjs": "load",
                    "/explicit-space.mjs": "load",
                    "/explicit-good.mjs": "load",
                    "/explicit-bad.mjs": "error",
                },
                "executions": 4,
            }));
            server.await.expect("each module request should reach the fixture");
            anyhow::Ok(())
        }).await?;
        anyhow::Ok(())
    }).await.expect("child module integrity must use its preparation Document and attribute presence");
}

#[tokio::test]
async fn child_external_modules_capture_import_map_integrity_at_preparation() {
    run_page_vm_async_test(async move {
        let source = "globalThis.moduleExecutions = (globalThis.moduleExecutions || 0) + 1;";
        let matching = format!(
            "sha384-{}",
            base64::engine::general_purpose::STANDARD
                .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(source.as_bytes()))
        );
        let mismatching = format!(
            "sha384-{}",
            base64::engine::general_purpose::STANDARD
                .encode(moli_crypto::DigestAlgorithm::Sha384.digest_bytes(b"different source"))
        );
        let import_map = json!({"integrity": {
            "/parser-bad.mjs": mismatching,
            "/implicit-good.mjs": matching,
            "/implicit-bad.mjs": mismatching,
            "/explicit-empty.mjs": mismatching,
            "/explicit-space.mjs": mismatching,
            "/explicit-good.mjs": mismatching,
            "/explicit-bad.mjs": matching,
        }});
        let child = r#"<!doctype html>
<script type="importmap">IMPORT_MAP</script>
<script>globalThis.parserResult = 'pending';</script>
<script type="module" src="/parser-bad.mjs" onload="parserResult='load'" onerror="parserResult='error'"></script>
<script>
const good = GOOD_INTEGRITY;
const bad = BAD_INTEGRITY;
function loadModule(path, integrity) {
  return new Promise(resolve => {
    const script = document.createElement('script');
    script.type = 'module';
    script.src = path;
    if (integrity !== null) script.integrity = integrity;
    script.onload = () => resolve(path + ':load');
    script.onerror = () => resolve(path + ':error');
    document.body.appendChild(script);
  });
}
addEventListener('load', async () => {
  const results = [];
  for (const [path, integrity] of [
    ['/implicit-good.mjs', null], ['/implicit-bad.mjs', null],
    ['/explicit-empty.mjs', ''], ['/explicit-space.mjs', '   '],
    ['/explicit-good.mjs', good], ['/explicit-bad.mjs', bad]
  ]) results.push(await loadModule(path, integrity));
  const prepared = loadModule('/late-map.mjs', null);
  const lateMap = document.createElement('script');
  lateMap.type = 'importmap';
  lateMap.textContent = JSON.stringify({integrity: {'/late-map.mjs': bad}});
  document.head.appendChild(lateMap);
  results.push(await prepared);
  parent.postMessage({parser: parserResult, results, executions: moduleExecutions}, '*');
});
</script>"#
            .replace("IMPORT_MAP", &import_map.to_string())
            .replace("GOOD_INTEGRITY", &serde_json::to_string(&matching)?)
            .replace("BAD_INTEGRITY", &serde_json::to_string(&mismatching)?);
        let mut responses = vec![("/child.html", "HTTP/1.1 200 OK", child, Duration::ZERO)];
        for path in [
            "/parser-bad.mjs", "/implicit-good.mjs", "/implicit-bad.mjs",
            "/explicit-empty.mjs", "/explicit-space.mjs", "/explicit-good.mjs",
            "/explicit-bad.mjs", "/late-map.mjs",
        ] {
            responses.push((path, "HTTP/1.1 200 OK", source.to_owned(), Duration::ZERO));
        }
        let (base, server) = spawn_path_response_http_server(responses).await;
        let mut page_vm = test_page_vm_with_document_url(Url::parse(&format!("{base}/parent"))?);
        let executor = page_vm.local_executor.clone();
        executor.run(async move {
            let parent_map = serde_json::to_string(&json!({"integrity": {
                "/parser-bad.mjs": matching,
                "/implicit-good.mjs": mismatching,
                "/implicit-bad.mjs": matching,
            }}).to_string())?;
            page_vm.vm_mut().eval(&format!(r#"
const map = document.createElement('script'); map.type = 'importmap';
map.textContent = {parent_map}; document.head.appendChild(map);
addEventListener('message', event => {{ globalThis.childModuleIntegrityResult = event.data; }});
const frame = document.createElement('iframe'); frame.src = '/child.html';
document.body.appendChild(frame);
"#))?;
            drive_websocket_until_done(
                &mut page_vm,
                "String(globalThis.childModuleIntegrityResult !== undefined)",
                "child module integrity preparation",
            ).await?;
            let result = page_vm.vm_mut().eval("JSON.stringify(childModuleIntegrityResult)")?;
            assert_eq!(serde_json::from_str::<serde_json::Value>(&result)?, json!({
                "parser": "error",
                "results": [
                    "/implicit-good.mjs:load", "/implicit-bad.mjs:error",
                    "/explicit-empty.mjs:load", "/explicit-space.mjs:load",
                    "/explicit-good.mjs:load", "/explicit-bad.mjs:error",
                    "/late-map.mjs:load",
                ],
                "executions": 5,
            }));
            server.await.expect("each module request should reach the fixture");
            anyhow::Ok(())
        }).await?;
        anyhow::Ok(())
    }).await.expect("child module integrity must use its preparation Document and attribute presence");
}
