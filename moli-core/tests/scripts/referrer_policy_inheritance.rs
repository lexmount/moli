use super::*;

async fn check_referrer_policy_inheritance(nested_parent: bool) -> Result<()> {
    let servers = IntegrityServers::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!(
            "{}/document-referrer.html?response={}",
            servers.origin,
            if nested_parent {
                "no-referrer"
            } else {
                "unsafe-url"
            },
        ))
        .await?;
    let probe = format!(
        r#"(async () => {{
            const source = location.origin + '/private-path?inheritance';
            const origin = location.origin + '/';
            let requestId = 0;
            function policy(doc, value) {{
                const meta = doc.createElement('meta');
                meta.name = 'referrer'; meta.content = value;
                doc.head.append(meta); meta.remove();
            }}
            function spawn(owner, kind) {{
                top.__inheritanceProgress = 'spawn ' + kind;
                if (kind === 'popup') {{
                    const popup = owner.open('about:blank');
                    return {{loaded:Promise.resolve(), window:() => popup, close:() => popup.close()}};
                }}
                const frame = owner.document.createElement('iframe');
                // An iframe's navigation policy must not become the new
                // Document's default policy for subsequent Fetch requests.
                frame.setAttribute('referrerpolicy', 'unsafe-url');
                let loaded = Promise.resolve();
                if (kind !== 'initial') {{
                    loaded = new Promise(resolve => {{frame.onload = resolve;}});
                    if (kind === 'srcdoc') frame.srcdoc = '<!doctype html><body>Inherited policy';
                    else frame.src = 'about:blank';
                }}
                owner.document.body.append(frame);
                return {{loaded, window:() => frame.contentWindow, close:() => frame.remove()}};
            }}
            async function check(owner, expected, label) {{
                for (const target of {origins}) {{
                    top.__inheritanceProgress = label + ': ' + target;
                    const url = target + '/meta-policy-fetch?inheritance-' + ++requestId;
                    const actual = await (await owner.fetch(url, {{referrer:source}})).json();
                    if (actual !== expected) throw new Error(label + ': ' + actual + ' != ' + expected);
                    const explicit = await (await owner.fetch(url + '-explicit', {{
                        referrer:source, referrerPolicy:'unsafe-url'
                    }})).json();
                    if (explicit !== source) throw new Error(label + ': explicit policy ' + explicit);
                }}
            }}
            for (const kind of ['initial', 'about:blank', 'srcdoc', 'popup']) {{
                policy(document, 'origin');
                const older = spawn(window, kind);
                // The initial empty Document inherits synchronously, before
                // reading contentWindow can materialize its realm.
                if (kind !== 'initial') await older.loaded;
                policy(document, 'no-referrer');
                const child = older.window();
                await check(child, origin, kind + ': creation snapshot');
                const newer = spawn(window, kind); await newer.loaded;
                await check(newer.window(), null, kind + ': later creation');
                policy(document, 'unsafe-url');
                await check(child, origin, kind + ': later parent relaxation');
                await check(newer.window(), null, kind + ': older no-referrer retained');

                policy(child.document, 'no-referrer');
                const grandchild = spawn(child, 'initial');
                await check(grandchild.window(), null, kind + ': immediate parent policy');
                policy(child.document, 'origin');
                await check(grandchild.window(), null, kind + ': grandchild snapshot retained');
                const laterGrandchild = spawn(child, 'initial');
                await check(laterGrandchild.window(), origin, kind + ': later grandchild');
                await check(window, source, kind + ': creator unchanged');
                older.close(); newer.close();
            }}
            return true;
        }})()"#,
        origins = serde_json::to_string(&[&servers.origin, &servers.cross_origin])?,
    );
    let expression = if nested_parent {
        format!(
            r#"(async () => {{
                const frame = document.createElement('iframe');
                frame.src = {url};
                await new Promise(resolve => {{frame.onload=resolve;document.body.append(frame);}});
                return frame.contentWindow.eval({probe});
            }})()"#,
            url = serde_json::to_string(&format!(
                "{}/document-referrer.html?response=unsafe-url&parent=child",
                servers.origin,
            ))?,
            probe = serde_json::to_string(&probe)?,
        )
    } else {
        probe
    };
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await;
    let result = match result {
        Ok(result) => result?,
        Err(error) => {
            let state = page
                .evaluate_runtime_expression_with_await_async(
                    "String(globalThis.__inheritanceProgress)",
                    false,
                )
                .await?;
            panic!("nested_parent={nested_parent}: {error}; progress={state}");
        }
    };
    assert_eq!(
        result["value"], true,
        "nested_parent={nested_parent}: {result}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_referrer_policy_inheritance_is_a_creation_snapshot() -> Result<()> {
    check_referrer_policy_inheritance(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_document_referrer_policy_inheritance_uses_the_immediate_parent() -> Result<()> {
    check_referrer_policy_inheritance(true).await
}
