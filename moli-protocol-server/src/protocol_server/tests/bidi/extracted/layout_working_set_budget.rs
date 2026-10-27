use super::*;
use moli_core::{LayoutWorkingSetBudget, LayoutWorkingSetBudgetSnapshot};

type BudgetTestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn budget_server(
    pool: &LayoutWorkingSetBudget,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_test_protocol_server_with_runtime_config(
        NavigationRuntimeConfig::new(
            protocol_server_test_fetch_config(FetchConfig::default()),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::Realtime,
        )
        .with_layout_working_set_budget(pool.clone()),
    )
    .await
}

fn budget_width_expression(width: u32) -> String {
    format!(
        "(() => {{\
         let box=document.getElementById('budget-box');\
         if(!box){{box=document.createElement('div');box.id='budget-box';\
         box.style.cssText='display:block;height:20px;padding:0;border:0';\
         (document.body||document.documentElement).appendChild(box);}}\
         box.style.width='{width}px';return box.offsetWidth;}})()"
    )
}

async fn budget_cdp_command(
    socket: &mut BudgetTestSocket,
    id: u64,
    method: &str,
    session: Option<&str>,
    params: serde_json::Value,
) -> serde_json::Value {
    let response = send_cdp_command(socket, id, method, session, params)
        .await
        .into_iter()
        .find(|message| message["id"] == json!(id))
        .expect("CDP command response");
    assert!(response.get("error").is_none(), "{method}: {response}");
    response
}

async fn budget_cdp_width(socket: &mut BudgetTestSocket, id: u64, session: &str, width: u32) {
    let response = budget_cdp_command(
        socket,
        id,
        "Runtime.evaluate",
        Some(session),
        json!({"expression": budget_width_expression(width), "returnByValue": true}),
    )
    .await;
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        json!(width),
        "{response}"
    );
}

async fn budget_bidi_width(socket: &mut BudgetTestSocket, id: u64, context: &str, width: u32) {
    let response = send_bidi_command_response(
        socket,
        id,
        "script.evaluate",
        json!({
            "expression": budget_width_expression(width),
            "target": {"context": context},
            "awaitPromise": false,
        }),
    )
    .await;
    assert_eq!(response["type"], "success", "{response}");
    assert_eq!(response["result"]["type"], "success", "{response}");
    assert_eq!(
        response["result"]["result"]["value"],
        json!(width),
        "{response}"
    );
}

async fn budget_classic_width(addr: std::net::SocketAddr, session: &str, width: u32) {
    let response = timeout(
        Duration::from_secs(30),
        classic_request_on_server_with_body(
            addr,
            "POST",
            &format!("/session/{session}/execute/sync"),
            json!({"script": format!("return {};", budget_width_expression(width)), "args": []}),
        ),
    )
    .await
    .expect("Classic geometry command completes");
    assert_eq!(response["value"], json!(width), "{response}");
}

