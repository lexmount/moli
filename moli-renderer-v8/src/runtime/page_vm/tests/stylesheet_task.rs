use super::*;

use crate::page_task_queue::{
    PageConnectedStyleEventTargetEffect, PageConnectedStyleLoadDelayEffect,
    PageDomManipulationTurnAction, PageNetworkingTurnAction, PageStylesheetNetworkingTargetEffect,
    RendererOwnerWakeSource, RendererPageConnectedStyleEventTask,
};
use crate::runtime::PageTaskCompletion;

fn take_next_style_element_event_task_for_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageConnectedStyleEventTask> {
    let sources = page_vm.page_task_executor_sources_for_test();
    let task = sources.take_scheduler_task_for_executor_test(|descriptor| {
        matches!(
            descriptor,
            crate::page_task_queue::RendererPageReadyDescriptor::Networking {
                owner: crate::page_task_queue::RendererPageNetworkingOwner::StyleElementEvent(_),
                ..
            }
        )
    })?;
    let crate::page_task_queue::RendererPageSchedulerTask::Networking(
        crate::page_task_queue::RendererPageNetworkingTask::StyleElementEvent(task),
    ) = task
    else {
        unreachable!("style-element descriptor must dequeue its Networking task")
    };
    Some(task)
}

fn take_next_link_element_event_task_for_test(
    page_vm: &mut PageVm,
) -> Option<RendererPageConnectedStyleEventTask> {
    let task = page_vm.take_dom_manipulation_body_task_for_test(
        PageDomManipulationTestFamily::ConnectedStyleEvent,
    )?;
    let crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(task) = task
    else {
        unreachable!("exact connected-style selection must preserve its task variant")
    };
    Some(task)
}

async fn wait_for_stylesheet_source(
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    expected: RendererOwnerWakeSource,
) {
    let mut observed = Vec::new();
    let arrival = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let wake = wake_rx
                .recv()
                .await
                .expect("stylesheet Page route must remain attached");
            let source = wake.source_for_test();
            observed.push(source);
            if source == expected {
                break;
            }
        }
    })
    .await;
    assert!(
        arrival.is_ok(),
        "stylesheet source should become ready without an external retry; expected {expected:?}, observed {observed:?}"
    );
}

async fn complete_stylesheet_preload_for_cache_hit(
    page_vm: &mut PageVm,
    wake_rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
    loader: &crate::network::ResourceRequestClient,
    href: &str,
) -> anyhow::Result<()> {
    let href = serde_json::to_string(href)?;
    page_vm.vm_mut().eval(&format!(
        r#"
const cachedPreload = document.createElement("link");
cachedPreload.rel = "preload";
cachedPreload.as = "style";
cachedPreload.href = {href};
document.head.append(cachedPreload);
"queued"
"#,
    ))?;
    page_vm
        .vm_mut()
        .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    wait_for_stylesheet_source(wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
    assert!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::StylesheetCompletion,
                loader,
            )
            .await?,
        "the preload physical terminal should consume one exact Networking turn"
    );
    wait_for_stylesheet_source(wake_rx, RendererOwnerWakeSource::DomManipulationTask).await;
    let event = take_next_link_element_event_task_for_test(page_vm)
        .expect("the completed preload should publish one link event");
    page_vm
        .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
            crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(event),
            loader,
        )
        .await?;
    Ok(())
}

fn parser_captured_stylesheet_input_for_test(
    page_vm: &PageVm,
    id: &str,
) -> crate::DocumentOwnedBlockingStylesheetDiscoveryInput {
    let link = page_vm
        .vm()
        .document_runtime
        .get_element_by_id(id)
        .expect("test link should be connected");
    let node_id = crate::dom::NodeId::new(link.index());
    let disposition = crate::stylesheet_blocking::stylesheet_link_disposition(
        page_vm.vm().document_runtime.dom_host(),
        node_id,
    )
    .expect("test link should have a stylesheet disposition");
    assert!(
        disposition.is_blocking(),
        "test link should represent a parser-blocking stylesheet"
    );
    let candidate = moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetCandidate::Link {
        node_id,
        url: disposition.url().clone(),
        options: disposition.options().clone(),
    };
    crate::DocumentOwnedBlockingStylesheetDiscoveryInput::from(&candidate)
}

fn note_parser_captured_stylesheet_inputs_for_test(
    page_vm: &mut PageVm,
    inputs: &[crate::DocumentOwnedBlockingStylesheetDiscoveryInput],
) {
    let completed_clients = page_vm
        .vm_mut()
        .document_runtime
        .note_discovered_document_owned_blocking_stylesheet_inputs(inputs);
    page_vm
        .vm_mut()
        .settle_stylesheet_link_clients(completed_clients);
}

fn queue_stylesheet_checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> anyhow::Result<()> {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!(
            r#"
globalThis.{marker} = 0;
Promise.resolve().then(() => globalThis.{marker} += 1);
"reaction queued"
"#,
        ))?;
    Ok(())
}

fn stylesheet_checkpoint_marker(page_vm: &mut PageVm, marker: &str) -> anyhow::Result<String> {
    page_vm
        .vm_mut()
        .eval_without_microtask_checkpoint_for_test(&format!("String(globalThis.{marker})"))
}

#[tokio::test(flavor = "current_thread")]
async fn completed_data_preload_settles_new_stylesheet_client_synchronously() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/stylesheet-completed-cache-hit").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let href = "data:text/css,.completed-cache-hit%7Bcolor%3Argb(1%2C%202%2C%203)%7D";
        complete_stylesheet_preload_for_cache_hit(&mut page_vm, &mut wake_rx, &loader, href)
            .await?;

        let href = serde_json::to_string(href)?;
        let synchronous_state = page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__completedCacheHitEvents = [];
