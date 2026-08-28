use super::*;

use crate::page_task_queue::{
    PageChildClassicScriptSourceLoadTargetEffect, PageChildParserModuleRootStartTargetEffect,
    PageChildRealmMaterializationTargetEffect, RendererOwnerWakeSource,
};

fn request_new_child_realm(page_vm: &mut PageVm, element_id: &str, expose_twice: bool) {
    page_vm
        .vm_mut()
        .eval(&format!(
            r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.id = {element_id:?};
  document.body.appendChild(frame);
  void frame.contentWindow.Function;
  if ({expose_twice}) void frame.contentWindow.Object;
  return "queued";
}})()
"#
        ))
        .expect("child Window exposure should enqueue one exact-Document realm task");
}

fn request_existing_child_realm(page_vm: &mut PageVm, element_id: &str) {
    page_vm
        .vm_mut()
        .eval(&format!(
            "void document.getElementById({element_id:?}).contentWindow.Function; 'queued'"
        ))
        .expect("replacement child Window exposure should enqueue its exact-Document realm task");
}

#[tokio::test(flavor = "current_thread")]
async fn child_realm_source_deduplicates_reentrant_exposure_and_consumes_one_child_per_turn() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-realm-one-turn").unwrap();
        let (mut page_vm, _resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        request_new_child_realm(&mut page_vm, "first-realm-child", true);
        request_new_child_realm(&mut page_vm, "second-realm-child", false);
        let first_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("first-realm-child")
            .expect("first child handle");
        let second_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("second-realm-child")
            .expect("second child handle");

        let enqueue_wakes = std::iter::from_fn(|| owner_wake_rx.try_recv().ok())
            .collect::<Vec<_>>();
        assert_eq!(
            enqueue_wakes
                .iter()
                .filter(|wake| {
                    wake.source_for_test() == RendererOwnerWakeSource::ChildFrameTask
                })
                .count(),
            1,
            "one empty-to-nonempty edge must admit the ready source; repeated exposure and a second queued child must coalesce"
        );

        let first = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("first exact child should consume one Page turn");
        assert_eq!(first.action.owner.target().child_handle(), Some(first_handle));
        assert_eq!(
            first.action.target_effect,
            PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
        );

        assert!(
            std::iter::from_fn(|| owner_wake_rx.try_recv().ok()).all(|wake| {
                wake.source_for_test() != RendererOwnerWakeSource::ChildFrameTask
            }),
            "successful application must not publish a phantom typed-source wake"
        );

        let second = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("second exact child should consume the next Page turn");
        assert_eq!(
            second.action.owner.target().child_handle(),
            Some(second_handle)
        );
        assert_eq!(
            second.action.target_effect,
            PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
        );
        assert!(
            page_vm
                .run_child_realm_materialization_body_for_test()?
                .is_none(),
            "reentrant exposure of the first child must not leave a duplicate durable task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child realm one-turn contract should hold");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_realm_inventory_observes_without_materializing_pending_child_realm() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-realm-observer").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        request_new_child_realm(&mut page_vm, "observed-realm-child", false);
        assert!(
            page_vm
                .vm_mut()
                .live_child_default_runtime_realm_inventory()
                .is_empty(),
            "Runtime inventory must not create a child realm before its Page turn"
        );
        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "observer reads must retain the durable exact-Document request"
        );

        let outcome = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("the pending typed task should remain executable after observation");
        assert_eq!(
            outcome.action.target_effect,
            PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .live_child_default_runtime_realm_inventory()
                .len(),
            1,
            "the authorized Page turn, not the observer, must publish the realm"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Runtime realm inventory should remain observer-only");
}