async fn wait_for_budget_snapshot(
    pool: &LayoutWorkingSetBudget,
    description: &str,
    predicate: impl Fn(&LayoutWorkingSetBudgetSnapshot) -> bool,
) -> LayoutWorkingSetBudgetSnapshot {
    // This waits for transport/owner teardown, never for a desired budget
    // policy interval. Every pressure assertion also forces a real layout read.
    timeout(Duration::from_secs(30), async {
        loop {
            let snapshot = pool.snapshot();
            if predicate(&snapshot) {
                break snapshot;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{description}: last snapshot {:?}", pool.snapshot()))
}

async fn wait_for_empty_budget(pool: &LayoutWorkingSetBudget) {
    wait_for_budget_snapshot(
        pool,
        "all protocol Pages and payloads retired",
        |snapshot| {
            snapshot.open_pages == 0
                && snapshot.charged_bytes == 0
                && snapshot.live_leases == 0
                && snapshot.queued_pages == 0
                && snapshot.ready_bytes == 0
        },
    )
    .await;
}

#[tokio::test]
async fn socket_cdp_bidi_and_classic_pages_share_one_realtime_retention_pool() {
    let pool = LayoutWorkingSetBudget::default();
    let (addr, server) = budget_server(&pool).await;
    assert_eq!(pool.snapshot().open_pages, 0);
    assert_eq!(pool.snapshot().charged_bytes, 0);

    let (mut cdp, _) = connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
        .await
        .expect("CDP browser connection");
    let cdp_context = cdp_create_browser_context(&mut cdp, 1).await;
    let cdp_target = cdp_create_attached_target(&mut cdp, 2, &cdp_context).await;
    budget_cdp_width(&mut cdp, 4, &cdp_target.session_id, 100).await;
    let cdp_retained = pool.snapshot();
    assert!(cdp_retained.open_pages > 0, "{cdp_retained:?}");
    assert!(
        cdp_retained.charged_bytes > 0 && cdp_retained.live_leases > 0,
        "{cdp_retained:?}"
    );

    let (mut bidi, bidi_context) = bidi_session_with_context(addr).await;
    budget_bidi_width(&mut bidi, 3, &bidi_context, 200).await;
    let bidi_retained = pool.snapshot();
    assert!(
        bidi_retained.open_pages > cdp_retained.open_pages,
        "{bidi_retained:?}"
    );
    assert!(
        bidi_retained.live_leases > cdp_retained.live_leases,
        "{bidi_retained:?}"
    );

    let classic_session = classic_new_session_on_server(addr).await;
    budget_classic_width(addr, &classic_session, 300).await;
    let all_retained = pool.snapshot();
    assert!(
        all_retained.open_pages > bidi_retained.open_pages,
        "{all_retained:?}"
    );
    assert!(
        all_retained.live_leases > bidi_retained.live_leases,
        "{all_retained:?}"
    );

    // One external pressure request must govern all three actual adapters.
    // They continue to return fresh geometry without retaining computation.
    pool.set_pressure_target(0).unwrap();
    budget_cdp_width(&mut cdp, 5, &cdp_target.session_id, 110).await;
    budget_bidi_width(&mut bidi, 4, &bidi_context, 210).await;
    budget_classic_width(addr, &classic_session, 310).await;
    let trimmed = wait_for_budget_snapshot(&pool, "shared zero-pressure trim", |snapshot| {
        snapshot.charged_bytes == 0 && snapshot.live_leases == 0
    })
    .await;
    assert_eq!(trimmed.open_pages, all_retained.open_pages);
    assert_eq!(trimmed.queued_pages, 0);

    pool.set_pressure_target(pool.limits().browser_bytes)
        .unwrap();
    budget_cdp_width(&mut cdp, 6, &cdp_target.session_id, 120).await;
    let cdp_rebuilt = pool.snapshot();
    assert!(
        cdp_rebuilt.charged_bytes > 0 && cdp_rebuilt.live_leases > 0,
        "{cdp_rebuilt:?}"
    );
    budget_bidi_width(&mut bidi, 5, &bidi_context, 220).await;
    let bidi_rebuilt = pool.snapshot();
    assert!(
        bidi_rebuilt.live_leases > cdp_rebuilt.live_leases,
        "{bidi_rebuilt:?}"
    );
    budget_classic_width(addr, &classic_session, 320).await;
    assert!(pool.snapshot().live_leases > bidi_rebuilt.live_leases);

    budget_cdp_command(
        &mut cdp,
        7,
        "Target.disposeBrowserContext",
        None,
        json!({"browserContextId": cdp_context}),
    )
    .await;
    let ended = send_bidi_command_response(&mut bidi, 6, "session.end", json!({})).await;
    assert_eq!(ended["type"], "success", "{ended}");
    let deleted = classic_request_on_server_with_body(
        addr,
        "DELETE",
        &format!("/session/{classic_session}"),
        json!({}),
    )
    .await;
    assert_eq!(deleted, json!({"value": null}));
    let _ = cdp.close(None).await;
    let _ = bidi.close(None).await;
    wait_for_empty_budget(&pool).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn socket_cdp_browser_context_recreation_keeps_the_root_budget() {
    let pool = LayoutWorkingSetBudget::default();
    let (addr, server) = budget_server(&pool).await;
    let (mut socket, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .expect("CDP browser connection");
    let first_context = cdp_create_browser_context(&mut socket, 1).await;
    let first = cdp_create_attached_target(&mut socket, 2, &first_context).await;
    budget_cdp_width(&mut socket, 4, &first.session_id, 111).await;
    let first_retained = pool.snapshot();
    assert!(
        first_retained.open_pages > 0 && first_retained.live_leases > 0,
        "{first_retained:?}"
    );

    let second_context = cdp_create_browser_context(&mut socket, 5).await;
    let second = cdp_create_attached_target(&mut socket, 6, &second_context).await;
    budget_cdp_width(&mut socket, 8, &second.session_id, 222).await;
    let both = pool.snapshot();
    assert!(both.open_pages > first_retained.open_pages, "{both:?}");
    assert!(both.live_leases > first_retained.live_leases, "{both:?}");

    budget_cdp_command(
        &mut socket,
        9,
        "Target.disposeBrowserContext",
        None,
        json!({"browserContextId": first_context}),
    )
    .await;
    let only_second = wait_for_budget_snapshot(&pool, "first context retired", |snapshot| {
        snapshot.open_pages > 0
            && snapshot.open_pages < both.open_pages
            && snapshot.live_leases > 0
            && snapshot.live_leases < both.live_leases
    })
    .await;
    assert!(only_second.charged_bytes > 0);
    budget_cdp_width(&mut socket, 10, &second.session_id, 223).await;
    budget_cdp_command(
        &mut socket,
        11,
        "Target.disposeBrowserContext",
        None,
        json!({"browserContextId": second_context}),
    )
    .await;
    wait_for_empty_budget(&pool).await;

    // Recreating contexts after all targets have closed must preserve the
    // original pool, including its current pressure. The connection-level
    // diagnostic separately verifies actual idle-engine replacement.
    pool.set_pressure_target(0).unwrap();
    let replacement_context = cdp_create_browser_context(&mut socket, 13).await;
    let replacement = cdp_create_attached_target(&mut socket, 14, &replacement_context).await;
    budget_cdp_width(&mut socket, 16, &replacement.session_id, 333).await;
    let zero_pressure = pool.snapshot();
    assert!(zero_pressure.open_pages > 0, "{zero_pressure:?}");
    assert_eq!(zero_pressure.charged_bytes, 0);
    assert_eq!(zero_pressure.live_leases, 0);

    pool.set_pressure_target(pool.limits().browser_bytes)
        .unwrap();
    budget_cdp_width(&mut socket, 17, &replacement.session_id, 444).await;
    let recovered = pool.snapshot();
    assert_eq!(recovered.open_pages, zero_pressure.open_pages);
    assert!(
        recovered.charged_bytes > 0 && recovered.live_leases > 0,
        "{recovered:?}"
    );
    budget_cdp_command(
        &mut socket,
        18,
        "Target.disposeBrowserContext",
        None,
        json!({"browserContextId": replacement_context}),
    )
    .await;
    wait_for_empty_budget(&pool).await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
