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
async fn worker_json_parse_failure_preserves_pending_top_level_await() {
    json_parse_failure_keeps_pending_sibling(true, false).await;
}

#[tokio::test]
async fn worker_json_parse_failure_reuses_pending_and_completed_sibling() {
    json_parse_failure_keeps_pending_sibling(false, false).await;
}

#[tokio::test]
async fn worker_json_parse_failure_caches_sibling_after_graph_is_gone() {
    for top_level_await in [true, false] {
        json_parse_failure_keeps_pending_sibling(top_level_await, true).await;
    }
}

async fn json_parse_failure_keeps_pending_sibling(
    top_level_await: bool,
    sibling_response_first: bool,
) {
    ensure_v8();
    for imports in [
        r#"import './side.mjs'; import data from './bad.json' with {type:'json'};"#,
        r#"import data from './bad.json' with {type:'json'}; import './side.mjs';"#,
    ] {
        let mut server = ModuleSourceServer::start().await;
        let source = r#"
            self.runs = 0;
            let graphError;
            try { await import('./graph.mjs'); }
            catch (error) { graphError = error; }
            postMessage(['caught', graphError.name, self.runs]);
            await fetch('./hold');
            const pendingSide = import('./side.mjs');
            postMessage('joining sibling');
            try {
                const side = await pendingSide;
                const again = await import('./side.mjs');
                const graphAgain = await import('./graph.mjs').catch(error => error);
                postMessage([true, graphError.name, side.value, self.runs,
                    side === again, graphError === graphAgain]);
            } catch (error) { postMessage({laterError: error.name}); }
        "#;
        let source = if top_level_await {
            source.to_owned()
        } else {
            format!(
                "(async () => {{ {source} }})().catch(error => postMessage({{error: error.name}}));"
            )
        };
        let mut handle = server.worker(source, WorkerScriptKind::Module);
        server
            .respond("/worker/graph.mjs", "200 OK", "text/javascript", imports)
            .await;
        let (sibling, invalid_json) = server
            .request_pair("/worker/side.mjs", "/worker/bad.json")
            .await;
        ModuleSourceServer::write(invalid_json, "200 OK", "application/json", "{invalid}").await;
        assert_eq!(
            recv_post_json(&mut handle).await,
            r#"["caught","SyntaxError",0]"#
        );
        let start_independent_import = async {
            server
                .respond("/worker/hold", "200 OK", "text/plain", "release")
                .await;
            assert_eq!(recv_post_json(&mut handle).await, r#""joining sibling""#);
        };
        let sibling_response = ModuleSourceServer::write(
            sibling,
            "200 OK",
            "text/javascript",
            "++self.runs; export const value = 42;",
        );
        if sibling_response_first {
            // No live import needs this response yet; retain its module record.
            sibling_response.await;
            start_independent_import.await;
        } else {
            // The independent import must reuse the original in-flight fetch.
            start_independent_import.await;
            sibling_response.await;
        }
        let observed = tokio::select! {
            observed = recv_post_json(&mut handle) => observed,
            duplicate = server.request("/worker/side.mjs") => {
                ModuleSourceServer::write(duplicate, "503 Service Unavailable", "text/plain", "duplicate fetch").await;
                recv_post_json(&mut handle).await
            }
        };
        assert_eq!(
            observed, r#"[true,"SyntaxError",42,1,true,true]"#,
            "{imports}"
        );
        handle.terminate_and_join();
        server.assert_no_more_requests();
    }
}