#[tokio::test(flavor = "current_thread")]
async fn prebootstrapped_child_inline_script_runs_before_typed_realm_publication() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/prebootstrapped-child-script").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__prebootstrappedChildScript = "pending";
  const frame = document.createElement("iframe");
  frame.id = "prebootstrapped-script-child";
  document.body.appendChild(frame);
  void frame.contentWindow.Function;
  const script = frame.contentDocument.createElement("script");
  script.textContent = `
    parent.__prebootstrappedChildScript =
      "ran:" + (globalThis === self) + ":" + document.currentScript.tagName.toLowerCase();
  `;
  frame.contentDocument.body.appendChild(script);
})()
"#,
        )?;

        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "synchronous prebootstrap must still leave one typed exact-Document realm publication"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                )
                .is_none(),
            "the realm prerequisite must remain the shared child-frame FIFO head"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__prebootstrappedChildScript")?,
            "ran:true:script",
            "dynamic inline classic execution must complete before the DOM mutation returns"
        );

        let materialization = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("typed realm source should retain the exact child request");
        assert_eq!(
            materialization.action.target_effect,
            PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
        );
        assert_eq!(
            page_vm.vm_mut().eval("__prebootstrappedChildScript")?,
            "ran:true:script",
            "typed realm publication must retain the already executed child realm"
        );
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                )
                .is_none(),
            "typed admission must not duplicate the child script task"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("prebootstrapped child script should execute before typed realm publication");
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_root_stays_queued_until_exact_child_realm_is_materialized() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/realm-gated-root.js",
            "HTTP/1.1 200 OK",
            "export const value = 1;".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(&format!(
            r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.id = "realm-gated-module";
  frame.srcdoc = `<script type="module" src="{base_url}/realm-gated-root.js"><\/script>`;
  document.body.appendChild(frame);
}})()
"#,
        ))?;

        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "navigation must install the exact child Document before its realm prerequisite"
        );
        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "parser discovery must publish one typed exact-Document realm prerequisite"
        );
        assert_eq!(
            page_vm
                .run_child_parser_module_root_start_body_for_test()?,
            None,
            "the shared family FIFO must keep the parser-module root behind its realm prerequisite"
        );
        assert!(
            !server.is_finished(),
            "the blocked parser-module root must not start its network request"
        );

        let materialization = run_expected_pending_child_realm_materialization_turn(
            &mut page_vm,
            "parser-module realm prerequisite",
        )?;
        assert_eq!(
            materialization.action.target_effect,
            PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
        );

        let mut observed_sources = Vec::new();
        while !observed_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart) {
            let source = page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await
                .expect("materialized child source should remain runnable");
            observed_sources.push(source);
            assert!(
                observed_sources.len() <= 3,
                "parser-module root should become runnable after bounded lifecycle admission: {observed_sources:?}"
            );
        }
        assert!(
            observed_sources
                .iter()
                .take(observed_sources.len() - 1)
                .all(|source| *source == ChildFrameSemanticTurnKind::DocumentLifecycle),
            "only earlier document lifecycle work may precede the parser-module root: {observed_sources:?}"
        );

        server
            .await
            .expect("materialized parser-module root request server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("parser-module root must retain its work across realm materialization");
}

#[tokio::test(flavor = "current_thread")]
async fn classic_source_load_starts_before_exact_child_realm_materialization() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/realm-gated-classic.js",
            "HTTP/1.1 200 OK",
            "parent.__realmGatedClassic = 'ran';".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(&format!(
            r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.id = "realm-gated-classic";
  frame.srcdoc = `<script src="{base_url}/realm-gated-classic.js"><\/script>`;
  document.body.appendChild(frame);
}})()
"#,
        ))?;

        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "navigation must install the exact child Document before its realm prerequisite"
        );
        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "parser discovery must publish the later typed execution-realm task"
        );

        let start = page_vm
            .run_child_classic_source_load_body_for_test()?
            .expect("parser classic must retain one typed fetch-start task before realm setup");
        let crate::page_task_queue::RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(
            target,
        ) = start.action.owner.target()
        else {
            panic!("classic fetch-start turn must retain its typed target")
        };
        assert_eq!(
            start.action.target_effect,
            PageChildClassicScriptSourceLoadTargetEffect::NetworkRequestStartedForCurrentOwner
        );
        assert_eq!(
            page_vm.run_child_classic_source_load_body_for_test()?,
            None,
            "one typed task must start the parser classic request exactly once"
        );
        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "starting the fetch must not consume its later execution-realm task"
        );
        assert!(
            page_vm
                .vm_mut()
                .live_child_default_runtime_realm_inventory()
                .is_empty(),
            "network admission must not materialize a V8 context as a side effect"
        );

        let materialization = run_expected_pending_child_realm_materialization_turn(
            &mut page_vm,
            "parser-classic execution realm",
        )?;
        assert_eq!(
            materialization.action.owner.target().child_handle(),
            Some(target.child_handle())
        );
        assert_eq!(
            materialization.action.owner.target().document_owner(),
            target.document_owner()
        );

        server
            .await
            .expect("parser-classic request server should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("parser classic fetch start must precede its execution realm");
}