const cachedTarget = document.createElement("div");
cachedTarget.className = "completed-cache-hit";
document.body.append(cachedTarget);
const cachedStyle = document.createElement("link");
cachedStyle.id = "completed-cache-hit-style";
cachedStyle.rel = "stylesheet";
cachedStyle.href = {href};
cachedStyle.addEventListener("load", () => __completedCacheHitEvents.push("load"));
cachedStyle.addEventListener("error", () => __completedCacheHitEvents.push("error"));
document.head.append(cachedStyle);
[
  Object.prototype.toString.call(cachedStyle.sheet),
  getComputedStyle(cachedTarget).color,
  __completedCacheHitEvents.join(",")
].join("|")
"#,
        ))?;
        assert_eq!(
            synchronous_state,
            "[object CSSStyleSheet]|rgb(1, 2, 3)|",
            "a completed cache hit must install and style in the same append call while its link event remains asynchronous"
        );

        assert!(
            page_vm
                .take_stylesheet_networking_body_task_for_test()
                .is_none(),
            "a completed cache hit must not manufacture another physical Networking terminal"
        );
        assert!(
            page_vm.has_ready_dom_manipulation_task_for_test(),
            "client admission must publish the later link event without an external parser retry"
        );

        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the synchronously settled stylesheet should retain one link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(event),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__completedCacheHitEvents.join('|')")?,
            "load"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("completed data stylesheet cache-hit test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn completed_data_import_graph_settles_late_stylesheet_client() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/stylesheet-completed-data-import").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let href = concat!(
            "data:text/css,@import%20url(%22data:text/css,",
            ".late-data-imported%257Bcolor%253Argb(21%252C%252022%252C%252023)%257D",
            "%22)%3B"
        );
        complete_stylesheet_preload_for_cache_hit(&mut page_vm, &mut wake_rx, &loader, href)
            .await?;

        let href = serde_json::to_string(href)?;
        assert_eq!(
            page_vm.vm_mut().eval(&format!(
                r#"
globalThis.__lateDataImportEvents = [];
const target = document.createElement("div");
target.className = "late-data-imported";
document.body.append(target);
const first = document.createElement("link");
first.id = "data-import-first";
first.rel = "stylesheet";
first.href = {href};
first.addEventListener("load", () => __lateDataImportEvents.push("first-load"));
first.addEventListener("error", () => __lateDataImportEvents.push("first-error"));
document.head.append(first);
[
  Object.prototype.toString.call(first.sheet),
  getComputedStyle(target).color,
  __lateDataImportEvents.join(",")
].join("|")
"#,
            ))?,
            "[object CSSStyleSheet]|rgb(21, 22, 23)|",
            "the first client must complete the shared data import graph before its asynchronous event"
        );
        let first_event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the first data import client should publish one load event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    first_event,
                ),
                &loader,
            )
            .await?;

        assert_eq!(
            page_vm.vm_mut().eval(&format!(
                r#"
const second = document.createElement("link");
second.id = "data-import-second";
second.rel = "stylesheet";
second.href = {href};
second.addEventListener("load", () => __lateDataImportEvents.push("second-load"));
second.addEventListener("error", () => __lateDataImportEvents.push("second-error"));
document.head.append(second);
[
  Object.prototype.toString.call(second.sheet),
  getComputedStyle(target).color,
  __lateDataImportEvents.join(",")
].join("|")
"#,
            ))?,
            "[object CSSStyleSheet]|rgb(21, 22, 23)|first-load",
            "a late client must synchronously install the retained data import graph"
        );
        assert!(
            !page_vm.has_ready_page_networking_task(),
            "late data graph installation must not create another Networking task"
        );

        let second_event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the late data import client must publish one terminal event");
        let body = page_vm.apply_selected_page_connected_style_event_turn(second_event)?;
        assert_eq!(
            body.action.target_effect,
            PageConnectedStyleEventTargetEffect::DispatchedToCurrentOwner {
                load_delay_effect: PageConnectedStyleLoadDelayEffect::ReleasedExactBinding,
            },
            "the late client event must release its exact Document load-delay binding"
        );
        page_vm
            .finish_selected_page_dom_manipulation_task(
                PageDomManipulationTurnAction::ConnectedStyleEvent(body.action),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__lateDataImportEvents.join(',')")?,
            "first-load,second-load"
        );
        assert!(
            take_next_link_element_event_task_for_test(&mut page_vm).is_none(),
            "each data import client must settle exactly once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("completed data import graph late-client test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn completed_preload_settlement_starts_imports_before_publishing_link_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/cached-root.css",
                "HTTP/1.1 200 OK",
                "@import './cached-child.css'; .cached-root { color: rgb(4, 5, 6); }"
                    .to_owned(),
                Duration::ZERO,
            ),
            (
                "/cached-child.css",
                "HTTP/1.1 200 OK",
                ".cached-child { color: rgb(7, 8, 9); }".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let href = format!("{base_url}/cached-root.css");
        complete_stylesheet_preload_for_cache_hit(&mut page_vm, &mut wake_rx, &loader, &href)
            .await?;

        let href = serde_json::to_string(&href)?;
        let synchronous_state = page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__cachedImportEvents = [];
for (const className of ["cached-root", "cached-child"]) {{
  const target = document.createElement("div");
  target.className = className;
  document.body.append(target);
}}
const cachedImport = document.createElement("link");
cachedImport.id = "cached-import";
cachedImport.rel = "stylesheet";
cachedImport.href = {href};
cachedImport.addEventListener("load", () => __cachedImportEvents.push("load"));
cachedImport.addEventListener("error", () => __cachedImportEvents.push("error"));
document.head.append(cachedImport);
[
  Object.prototype.toString.call(cachedImport.sheet),
  String(cachedImport.sheet.cssRules[0].styleSheet),
  getComputedStyle(document.querySelector(".cached-root")).color,
  __cachedImportEvents.join(",")
].join("|")
"#,
        ))?;

        assert_eq!(
            synchronous_state,
            "[object CSSStyleSheet]|null|rgb(4, 5, 6)|",
            "client admission must synchronously install the root and start its import while keeping the event asynchronous"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "the cached link event must wait for its import graph"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the cache-hit import graph should complete as Networking work"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                r##"
(() => {
  const sheet = document.querySelector("#cached-import").sheet;
  const child = sheet.cssRules[0].styleSheet;
  return [
    Object.prototype.toString.call(child),
    child.cssRules.length,
    getComputedStyle(document.querySelector(".cached-child")).color,
    __cachedImportEvents.join(",")
  ].join("|");
})()
"##,
            )?,
            "[object CSSStyleSheet]|1|rgb(7, 8, 9)|"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the completed cache-hit import graph should publish one link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(event),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__cachedImportEvents.join('|')")?,
            "load"
        );
        server.await.expect("cached import fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("completed stylesheet cache-hit import test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn separately_appended_cache_hit_links_share_import_fetch_but_install_both_graphs() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/shared-root.css",
                "HTTP/1.1 200 OK",
                "@import './slow-child.css';".to_owned(),
                Duration::ZERO,
            ),
            (
                "/slow-child.css",
                "HTTP/1.1 200 OK",
                ".shared-import-target { color: rgb(17, 34, 51); }".to_owned(),
                Duration::from_millis(100),
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let href = format!("{base_url}/shared-root.css");
        complete_stylesheet_preload_for_cache_hit(&mut page_vm, &mut wake_rx, &loader, &href)
            .await?;

        let href = serde_json::to_string(&href)?;
        assert_eq!(
            page_vm.vm_mut().eval(&format!(
                r#"
globalThis.__sharedImportEvents = [];
const target = document.createElement("div");
target.className = "shared-import-target";
document.body.append(target);
const first = document.createElement("link");
first.id = "shared-import-first";
first.rel = "stylesheet";
first.href = {href};
first.addEventListener("load", () => __sharedImportEvents.push("first-load"));
document.head.append(first);
const second = document.createElement("link");
second.id = "shared-import-second";
second.rel = "stylesheet";
second.href = {href};
second.addEventListener("load", () => __sharedImportEvents.push("second-load"));
document.head.append(second);
[
  String(first.sheet.cssRules[0].styleSheet),
  String(second.sheet.cssRules[0].styleSheet),
  __sharedImportEvents.join(",")
].join("|")
"#,
            ))?,
            "null|null|",
            "both owners must synchronously install their root sheet while the shared import remains pending"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the shared import graph must complete as one Networking turn"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                r##"
(() => {
  const first = document.querySelector("#shared-import-first");
  const second = document.querySelector("#shared-import-second");
  return [
    first.sheet.cssRules[0].styleSheet !== null,
    second.sheet.cssRules[0].styleSheet !== null,
    getComputedStyle(document.querySelector(".shared-import-target")).color
  ].join("|");
})()
"##,
            )?,
            "true|true|rgb(17, 34, 51)",
            "the shared terminal must install the import graph into every owner sheet"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        for _ in 0..2 {
            let event = take_next_link_element_event_task_for_test(&mut page_vm)
                .expect("each current link client must publish one load event");
            page_vm
                .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                    crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                        event,
                    ),
                    &loader,
                )
                .await?;
        }
        assert_eq!(
            page_vm.vm_mut().eval(
                r##"
(() => {
  document.querySelector("#shared-import-first").remove();
  const surviving = document.querySelector("#shared-import-second");
  return [
    surviving.sheet.cssRules[0].styleSheet !== null,
    getComputedStyle(document.querySelector(".shared-import-target")).color,
    __sharedImportEvents.sort().join(",")
  ].join("|");
})()
"##,
            )?,
            "true|rgb(17, 34, 51)|first-load,second-load",
            "removing the first owner must not remove the second owner's imported stylesheet"
        );
        assert_eq!(
            page_vm.vm_mut().eval(&format!(
                r#"
const late = document.createElement("link");
late.id = "shared-import-late";
late.rel = "stylesheet";
late.href = {href};
late.addEventListener("load", () => __sharedImportEvents.push("late-load"));
document.head.append(late);
[
  late.sheet.cssRules[0].styleSheet !== null,
  getComputedStyle(document.querySelector(".shared-import-target")).color,
  __sharedImportEvents.sort().join(",")
].join("|")
"#,
            ))?,
            "true|rgb(17, 34, 51)|first-load,second-load",
            "a client admitted after graph completion must synchronously replay the retained graph"
        );
        assert!(
            !page_vm.has_ready_page_networking_task(),
            "late per-owner graph replay must not manufacture another Networking task"
        );
        let late_event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the late graph replay must publish its link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    late_event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__sharedImportEvents.sort().join(',')")?,
            "first-load,late-load,second-load"
        );
        server.await.expect("shared cache-hit import fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("shared cache-hit stylesheet import test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_completed_preload_settles_empty_sheet_synchronously_before_link_error() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/cached-failure.css",
            "HTTP/1.1 404 Not Found",
            "not found".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let href = format!("{base_url}/cached-failure.css");
        complete_stylesheet_preload_for_cache_hit(&mut page_vm, &mut wake_rx, &loader, &href)
            .await?;

        let href = serde_json::to_string(&href)?;
        let synchronous_state = page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__failedCacheHitEvents = [];
const failedCacheHit = document.createElement("link");
failedCacheHit.id = "failed-cache-hit";
failedCacheHit.rel = "stylesheet";
failedCacheHit.href = {href};
failedCacheHit.addEventListener("load", () => __failedCacheHitEvents.push("load"));
failedCacheHit.addEventListener("error", () => __failedCacheHitEvents.push("error"));
document.head.append(failedCacheHit);
[
  Object.prototype.toString.call(failedCacheHit.sheet),
  failedCacheHit.sheet.cssRules.length,
  __failedCacheHitEvents.join(",")
].join("|")
"#,
        ))?;
        assert_eq!(
            synchronous_state,
            "[object CSSStyleSheet]|0|",
            "Blink-compatible failure settlement must install an empty CSSOM sheet in the append call while keeping error asynchronous"
        );

        assert!(
            page_vm
                .take_stylesheet_networking_body_task_for_test()
                .is_none(),
            "a failed completed cache hit must not create another physical task"
        );
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the failed cache hit should publish one link error task");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__failedCacheHitEvents.join('|')")?,
            "error"
        );
        server.await.expect("failed cache-hit fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("failed completed stylesheet cache-hit test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn posted_cache_hit_events_follow_link_and_document_lifetimes() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/checkpoint-cache-hit.css",
            "HTTP/1.1 200 OK",
            ".checkpoint-cache-hit { color: green; }".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let href = format!("{base_url}/checkpoint-cache-hit.css");
        complete_stylesheet_preload_for_cache_hit(&mut page_vm, &mut wake_rx, &loader, &href)
            .await?;
        let href = serde_json::to_string(&href)?;

        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__retiredCacheHitEvents = [];
globalThis.__hrefInvalidatedCacheHit = document.createElement("link");
__hrefInvalidatedCacheHit.rel = "stylesheet";
__hrefInvalidatedCacheHit.href = {href};
__hrefInvalidatedCacheHit.addEventListener("load", () => __retiredCacheHitEvents.push("href-load"));
__hrefInvalidatedCacheHit.addEventListener("error", () => __retiredCacheHitEvents.push("href-error"));
document.head.append(__hrefInvalidatedCacheHit);
Promise.resolve().then(() => __hrefInvalidatedCacheHit.removeAttribute("href"));
"queued"
"#,
        ))?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "[__hrefInvalidatedCacheHit.hasAttribute('href'), String(__hrefInvalidatedCacheHit.sheet), __retiredCacheHitEvents.join(',')].join('|')",
            )?,
            "false|null|"
        );
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the already-posted href-invalidated event should remain selectable");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__retiredCacheHitEvents.join(',')")?,
            "href-load",
            "removing href must not retract a load task posted by an already-settled client"
        );

        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__disconnectedCacheHit = document.createElement("link");
