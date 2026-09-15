use super::source_phase::ModuleSourceServer;
use super::*;

#[tokio::test]
async fn worker_json_parse_errors_prevent_sibling_execution_and_remain_cached() {
    ensure_v8();
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
            r#"
        import './side.mjs';
        import data from './bad.json' with {type:'json'};
        postMessage('unexpected evaluation');
    "#,
        )
        .await;
    server
        .respond(
            "/worker/side.mjs",
            "200 OK",
            "text/javascript",
            "++self.runs;",
        )
        .await;
    server
        .respond(
            "/worker/bad.json",
            "200 OK",
            "application/json",
            "{\n\"key\":\n}",
        )
        .await;
    assert_eq!(
        recv_post_json(&mut handle).await,
        "[0,0,true,true,true,true]"
    );
    handle.terminate_and_join();
    server.assert_no_more_requests();
}

#[tokio::test]
async fn worker_json_data_module_parse_errors_keep_syntax_error_identity() {
    ensure_v8();
    let mut handle = spawn_worker(
        r#"
        const url = 'data:application/json,%7Bbad';
        const capture = () => import(url, {with:{type:'json'}}).catch(error => error);
        Promise.all([capture(), capture()]).then(async errors => {
            const repeated = await capture();
            postMessage([errors[0] instanceof SyntaxError,
                errors[0] === errors[1], errors[0] === repeated]);
        });
    "#
        .into(),
        "https://json-errors.test/worker.js".into(),
    );
    assert_eq!(recv_post_json(&mut handle).await, "[true,true,true]");
    handle.terminate_and_join();
}