#[tokio::test(flavor = "current_thread")]
async fn classic_source_load_survives_stale_cross_origin_context_pruning() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/realm-prune-child.css",
                "HTTP/1.1 200 OK",
                "body { color: green; }".to_owned(),
                Duration::from_millis(100),
            ),
            (
                "/realm-prune-child.js",
                "HTTP/1.1 200 OK",
                "parent.__realmPruneClassicRan = true;".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("http://127.0.0.1:9/realm-prune-parent").expect("page URL");
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "realm-prune-child";
  document.body.appendChild(frame);
  void frame.contentWindow.Function;
})()
"#,
        )?;
        run_expected_pending_child_realm_materialization_turn(
            &mut page_vm,
            "initial same-origin about:blank realm",
        )?;
        page_vm.vm_mut().eval(&format!(
            r#"
(() => {{
  const markup = `<link rel="stylesheet" href="{base_url}/realm-prune-child.css"><script src="{base_url}/realm-prune-child.js"><\/script>`;
  const frame = document.getElementById("realm-prune-child");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.srcdoc = markup;
}})()
"#
        ))?;
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "cross-origin navigation must start from its explicit commit turn"
        );
        assert!(
            page_vm.has_ready_child_frame_semantic_turn_for_test(
                ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
            ),
            "replacement parser must publish its exact external classic source-start"
        );

        let _ = page_vm
            .vm_mut()
            .live_child_default_runtime_realm_inventory();
        let start = page_vm
            .run_child_classic_source_load_body_for_test()?
            .expect("realm pruning must leave the current parser source-start queued");
        assert_eq!(
            start.action.target_effect,
            PageChildClassicScriptSourceLoadTargetEffect::NetworkRequestStartedForCurrentOwner,
            "pruning the retired about:blank context must not retire the replacement Document's reserved realm"
        );

        server
            .await
            .expect("cross-origin child Document resources should finish");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale context pruning must preserve the current classic fetch start");
}