__disconnectedCacheHit.rel = "stylesheet";
__disconnectedCacheHit.href = {href};
__disconnectedCacheHit.addEventListener("load", () => __retiredCacheHitEvents.push("remove-load"));
__disconnectedCacheHit.addEventListener("error", () => __retiredCacheHitEvents.push("remove-error"));
document.head.append(__disconnectedCacheHit);
Promise.resolve().then(() => __disconnectedCacheHit.remove());
"queued"
"#,
        ))?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "[__disconnectedCacheHit.isConnected, String(__disconnectedCacheHit.sheet), __retiredCacheHitEvents.join(',')].join('|')",
            )?,
            "false|null|href-load"
        );
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the already-posted disconnected event should remain selectable");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__retiredCacheHitEvents.join(',')")?,
            "href-load,remove-load",
            "disconnecting the element must not retract its already-posted load task"
        );

        page_vm.vm_mut().eval(&format!(
            r#"
const replacedCacheHit = document.createElement("link");
replacedCacheHit.rel = "stylesheet";
replacedCacheHit.href = {href};
replacedCacheHit.addEventListener("load", () => __retiredCacheHitEvents.push("replace-load"));
replacedCacheHit.addEventListener("error", () => __retiredCacheHitEvents.push("replace-error"));
document.head.append(replacedCacheHit);
Promise.resolve().then(() => {{
  document.open();
  document.write("<!doctype html><body>replacement</body>");
  document.close();
}});
"queued"
"#,
        ))?;
        assert_eq!(page_vm.vm_mut().eval("document.body.textContent")?, "replacement");
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the old Document's already-posted event should remain selectable");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__retiredCacheHitEvents.join(',')")?,
            "href-load,remove-load",
            "exact-Document validation must discard the replaced Document's event task"
        );
        assert!(
            page_vm
                .take_stylesheet_networking_body_task_for_test()
                .is_none(),
            "invalidating cached clients must not create physical completion tasks"
        );
        server.await.expect("checkpoint cache-hit fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("checkpoint stylesheet cache-hit invalidation test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn completed_client_settlement_does_not_consume_an_unselected_physical_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/stylesheet-physical-residence").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__unselectedPhysicalEvents = [];
const physical = document.createElement("link");
physical.rel = "stylesheet";
physical.href = "data:text/css,.unselected%7Bcolor%3Agreen%7D";
physical.addEventListener("load", () => __unselectedPhysicalEvents.push("load"));
document.head.append(physical);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;

        page_vm
            .vm_mut()
            .eval("globalThis.__unrelatedTurnFinished = true")?;
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "an unrelated owner turn must not apply an unselected physical terminal"
        );
        let task = page_vm
            .take_stylesheet_networking_body_task_for_test()
            .expect("the physical stylesheet completion must remain in its Networking source");
        let outcome = page_vm.apply_selected_page_stylesheet_networking_turn(task)?;
        page_vm
            .finish_selected_page_networking_task(
                PageNetworkingTurnAction::StylesheetCompletion(outcome.action),
                &loader,
            )
            .await?;
        assert!(
            page_vm.has_ready_dom_manipulation_task_for_test(),
            "selecting the physical completion should publish its later link event"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("unselected physical stylesheet residence test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stylesheet_networking_body_leaves_checkpoint_and_runtime_residence_untouched() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/stylesheet-body-boundary").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
const link = document.createElement("link");
link.rel = "stylesheet";
link.href = "data:text/css,body%7Bcolor%3Argb(1%2C%202%2C%203)%7D";
document.head.append(link);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;

        queue_stylesheet_checkpoint_marker(&mut page_vm, "__stylesheetBodyCheckpoint")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        let task = page_vm
            .take_stylesheet_networking_body_task_for_test()
            .expect("one exact stylesheet Networking body should be ready");
        let outcome = page_vm.apply_selected_page_stylesheet_networking_turn(task)?;
        assert_eq!(
            outcome.action.target_effect,
            PageStylesheetNetworkingTargetEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            stylesheet_checkpoint_marker(&mut page_vm, "__stylesheetBodyCheckpoint")?,
            "0",
            "the stylesheet terminal body must leave task-end checkpoint authority to the selected dispatcher",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "the body must not consume unrelated main-runtime residence",
        );
        assert!(
            page_vm.has_ready_dom_manipulation_task_for_test(),
            "the body may publish the later link event without executing it",
        );
        assert!(matches!(
            outcome.action.into_page_task_completion(),
            PageTaskCompletion::CheckpointOnly
        ));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stylesheet Networking body boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn selected_stylesheet_networking_terminal_checkpoints_without_dispatching_later_event() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/selected-stylesheet-completion").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__selectedStylesheetEvents = [];
const link = document.createElement("link");
link.rel = "stylesheet";
link.href = "data:text/css,body%7Bcolor%3Agreen%7D";
link.addEventListener("load", () => __selectedStylesheetEvents.push("load"));
document.head.append(link);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;

        queue_stylesheet_checkpoint_marker(&mut page_vm, "__selectedStylesheetCheckpoint")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "one exact StylesheetCompletion task must enter the production selected dispatcher",
        );
        assert_eq!(
            stylesheet_checkpoint_marker(&mut page_vm, "__selectedStylesheetCheckpoint")?,
            "1",
            "the current selected terminal must submit its ordinary task-end checkpoint",
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval_without_microtask_checkpoint_for_test(
                    "__selectedStylesheetEvents.join('|')",
                )?,
            "",
            "stylesheet fetch completion must not inline the later element event task",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts
                .pending_source_load_count_for_test(),
            1,
            "CheckpointOnly must not borrow callback runtime-follow-up authority",
        );
        assert!(
            page_vm.has_ready_dom_manipulation_task_for_test(),
            "the later link load event should remain scheduler-visible",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("selected stylesheet Networking completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_claimed_stylesheet_terminal_does_not_checkpoint_replacement_document() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/stale-stylesheet-completion").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner");
        page_vm.vm_mut().eval(
            r#"
const retired = document.createElement("link");
retired.rel = "stylesheet";
retired.href = "data:text/css,body%7Bcolor%3Ared%7D";
retired.addEventListener("load", () => {
  throw new Error("retired stylesheet event must not reach the replacement Document");
});
document.head.append(retired);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        let claimed = page_vm
            .claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::StylesheetCompletion,
            )
            .expect("retired Document should retain one opaque stylesheet terminal claim");

        page_vm.vm_mut().eval(
            "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        )?;
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner");
        assert_ne!(retired_owner, current_owner);
        queue_stylesheet_checkpoint_marker(&mut page_vm, "__staleStylesheetCheckpoint")?;
        page_vm.vm_mut().enqueue_test_pending_runtime_source_load();

        page_vm
            .run_claimed_selected_page_task_for_test(claimed, &loader)
            .await?;
        assert_eq!(
            stylesheet_checkpoint_marker(&mut page_vm, "__staleStylesheetCheckpoint")?,
            "0",
            "a retired stylesheet terminal must not checkpoint the replacement agent",
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .runtime_script_work()
                .dynamic_scripts.pending_source_load_count_for_test(),
            1,
            "discarding a stale terminal must not consume replacement runtime residence",
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "a retired completion must not publish an element event for the replacement Document",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale stylesheet Networking completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn connected_style_body_settles_its_lease_but_leaves_reactions_for_task_completion() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/connected-style-body-boundary").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__connectedStyleBoundary = [];
const style = document.createElement("style");
style.textContent = "body { color: green; }";
style.addEventListener("load", () => {
  __connectedStyleBoundary.push("callback");
  Promise.resolve().then(() => {
    __connectedStyleBoundary.push("microtask");
    const script = document.createElement("script");
    script.textContent = "__connectedStyleBoundary.push('runtime-script')";
    document.body.appendChild(script);
  });
});
document.head.appendChild(style);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        let task = take_next_style_element_event_task_for_test(&mut page_vm)
            .expect("one exact connected-style event task should be ready");
        let body = page_vm.apply_selected_page_connected_style_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageConnectedStyleEventTargetEffect::DispatchedToCurrentOwner {
                load_delay_effect: PageConnectedStyleLoadDelayEffect::ReleasedExactBinding,
            },
            "the selected body must dispatch and release its exact load-delay token"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__connectedStyleBoundary.join('|')")?,
            "callback",
            "the body must leave listener Promise reactions pending"
        );

        page_vm
            .finish_selected_page_networking_task(
                PageNetworkingTurnAction::StyleElementEvent(body.action),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__connectedStyleBoundary.join('|')")?,
            "callback|microtask|runtime-script",
            "selected completion must own the checkpoint and runtime-script follow-up"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("connected-style body/completion boundary test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn connected_style_completion_syncs_a_microtask_created_child_after_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/connected-style-microtask-child").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__connectedStyleChildOrder = [];
const style = document.createElement("style");
style.textContent = "body { color: purple; }";
style.addEventListener("load", () => {
  __connectedStyleChildOrder.push("callback");
  Promise.resolve().then(() => {
    __connectedStyleChildOrder.push("microtask");
    const frame = document.createElement("iframe");
    frame.id = "connected-style-microtask-child";
    frame.srcdoc = "<!doctype html><body>child</body>";
    document.body.appendChild(frame);
  });
});
document.head.appendChild(style);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StyleElementEvent,
                    &loader,
                )
                .await?,
            "one exact connected-style event must enter the production selected dispatcher"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__connectedStyleChildOrder.join('|')")?,
            "callback|microtask"
        );
        assert!(
            page_vm.vm().has_pending_child_navigation_commit_for_test(),
            "a reaction-created srcdoc frame must publish a typed navigation commit during connected-style completion"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit)
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("connected-style post-checkpoint child synchronization test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_inline_import_transitions_from_null_to_an_empty_child_stylesheet() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/missing.css",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        assert_eq!(
            page_vm.vm_mut().eval(&format!(
                r#"
(() => {{
  globalThis.__failedInlineImportEvents = [];
  const style = document.createElement('style');
  style.textContent = '@import url("{base_url}/missing.css");';
  style.addEventListener('load', () => __failedInlineImportEvents.push('load'));
  style.addEventListener('error', () => __failedInlineImportEvents.push('error'));
  document.head.append(style);
  globalThis.__failedInlineImportRule = style.sheet.cssRules[0];
  return String(__failedInlineImportRule.styleSheet);
}})()
"#,
            ))?,
            "null",
            "Chromium exposes no child stylesheet while the import request is pending",
        );
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the failed inline import must complete as Networking work",
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
[
  Object.prototype.toString.call(__failedInlineImportRule.styleSheet),
  __failedInlineImportRule.styleSheet?.cssRules.length ?? -1,
  __failedInlineImportRule.styleSheet?.ownerRule === __failedInlineImportRule,
  __failedInlineImportEvents.join(',')
].join('|')
"#,
            )?,
            "[object CSSStyleSheet]|0|true|",
            "a failed terminal must bind one empty child to the existing import rule wrapper",
        );

        let task = take_next_style_element_event_task_for_test(&mut page_vm)
            .expect("the failed inline graph must publish one style event");
        let body = page_vm.apply_selected_page_connected_style_event_turn(task)?;
        page_vm
            .finish_selected_page_networking_task(
                PageNetworkingTurnAction::StyleElementEvent(body.action),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__failedInlineImportEvents.join('|')")?,
            "error",
        );
        server.await.expect("failed inline import fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("failed inline import terminal test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn linked_import_graph_installs_native_children_before_the_link_load_event() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/root.css",
                "HTTP/1.1 200 OK",
                "@import './nested/child.css'; @import './nested/child.css'; .root { color: rgb(1, 2, 3); }"
                    .to_owned(),
                Duration::ZERO,
            ),
            (
                "/nested/child.css",
                "HTTP/1.1 200 OK",
                "@import './leaf.css'; .child { color: rgb(4, 5, 6); }".to_owned(),
                Duration::ZERO,
            ),
            (
                "/nested/leaf.css",
                "HTTP/1.1 200 OK",
                ".leaf { color: rgb(7, 8, 9); }".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__nativeImportEvents = [];
for (const className of ['root', 'child', 'leaf']) {{
  const node = document.createElement('div');
  node.className = className;
  document.body.append(node);
}}
const link = document.createElement('link');
link.id = 'native-import-root';
link.rel = 'stylesheet';
link.href = '{base_url}/root.css';
link.addEventListener('load', () => __nativeImportEvents.push('load'));
link.addEventListener('error', () => __nativeImportEvents.push('error'));
document.head.append(link);
"queued"
"#,
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the root stylesheet response must complete as Networking work"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "String(document.querySelector('#native-import-root').sheet.cssRules[0].styleSheet)"
            )?,
            "null",
            "CSSImportRule.styleSheet stays null until its native child response is installed"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__nativeImportEvents.join('|')")?,
            "",
            "the link event must wait for the complete nested import graph"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the native import graph must complete as one exact Networking turn"
        );
        let result = page_vm.vm_mut().eval(
            r#"
(() => {
  const root = document.querySelector('#native-import-root').sheet;
  const first = root.cssRules[0].styleSheet;
  const second = root.cssRules[1].styleSheet;
  const firstLeaf = first?.cssRules[0]?.styleSheet ?? null;
  const secondLeaf = second?.cssRules[0]?.styleSheet ?? null;
  return [
    first !== null,
    second !== null,
    first !== second,
    first?.ownerRule === root.cssRules[0],
    first?.parentStyleSheet === root,
    firstLeaf !== null,
    secondLeaf !== null,
    firstLeaf !== secondLeaf,
    first?.cssRules.length ?? -1,
    second?.cssRules.length ?? -1,
    getComputedStyle(document.querySelector('.root')).color,
    getComputedStyle(document.querySelector('.child')).color,
    getComputedStyle(document.querySelector('.leaf')).color,
    __nativeImportEvents.join(',')
  ].join('|');
})()
"#,
        )?;
        assert_eq!(
            result,
            "true|true|true|true|true|true|true|true|2|2|rgb(1, 2, 3)|rgb(4, 5, 6)|rgb(7, 8, 9)|"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the completed native graph must publish one link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__nativeImportEvents.join('|')")?,
            "load"
        );
        server.await.expect("stylesheet fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("native linked import graph test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_import_child_applies_but_denies_cssom_rule_access() {
    run_page_vm_async_test(async move {
        let (child_base_url, child_server) = spawn_path_response_http_server(vec![(
            "/child.css",
            "HTTP/1.1 200 OK",
            ".cross-origin-child { color: rgb(4, 5, 6); }".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let root_css = format!(
            "@import url('{child_base_url}/child.css'); .same-origin-root {{ color: rgb(1, 2, 3); }}"
        );
        let (root_base_url, root_server) = spawn_path_response_http_server(vec![(
            "/root.css",
            "HTTP/1.1 200 OK",
            root_css,
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{root_base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__crossOriginImportEvents = [];
for (const className of ['same-origin-root', 'cross-origin-child']) {{
  const node = document.createElement('div');
  node.className = className;
  document.body.append(node);
}}
const link = document.createElement('link');
link.id = 'cross-origin-import-root';
link.rel = 'stylesheet';
link.href = '{root_base_url}/root.css';
link.addEventListener('load', () => __crossOriginImportEvents.push('load'));
link.addEventListener('error', () => __crossOriginImportEvents.push('error'));
document.head.append(link);
"queued"
"#,
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the same-origin root stylesheet must complete first"
        );
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the cross-origin import graph must complete separately"
        );

        let result = page_vm.vm_mut().eval(
            r#"
(() => {
  const root = document.querySelector('#cross-origin-import-root').sheet;
  const importRule = root.cssRules[0];
  const child = importRule.styleSheet;
  let childRules;
  try {
    void child.cssRules;
    childRules = 'accessible';
  } catch (error) {
    childRules = `${error.name}:${error instanceof DOMException}`;
  }
  return [
    root.cssRules.length,
    child !== null,
    child?.ownerRule === importRule,
    child?.parentStyleSheet === root,
    childRules,
    getComputedStyle(document.querySelector('.same-origin-root')).color,
    getComputedStyle(document.querySelector('.cross-origin-child')).color,
    __crossOriginImportEvents.join(',')
  ].join('|');
})()
"#,
        )?;
        assert_eq!(
            result,
            "2|true|true|true|SecurityError:true|rgb(1, 2, 3)|rgb(4, 5, 6)|"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the cross-origin graph must publish one link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__crossOriginImportEvents.join('|')")?,
            "load"
        );
        root_server.await.expect("root stylesheet fixture server");
        child_server
            .await
            .expect("cross-origin stylesheet fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("cross-origin native import graph test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn failed_import_installs_an_empty_child_and_errors_the_root_link() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/root.css",
                "HTTP/1.1 200 OK",
                "@import './missing.css'; .root-survives { color: rgb(1, 2, 3); }".to_owned(),
                Duration::ZERO,
            ),
            (
                "/missing.css",
                "HTTP/1.1 404 Not Found",
                "not found".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__failedImportEvents = [];
const target = document.createElement('div');
target.className = 'root-survives';
document.body.append(target);
const link = document.createElement('link');
link.id = 'failed-import-root';
link.rel = 'stylesheet';
link.href = '{base_url}/root.css';
link.addEventListener('load', () => __failedImportEvents.push('load'));
link.addEventListener('error', () => __failedImportEvents.push('error'));
document.head.append(link);
"queued"
"#,
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?
        );
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?
        );

        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
(() => {
  const root = document.querySelector('#failed-import-root').sheet;
  const child = root.cssRules[0].styleSheet;
  return [
    child !== null,
    child?.cssRules.length ?? -1,
    child?.ownerRule === root.cssRules[0],
    getComputedStyle(document.querySelector('.root-survives')).color,
    __failedImportEvents.join(',')
  ].join('|');
})()
"#,
            )?,
            "true|0|true|rgb(1, 2, 3)|"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("the failed graph must publish one link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(event),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__failedImportEvents.join('|')")?,
            "error"
        );
        server.await.expect("failed import fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("failed native import graph test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn deleted_dynamic_import_rejects_its_late_completion_without_reopening_link_events() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/root.css",
                "HTTP/1.1 200 OK",
                ".root { color: green; }".to_owned(),
                Duration::ZERO,
            ),
            (
                "/slow.css",
                "HTTP/1.1 200 OK",
                concat!(
                    "@font-face { font-family: StaleImport; ",
                    "src: url('/stale-import.woff2'); } ",
                    ".late { color: red; }",
                )
                .to_owned(),
                Duration::from_millis(75),
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let document_url = Url::parse(&format!("{base_url}/page.html"))?;
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(&format!(
            r#"
globalThis.__dynamicImportEvents = [];
const link = document.createElement('link');
link.id = 'dynamic-import-root';
link.rel = 'stylesheet';
link.href = '{base_url}/root.css';
link.addEventListener('load', () => __dynamicImportEvents.push('load'));
link.addEventListener('error', () => __dynamicImportEvents.push('error'));
document.head.append(link);
"queued"
"#,
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?
        );
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let initial_event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("initial link load event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    initial_event,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__dynamicImportEvents.join('|')")?,
            "load"
        );

        page_vm.vm_mut().eval(&format!(
            r#"
(() => {{
  const sheet = document.querySelector('#dynamic-import-root').sheet;
  sheet.insertRule('@import url("{base_url}/slow.css");', 0);
  if (sheet.cssRules[0].styleSheet !== null) throw new Error('pending child must be null');
  sheet.deleteRule(0);
  return sheet.cssRules.length;
}})()
"#,
        ))?;
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the deleted edge's physical terminal remains observable Networking work"
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                "[document.querySelector('#dynamic-import-root').sheet.cssRules.length, __dynamicImportEvents.join(',')].join('|')"
            )?,
            "1|load",
            "a stale dynamic completion must neither restore the rule nor reopen link events"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "CSSOM import loads do not publish a second link load/error event"
        );
        assert_eq!(
            page_vm.pending_subresource_request_count(),
            0,
            "a stale import response must not schedule dependent resources"
        );
        server.await.expect("dynamic import fixture server");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dynamic import stale completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn connected_style_body_leaves_a_replaced_document_lease_ledger_untouched() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/connected-style-sync-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");
        page_vm.vm_mut().eval(
            r#"
const style = document.createElement("style");
style.textContent = "body { color: maroon; }";
style.addEventListener("load", () => {
  document.open();
  document.write("<!doctype html><main id='sync-style-replacement'>replacement</main>");
  document.close();
});
document.head.appendChild(style);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        let task = take_next_style_element_event_task_for_test(&mut page_vm)
            .expect("one exact connected-style event task should be ready");
        let body = page_vm.apply_selected_page_connected_style_event_turn(task)?;
        assert_eq!(
            body.action.target_effect,
            PageConnectedStyleEventTargetEffect::DispatchedToCurrentOwner {
                load_delay_effect:
                    PageConnectedStyleLoadDelayEffect::ExactBindingRetiredWithDocument,
            },
            "synchronous replacement must retire the old lease instead of retargeting it"
        );
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(retired_owner, current_owner);

        page_vm
            .finish_selected_page_networking_task(
                PageNetworkingTurnAction::StyleElementEvent(body.action),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "document.querySelector('#sync-style-replacement')?.textContent ?? 'missing'"
            )?,
            "replacement"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("connected-style synchronous replacement ownership test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn style_element_and_toggle_complete_in_distinct_normative_task_sources() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/connected-style-shared-dom-source").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__connectedStyleSharedOrder = [];
const style = document.createElement("style");
style.textContent = "body { color: navy; }";
style.addEventListener("load", () => {
  __connectedStyleSharedOrder.push("style");
  Promise.resolve().then(() => {
    __connectedStyleSharedOrder.push("microtask:style");
  });
});
document.head.appendChild(style);
"style-queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        page_vm.vm_mut().eval(
            r#"
const details = document.createElement("details");
details.addEventListener("toggle", () => {
  __connectedStyleSharedOrder.push("toggle");
  Promise.resolve().then(() => {
    __connectedStyleSharedOrder.push("microtask:toggle");
  });
});
document.body.appendChild(details);
details.open = true;
"toggle-queued"
"#,
        )?;

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StyleElementEvent,
                    &loader,
                )
                .await?,
            "<style> should consume its Networking turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__connectedStyleSharedOrder.join('|')")?,
            "style|microtask:style"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(
                        PageDomManipulationTestFamily::ElementToggle
                    ),
                    &loader,
                )
                .await?,
            "element toggle should consume its DOM-manipulation turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__connectedStyleSharedOrder.join('|')")?,
            "style|microtask:style|toggle|microtask:toggle"
        );
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "the toggle must remain the only DOM-manipulation task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("connected-style shared DOM source test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn connected_style_event_discards_a_document_open_task_before_replacement_tail() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/connected-style-document-open").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        page_vm.vm_mut().eval(
            r#"
