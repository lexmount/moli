use super::*;

pub(super) fn fixture_response(path: &str, cross: &str) -> (&'static str, String, String) {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let javascript = "Content-Type: text/javascript\r\nCache-Control: no-store\r\n";
    match path {
        "/module-records.html" => (
            "200 OK",
            "Content-Type: text/html\r\nCache-Control: no-store\r\n".to_owned(),
            r#"<!doctype html><body>
                <script>globalThis.moduleRecords = []; globalThis.moduleLoads = 0;</script>
                <script type="module" src="/module-record.js" crossorigin="anonymous" onload="moduleLoads++"></script>
                <script type="module" src="/module-record.js" crossorigin="use-credentials" onload="moduleLoads++"></script>
                <script type="module" src="/module-record-wrapper.js" onload="moduleLoads++"></script>
                <script type="module" src="/module-record.js" onload="moduleLoads++"></script>
                <script type="module">globalThis.inlineRuns = (globalThis.inlineRuns || 0) + 1;</script>
                <script type="module">globalThis.inlineRuns = (globalThis.inlineRuns || 0) + 1;</script>
            "#
            .to_owned(),
        ),
        "/module-record.js" => (
            "200 OK",
            javascript.to_owned(),
            format!(
                "export const identity = {{}}; globalThis.moduleRecords.push(identity); \
                 export const later = () => import('{cross}/echo-origin.js?child-record-later');"
            ),
        ),
        "/module-record-tla-page.html" => (
            "200 OK",
            "Content-Type: text/html\r\nCache-Control: no-store\r\n".to_owned(),
            format!(r#"<!doctype html><head><script>
                globalThis.moduleStarts = 0; globalThis.moduleLoads = 0;
                globalThis.moduleStarted = () => {{}};
                globalThis.moduleGate = new Promise((resolve, reject) => {{
                    globalThis.finishModuleGate = resolve; globalThis.rejectModuleGate = reject;
                }});
            </script>
            <script type="module" src="/module-record-tla.js?{query}" onload="moduleLoads++"></script>
            <script type="module" src="/module-record-tla.js?{query}" onload="moduleLoads++"></script>
            </head><body>"#),
        ),
        "/module-record-tla.js" => (
            "200 OK",
            javascript.to_owned(),
            "globalThis.moduleStarts++; globalThis.moduleStarted(); \
             await globalThis.moduleGate; export const identity = {};"
                .to_owned(),
        ),
        "/module-record-wrapper.js" => (
            "200 OK",
            javascript.to_owned(),
            "import {identity} from './module-record.js'; globalThis.wrapperRecord = identity;"
                .to_owned(),
        ),
        "/module-record-redirect.js" => (
            "302 Found",
            format!("{javascript}Location: /module-record.js\r\n"),
            String::new(),
        ),
        _ => unreachable!("unexpected module record fixture: {path}"),
    }
}

async fn evaluate_record_probe(
    page: &mut moli_core::page::Page,
    expression: &str,
    servers: &IntegrityServers,
) -> Result<serde_json::Value> {
    match tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(expression, true),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let snapshot = tokio::time::timeout(
                Duration::from_secs(2),
                page.evaluate_runtime_expression_without_navigation_follow_with_await_async(
                    r#"JSON.stringify(Array.from(document.querySelectorAll('iframe'), frame => {
                        const win = frame.contentWindow;
                        return {src: frame.src, readyState: win.document.readyState,
                            records: win.moduleRecords?.length, loads: win.moduleLoads,
                            inlineRuns: win.inlineRuns, starts: win.moduleStarts,
                            scriptCount: win.document.scripts.length};
                    }))"#,
                    false,
                ),
            )
            .await;
            Err(anyhow::Error::new(error).context(format!(
                "awaited child module record probe: snapshot={snapshot:?}; requests={:?}; errors={:?}; console={:?}",
                servers.requests.lock(), page.script_execution().lifecycle_errors(),
                page.script_execution().console_messages(),
            )))
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_module_scripts_reuse_records_within_each_realm() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/cookie-page.html", servers.origin))
        .await?;
    let probe = r#"(async () => {
        const check = (value, message) => { if (!value) throw new Error(message); };
        check(moduleRecords.length === 1, 'parser roots and static dependency share evaluation');
        check(moduleLoads === 4, 'each parser script receives its load event');
        check(inlineRuns === 2, 'identical inline scripts have independent records');
        const first = await import('/module-record.js');
        check(first.identity === moduleRecords[0], 'import reuses parser record');
        check(first.identity === wrapperRecord, 'static dependency uses same namespace');
        check(first === await import('/module-record.js'), 'cached imports retain namespace identity');
        check(moduleRecords.length === 1, 'cached imports reuse parser evaluation');
        // The cached record retains the first script's anonymous credentials.
        await first.later();
        const redirected = await import('/module-record-redirect.js');
        check(redirected !== first, 'different request URLs have separate module records');
        check(redirected.identity !== first.identity, 'redirect does not alias the request key');
        check(redirected === await import('/module-record-redirect.js'), 'redirected record is reused');
        check(first === await import('/module-record.js'), 'redirect preserves original record');
        check(moduleRecords.length === 2, 'one evaluation per request URL');
        return true;
    })()"#;
    let expression = format!(
        r#"(async () => {{
            const frames = [];
            for (let i = 0; i < 2; i++) {{
                const frame = document.createElement('iframe');
                frame.src = '/module-records.html';
                await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                if (await frame.contentWindow.eval({probe}) !== true) throw new Error('child probe');
                frames.push(frame.contentWindow);
            }}
            if (frames[0].moduleRecords[0] === frames[1].moduleRecords[0])
                throw new Error('record leaked between realms');
            if (!(frames[0].moduleRecords[0] instanceof frames[0].Object))
                throw new Error('record created in wrong realm');
            return globalThis.moduleRecords === undefined;
        }})()"#,
        probe = serde_json::to_string(probe)?,
    );
    let result = evaluate_record_probe(&mut page, &expression, &servers).await?;
    assert_eq!(
        result["value"],
        true,
        "{result}; {:?}",
        servers.requests.lock()
    );
    let requests = servers.requests.lock();
    for (path, count) in [
        ("/module-record.js", 4),
        ("/module-record-wrapper.js", 2),
        ("/module-record-redirect.js", 2),
        ("/echo-origin.js?child-record-later", 2),
    ] {
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == path)
                .count(),
            count,
            "{path}: {requests:?}"
        );
    }
    for request in requests
        .iter()
        .filter(|request| request.path == "/echo-origin.js?child-record-later")
    {
        assert_eq!(
            request.cookie, None,
            "cached module credentials: {request:?}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_module_consumers_share_pending_evaluation() -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/page.html", servers.origin))
        .await?;
    let probe = r#"(async () => {
        const check = (value, message) => { if (!value) throw new Error(message); };
        const fail = FAILURE_MODE;
        const url = '/module-record-tla.js?fail=' + fail;
        check(moduleStarts === 1, 'parser consumers share pending evaluation');
        check(moduleLoads === 2, 'each parser consumer loads before TLA settles');
        let settled = false;
        const imported = import(url).then(value => { settled = true; return {value}; },
            error => { settled = true; return {error}; });
        const sibling = import(url).then(value => ({value}), error => ({error}));
        await Promise.resolve();
        check(!settled, 'imports must remain pending before the gate resolves');
        check(moduleStarts === 1, 'imports must reuse the evaluating parser record');
        const sentinel = {failure: true};
        if (fail) rejectModuleGate(sentinel); else finishModuleGate();
        const first = await imported, second = await sibling;
        const third = await import(url).then(value => ({value}), error => ({error}));
        if (fail) check(first.error === sentinel && second.error === sentinel &&
            third.error === sentinel, 'rejection identity');
        else check(first.value === second.value && second.value === third.value,
            'fulfilled namespace identity');
        check(moduleStarts === 1 && moduleLoads === 2, 'settlement must not restart parser consumers');
        return true;
    })()"#;
    let expression = format!(
        r#"(async () => {{
            const probe = {};
            for (const fail of [false, true]) {{
                const frame = document.createElement('iframe');
                frame.src = '/module-record-tla-page.html?fail=' + fail;
                await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                if (await frame.contentWindow.eval(probe.replace('FAILURE_MODE', String(fail))) !== true)
                    throw new Error('child pending parser record probe');
            }}
            return true;
        }})()"#,
        serde_json::to_string(probe)?,
    );
    let result = evaluate_record_probe(&mut page, &expression, &servers).await?;
    assert_eq!(
        result["value"],
        true,
        "{result}; {:?}",
        servers.requests.lock()
    );
    let requests = servers.requests.lock();
    for failed in [false, true] {
        let path = format!("/module-record-tla.js?fail={failed}");
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == path)
                .count(),
            1,
            "{path}: {requests:?}"
        );
    }
    Ok(())
}