#[tokio::test(flavor = "current_thread")]
async fn classic_source_load_stale_discards_after_child_document_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.test/classic-source-child-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "stale-classic-source-child";
  frame.srcdoc = `<script src="/retired-parser-classic.js"><\/script>`;
  document.body.appendChild(frame);
})()
"#,
        )?;
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "the first navigation must install the parser classic's exact Document"
        );
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("stale-classic-source-child")
            .expect("classic source child should retain its iframe handle");
        let retired_owner = page_vm
            .vm()
            .current_child_document_task_owner(child_handle)
            .expect("classic source task must retain its exact child Document owner");

        page_vm.vm_mut().eval(
            "document.getElementById('stale-classic-source-child').srcdoc = \
             '<!doctype html><body>replacement</body>'; 'queued'",
        )?;
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "replacement navigation must retire the old exact Document before its fetch starts"
        );

        let stale = page_vm
            .run_child_classic_source_load_body_for_test()?
            .expect("the retired classic start must consume one stale-discard turn");
        assert_eq!(
            stale.action.owner.root_document(),
            page_vm.document_lifecycle.identity().document
        );
        let crate::page_task_queue::RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(
            stale_target,
        ) = stale.action.owner.target()
        else {
            panic!("stale classic turn must retain its typed target")
        };
        assert_eq!(stale_target.child_handle(), child_handle);
        assert_eq!(stale_target.document_owner(), retired_owner);
        assert_eq!(
            stale.action.target_effect,
            PageChildClassicScriptSourceLoadTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(
            page_vm.run_child_classic_source_load_body_for_test()?,
            None,
            "stale retirement must consume the old classic start exactly once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child replacement must prevent a retired parser classic from fetching");
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_root_stale_discards_after_child_document_replacement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url =
            Url::parse("https://example.test/parser-root-child-replacement").unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        page_vm.vm_mut().eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "stale-parser-root-child";
  frame.srcdoc = `<script type="module" src="/retired-parser-root.js"><\/script>`;
  document.body.appendChild(frame);
})()
"#,
        )?;
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "the first navigation must install the parser module's exact Document"
        );
        let materialization = run_expected_pending_child_realm_materialization_turn(
            &mut page_vm,
            "retired parser-module realm prerequisite",
        )?;
        let retired_owner = materialization.action.owner;
        let child_handle = retired_owner
            .target()
            .child_handle()
            .expect("parser root prerequisite must name its child");
        let retired_module_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("materialized retired Document must expose its exact module target");

        page_vm.vm_mut().eval(
            "document.getElementById('stale-parser-root-child').srcdoc = \
             '<!doctype html><body>replacement</body>'; 'queued'",
        )?;
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "replacement navigation must retire the old exact Document before its root starts"
        );

        let stale = page_vm
            .run_child_parser_module_root_start_body_for_test()?
            .expect("the retired parser root must consume one stale-discard turn");
        assert_eq!(
            stale.action.owner.root_document(),
            retired_owner.root_document()
        );
        let crate::page_task_queue::RendererPageChildFrameTaskTarget::ParserModuleRootStart(
            stale_target,
        ) = stale.action.owner.target()
        else {
            panic!("stale parser-root turn must retain its typed target")
        };
        assert_eq!(stale_target.child_handle(), child_handle);
        assert_eq!(
            stale_target.document_owner(),
            retired_module_target.task_owner()
        );
        assert_eq!(stale_target.realm_id(), retired_module_target.realm_id());
        assert_eq!(
            stale.action.target_effect,
            PageChildParserModuleRootStartTargetEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(
            page_vm.run_child_parser_module_root_start_body_for_test()?,
            None,
            "stale retirement must consume the old root exactly once"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child replacement must prevent a retired parser root from starting");
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_replacement_stale_drops_old_task_without_stealing_new_request() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/child-realm-document-replacement")
            .unwrap();
        let (mut page_vm, _resource_source, _owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);

        request_new_child_realm(&mut page_vm, "replaced-realm-child", false);
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("replaced-realm-child")
            .expect("replacement child handle");
        let root_document = page_vm.document_lifecycle.identity().document;

        page_vm.vm_mut().eval(
            "document.getElementById('replaced-realm-child').srcdoc = \
             '<!doctype html><body>replacement child</body>'; 'queued'",
        )?;
        loop {
            let outcome = page_vm
                .run_child_navigation_commit_body_for_test()?
                .expect("replacement must retain its exact navigation task");
            match outcome.action.target_effect {
                crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner => break,
                crate::page_task_queue::PageChildNavigationCommitTargetEffect::DiscardedStaleOwner {
                    ..
                } => {
                    // Preserve the old realm task deliberately: this test
                    // selects navigation ahead of that other ready source to
                    // exercise its stale-owner settlement after replacement.
                }
            }
        }
        request_existing_child_realm(&mut page_vm, "replaced-realm-child");
        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "the replacement producer must retain its exact request until its typed Page turn"
        );

        let stale = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("old child Document task should consume one stale-discard turn");
        let PageChildRealmMaterializationTargetEffect::IgnoredStaleOwner {
            current_owner: Some(current_owner),
        } = stale.action.target_effect
        else {
            panic!("old child Document task should expose the replacement exact owner")
        };
        assert_eq!(stale.action.owner.root_document(), root_document);
        assert_eq!(current_owner.root_document(), root_document);
        assert_eq!(
            stale.action.owner.target().child_handle(),
            Some(child_handle)
        );
        assert_eq!(current_owner.target().child_handle(), Some(child_handle));
        assert_ne!(
            stale.action.owner.target().document_owner(),
            current_owner.target().document_owner(),
            "child replacement must change the local Document owner while retaining the iframe handle"
        );

        assert!(
            page_vm.vm().has_pending_child_frame_realm_materialization(),
            "discarding the old exact Document must not clear the replacement request"
        );

        let current = page_vm
            .run_child_realm_materialization_body_for_test()?
            .expect("replacement child request should remain queued behind the stale task");
        assert_eq!(current.action.owner, current_owner);
        assert_eq!(
            current.action.target_effect,
            PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child Document replacement must preserve exact realm-task ownership");
}

#[test]
fn page_vm_replacement_rejects_naturally_colliding_child_realm_target() {
    run_page_vm_large_stack_async_test(
        "child-realm-real-page-vm-replacement-collision",
        || async move {
            let (base_url, server) = spawn_path_response_http_server(vec![(
                "/replacement.html",
                "HTTP/1.1 200 OK",
                "<!doctype html><html><head></head><body></body></html>".to_owned(),
                Duration::ZERO,
            )])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let document_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let (mut page_vm, _resource_source, _owner_wake_rx) =
                page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    request_new_child_realm(&mut page_vm, "colliding-realm-child", false);
                    let retired_root = page_vm.document_lifecycle.identity().document;

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
                    let current_root = page_vm.document_lifecycle.identity().document;
                    assert_ne!(retired_root, current_root);
                    request_new_child_realm(&mut page_vm, "colliding-realm-child", false);

                    let stale = page_vm
                        .run_child_realm_materialization_body_for_test()?
                        .expect("retired PageVm realm task should consume one stale turn");
                    let PageChildRealmMaterializationTargetEffect::IgnoredStaleOwner {
                        current_owner: Some(current_owner),
                    } = stale.action.target_effect
                    else {
                        panic!("retired root task should expose the colliding replacement owner")
                    };
                    assert_eq!(stale.action.owner.root_document(), retired_root);
                    assert_eq!(current_owner.root_document(), current_root);
                    assert_eq!(
                        stale.action.owner.target(),
                        current_owner.target(),
                        "fresh PageVm-local counters and identical DOM construction must naturally reproduce the entire local child target"
                    );


                    let current = page_vm
                        .run_child_realm_materialization_body_for_test()?
                        .expect("replacement PageVm request must survive the old root discard");
                    assert_eq!(current.action.owner, current_owner);
                    assert_eq!(
                        current.action.target_effect,
                        PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("PageVm replacement realm tasks should run through the exact owner arbiter");
            server
                .await
                .expect("PageVm replacement response server should finish");
        },
    );
}
