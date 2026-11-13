use super::source_phase::ModuleSourceServer;
use super::*;

#[tokio::test]
async fn worker_json_parse_errors_prevent_sibling_execution_and_remain_cached() {
    ensure_v8();
    for imports in [
        r#"import './side.mjs'; import data from './bad.json' with {type:'json'};"#,
        r#"import data from './bad.json' with {type:'json'}; import './side.mjs';"#,
    ] {
        let mut server = ModuleSourceServer::start().await;
        let mut handle = server.worker(
            r#"
        self.runs = 0;
        self.stackPreparations = 0;
        Error.prepareStackTrace = () => { ++self.stackPreparations; return 'author stack'; };
        JSON.parse = () => { throw new Error('author JSON.parse'); };
        const capture = promise => promise.then(() => 'unexpected', error => error);
        const errors = await Promise.all([
            capture(import('./graph.mjs')), capture(import('./graph.mjs'))
        ]);
        const direct = await capture(import('./bad.json', {with:{type:'json'}}));
        const graphAgain = await capture(import('./graph.mjs'));
        postMessage([self.runs, self.stackPreparations, errors[0] instanceof SyntaxError,
            errors[0] === errors[1], errors[0] === direct, errors[0] === graphAgain]);
    "#
            .into(),
            WorkerScriptKind::Module,
        );
        server
            .respond(
                "/worker/graph.mjs",
                "200 OK",
                "text/javascript",
                &format!("{imports}\npostMessage('unexpected evaluation');"),
            )
            .await;
        let (sibling, invalid_json) = server
            .request_pair("/worker/side.mjs", "/worker/bad.json")
            .await;
        ModuleSourceServer::write(sibling, "200 OK", "text/javascript", "++self.runs;").await;
        ModuleSourceServer::write(invalid_json, "200 OK", "application/json", "{\n\"key\":\n}")
            .await;
        assert_eq!(
            recv_post_json(&mut handle).await,
            "[0,0,true,true,true,true]",
            "{imports}"
        );
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}

#[tokio::test]
async fn worker_nested_json_errors_follow_declaration_order_and_reuse_native_exceptions() {
    ensure_v8();
    for first_branch in ["a", "b"] {
        let mut server = ModuleSourceServer::start().await;
        let mut handle = server.worker(r#"
            (async () => {
                self.runs = 0;
                onmessage = async event => {
                    await import(`./bad-${event.data}.json`, { with: { type: 'json' } }).catch(error => error);
                    postMessage(['stored', event.data]);
                };
                const chosen = await import('./root.mjs').catch(error => error);
                const fromA = await import('./bad-a.json', { with: { type: 'json' } }).catch(error => error);
                const fromB = await import('./bad-b.json', { with: { type: 'json' } }).catch(error => error);
                postMessage([chosen === fromA, chosen === fromB, self.runs, chosen.name]);
            })().catch(error => postMessage({ error: error.name }));
        "#.into(), WorkerScriptKind::Module);
        server
            .respond(
                "/worker/root.mjs",
                "200 OK",
                "text/javascript",
                "import './a.mjs'; import './b.mjs'; ++self.runs;",
            )
            .await;
        let (a, b) = server.request_pair("/worker/a.mjs", "/worker/b.mjs").await;
        let (first, second, second_branch) = if first_branch == "a" {
            (a, b, "b")
        } else {
            (b, a, "a")
        };
        ModuleSourceServer::write(
            first,
            "200 OK",
            "text/javascript",
            &format!(
                "import value from './bad-{first_branch}.json' with {{type:'json'}}; ++self.runs;"
            ),
        )
        .await;
        server
            .respond(
                &format!("/worker/bad-{first_branch}.json"),
                "200 OK",
                "application/json",
                "{invalid}",
            )
            .await;
        // A separate import settles only after the shared creation result has
        // been stored. The other parent remains withheld until that barrier.
        handle.post_message(serialize_test_string(first_branch));
        assert_eq!(
            recv_post_json(&mut handle).await,
            format!(r#"["stored","{first_branch}"]"#)
        );
        ModuleSourceServer::write(
            second,
            "200 OK",
            "text/javascript",
            &format!(
                "import value from './bad-{second_branch}.json' with {{type:'json'}}; ++self.runs;"
            ),
        )
        .await;
        server
            .respond(
                &format!("/worker/bad-{second_branch}.json"),
                "200 OK",
                "application/json",
                "{invalid}",
            )
            .await;
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"[true,false,0,"SyntaxError"]"#,
            "{first_branch} completed first"
        );
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}