globalThis.__documentExactStyleEvents = [];
const retiredStyle = document.createElement("style");
retiredStyle.textContent = "body { color: red; }";
retiredStyle.addEventListener("load", () => {
  __documentExactStyleEvents.push("retired");
});
document.head.appendChild(retiredStyle);
"retired-queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
document.open();
document.write("<!doctype html><html><head></head><body>replacement</body></html>");
document.close();
const currentStyle = document.createElement("style");
currentStyle.textContent = "body { color: green; }";
currentStyle.addEventListener("load", () => {
  __documentExactStyleEvents.push("current");
});
document.head.appendChild(currentStyle);
"replacement-queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(retired_owner, current_owner);

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StyleElementEvent,
                    &loader,
                )
                .await?,
            "retired connected-style event must remain a concrete stale turn"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentExactStyleEvents.join('|')")?,
            "",
            "the retired exact task must not enter the replacement Window"
        );

        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StyleElementEvent,
                    &loader,
                )
                .await?,
            "replacement connected-style event must survive the stale head"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__documentExactStyleEvents.join('|')")?,
            "current"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StyleElementEvent,
                )
                .is_none(),
            "both exact-Document tasks must consume exactly two turns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("connected-style document.open replacement test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stylesheet_tasks_keep_the_document_owner_captured_before_document_open() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/stylesheet-document-open").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main Document owner should exist");

        page_vm.vm_mut().eval(
            r#"
globalThis.__exactStyleEvents = [];
const retired = document.createElement("link");
retired.id = "retired-style";
retired.rel = "stylesheet";
retired.href = "data:text/css,body%7Bcolor%3Ared%7D";
retired.addEventListener("load", () => __exactStyleEvents.push("retired"));
document.head.append(retired);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        assert!(page_vm.vm().document_runtime.has_pending_style_loads());
        page_vm.vm_mut().eval(
            r#"
document.open();
document.write("<!doctype html><html><head></head><body>replacement</body></html>");
document.close();
"replaced"
"#,
        )?;
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main Document owner should exist");
        assert_ne!(retired_owner, current_owner);

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "retired stylesheet completion should consume one exact Networking turn",
        );
        assert_eq!(page_vm.vm_mut().eval("__exactStyleEvents.join('|')")?, "");
        assert!(
            !page_vm.has_ready_dom_manipulation_task_for_test(),
            "a stale completion must not publish an event for the replacement Document"
        );

        page_vm.vm_mut().eval(
            r#"
const current = document.createElement("link");
current.rel = "stylesheet";
current.href = "data:text/css,body%7Bcolor%3Agreen%7D";
current.addEventListener("load", () => __exactStyleEvents.push("current"));
document.head.append(current);
"queued"
"#,
        )?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "replacement stylesheet completion should consume one exact Networking turn",
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("replacement stylesheet event should consume one DOM-manipulation turn");
        assert_eq!(event.owner().document_owner(), current_owner);
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(event),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__exactStyleEvents.join('|')")?,
            "current"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("exact-Document stylesheet task test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn last_blocking_link_event_dispatches_before_parser_continuation() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/stylesheet-parser-continuation").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main Document owner");
        page_vm
            .vm_mut()
            .document_runtime
            .activate_main_parser_continuation(owner);

        page_vm.vm_mut().eval(
            r#"
globalThis.__blockingLinkEventSeen = false;
const blocking = document.createElement("link");
blocking.id = "blocking";
blocking.rel = "stylesheet";
blocking.href = "data:text/css,body%7Bcolor%3Argb(1%2C%202%2C%203)%7D";
blocking.addEventListener("load", () => {
  globalThis.__blockingLinkEventSeen = true;
});
document.head.append(blocking);
"queued"
"#,
        )?;
        // This low-level queue test starts after PageVm construction. Seed the
        // exact operation that the real HTML parser captures before consuming
        // the link's transient parser-processing state.
        let blocking_inputs = [parser_captured_stylesheet_input_for_test(
            &page_vm, "blocking",
        )];
        note_parser_captured_stylesheet_inputs_for_test(&mut page_vm, &blocking_inputs);
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "the exact Networking head must apply the stylesheet completion",
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .has_all_blocking_stylesheets_resolved(),
            "blocking state must be cleared before parser notification"
        );
        assert!(
            page_vm.has_ready_page_networking_task(),
            "stylesheet settlement should publish Blink-style parser reevaluation"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::DomManipulationTask)
            .await;
        let event = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("blocking link load event DOM-manipulation turn");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(event),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__blockingLinkEventSeen")?,
            "true",
            "the observable link event must finish before parser admission"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainParserContinuation,
                    &loader,
                )
                .await?,
            "stylesheet release must queue an exact main-parser continuation"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stylesheet parser continuation ordering test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn shared_blocking_fetch_waits_for_every_link_event_before_parser_continuation() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.com/shared-stylesheet-parser-continuation").unwrap();
        let (mut page_vm, _resource_source, mut wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main Document owner");
        page_vm
            .vm_mut()
            .document_runtime
            .activate_main_parser_continuation(owner);

        page_vm.vm_mut().eval(
            r#"
globalThis.__blockingLinkEvents = [];
for (const id of ["first", "second"]) {
  const blocking = document.createElement("link");
  blocking.id = id;
  blocking.rel = "stylesheet";
  blocking.href = "data:text/css,body%7Bcolor%3Argb(4%2C%205%2C%206)%7D";
  blocking.addEventListener("load", () => {
    globalThis.__blockingLinkEvents.push(id);
  });
  document.head.append(blocking);
}
"queued"
"#,
        )?;
        let blocking_inputs = [
            parser_captured_stylesheet_input_for_test(&page_vm, "first"),
            parser_captured_stylesheet_input_for_test(&page_vm, "second"),
        ];
        note_parser_captured_stylesheet_inputs_for_test(
            &mut page_vm,
            &blocking_inputs,
        );
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::StylesheetCompletion,
                    &loader,
                )
                .await?,
            "shared stylesheet completion must enter the production selected dispatcher"
        );
        assert!(
            page_vm.has_ready_page_networking_task(),
            "stylesheet settlement should publish one coalesced parser reevaluation"
        );
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainParserContinuation,
                    &loader,
                )
                .await?,
            "shared stylesheet completion must queue an early parser reevaluation turn"
        );

        wait_for_stylesheet_source(
            &mut wake_rx,
            RendererOwnerWakeSource::DomManipulationTask,
        )
        .await;
        let first = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("first blocking link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(first),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__blockingLinkEvents.length")?,
            "1"
        );
        assert!(
            !page_vm.has_ready_page_networking_task(),
            "the first client of a shared fetch must not release the parser while another blocking link event remains posted"
        );

        let second = take_next_link_element_event_task_for_test(&mut page_vm)
            .expect("second blocking link event");
        page_vm
            .run_claimed_dom_manipulation_task_through_selected_dispatcher_for_test(
                crate::page_task_queue::RendererPageDomManipulationTask::ConnectedStyleEvent(
                    second,
                ),
                &loader,
            )
            .await?;
        assert_eq!(
            page_vm.vm_mut().eval("__blockingLinkEvents.join('|')")?,
            "first|second"
        );

        wait_for_stylesheet_source(&mut wake_rx, RendererOwnerWakeSource::NetworkingTask).await;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::MainParserContinuation,
                    &loader,
                )
                .await?,
            "the last blocking link event must queue the parser continuation"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("shared stylesheet parser continuation ordering test should run");
}
