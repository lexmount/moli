use super::source_phase::ModuleSourceServer;
use super::*;

#[tokio::test]
async fn worker_text_modules_fetch_once_across_static_and_dynamic_imports() {
    ensure_v8();
    for kind in [WorkerScriptKind::Classic, WorkerScriptKind::Module] {
        let mut server = ModuleSourceServer::start().await;
        let prefix = if kind == WorkerScriptKind::Module {
            "import * as staticText from './payload.wasm' with {type:'text'};"
        } else {
            "const staticText = null;"
        };
        let mut handle = server.worker(
            format!(
                r#"
            {prefix}
            (async () => {{
                const [first, second] = await Promise.all([
                    import('./payload.wasm', {{with:{{type:'text'}}}}),
                    import('./payload.wasm', {{with:{{type:'text'}}}})
                ]);
                postMessage([first.default, first === second, !staticText || staticText === first]);
            }})().catch(e => postMessage({{error:e.name}}));
        "#
            ),
            kind,
        );
        server
            .respond_bytes(
                "/worker/payload.wasm",
                "200 OK",
                "application/wasm; charset=utf-16",
                b"\xef\xbb\xbfhello\x00\xff\r\n",
            )
            .await;
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"["hello\u0000�\r\n",true,true]"#
        );
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}
