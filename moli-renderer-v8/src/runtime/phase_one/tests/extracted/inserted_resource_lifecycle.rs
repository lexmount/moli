use super::*;

#[test]
fn parser_connected_document_write_inline_script_obeys_csp() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let env = default_test_page_vm_env_config_with(|env| {
            env.document_policy_container
                .response_content_security_policies = vec!["script-src 'nonce-outer'".to_owned()];
        });
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test_with_env(
            r#"<!doctype html><html><head><script nonce="outer">
globalThis.__blockedDocumentWriteRan = false;
globalThis.__documentWriteViolations = 0;
document.addEventListener("securitypolicyviolation", () => {
  globalThis.__documentWriteViolations += 1;
});
document.write('<script>globalThis.__blockedDocumentWriteRan = true;<\/script>');
globalThis.__outerDocumentWriteScriptContinued = true;
</script></head><body></body></html>"#,
            env,
        )
        .await;

        let before_dispatch = page_vm
            .evaluate_expression(
                r#"JSON.stringify({
  blockedScriptRan: globalThis.__blockedDocumentWriteRan ?? "missing",
  outerScriptContinued: globalThis.__outerDocumentWriteScriptContinued ?? "missing",
  violations: globalThis.__documentWriteViolations ?? "missing"
})"#,
            )
            .expect("pre-dispatch document.write CSP state should evaluate");
        assert_eq!(
            before_dispatch
                .get("value")
                .and_then(serde_json::Value::as_str),
            Some(r#"{"blockedScriptRan":false,"outerScriptContinued":true,"violations":0}"#),
            "the inner script must be blocked before its violation task dispatches"
        );
        let local_executor = page_vm.local_executor.clone();
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let violation_task_count = super::access::run_named_owner_local_task(
            local_executor,
            "document.write CSP violation test task channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                page_vm.page_task_queue.accept_ready_parse_time_wakes();
                let mut violation_task_count = 0;
                while let Some(task) = page_vm.page_task_queue.parse_time_pop_front() {
                    if matches!(
                        &task,
                        crate::page_task_queue::PageTask::DispatchContentSecurityPolicyViolation(_)
                    ) {
                        violation_task_count += 1;
                    }
                    let work = PostParsePageOwnedWork::lifecycle_work(
                        crate::page_task_queue::PostParseLifecycleWork::from_parse_time_page_task(
                            task,
                        ),
                    );
                    execute_page_owned_work_turn_on_local_task(page_vm, work).await?;
                }
                Ok(violation_task_count)
            },
        )
        .await
        .expect("document.write parser-boundary tasks should dispatch");
        assert_eq!(
            violation_task_count, 1,
            "the blocked document.write script should queue exactly one violation task"
        );
        let after_dispatch = page_vm
            .evaluate_expression(
                r#"JSON.stringify({
  blockedScriptRan: globalThis.__blockedDocumentWriteRan,
  outerScriptContinued: globalThis.__outerDocumentWriteScriptContinued,
  violations: globalThis.__documentWriteViolations
})"#,
            )
            .expect("document.write CSP state should evaluate");
        assert_eq!(
            after_dispatch
                .get("value")
                .and_then(serde_json::Value::as_str),
            Some(r#"{"blockedScriptRan":false,"outerScriptContinued":true,"violations":1}"#)
        );
    }));
}
#[test]
fn csp_rejected_document_write_stylesheet_does_not_strand_parser_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let env = default_test_page_vm_env_config_with(|env| {
                env.document_policy_container
                    .response_content_security_policies =
                    vec!["style-src 'none'; script-src 'unsafe-inline'".to_owned()];
            });
            let mut page_vm = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                parse_phase_one_html_into_page_vm_for_test_with_env(
                    r#"<!doctype html><html><head><script>
document.write('<link rel="stylesheet" href="https://example.test/blocked.css">');
globalThis.__outerContinued = true;
</script></head><body><p id="parser-tail">tail</p></body></html>"#,
                    env,
                ),
            )
            .await
            .expect("a synchronously rejected stylesheet must not strand its document.write owner");

            let result = page_vm
                .evaluate_expression(
                    "JSON.stringify({outerContinued: __outerContinued, tail: !!document.getElementById('parser-tail')})",
                )
                .expect("completed parser state should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"outerContinued":true,"tail":true}"#),
            );
        }));
}
#[test]
fn parser_driver_finish_parser_blocking_pause_scans_document_write_preloads() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
            bind_preload_state_to_current_test_runtime(&mut state.buffered_document_preloads);
            let session = state.parser_session.stream_handle().borrow().script_input_session();
            session.enqueue_script_input_preload_html("<script sr".to_owned());
            session.enqueue_script_input_preload_html("c=\"/write.js\"></script>".to_owned());

            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            driver.finish_parser_blocking_pause();

            assert!(
                driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/write.js")),
                "document.write insertions during a parser pause should feed the insertion preload scanner"
            );
            assert!(
                driver.buffered_document_preloads.insertion_scanner.is_none(),
                "Chromium resets the insertion preload scanner after resuming from a parser-blocking pause"
            );
            assert!(
                driver.parser_session.stream_handle().borrow_mut().take_next_insertion_preload_input().is_none(),
                "queued insertion preload html should be fully drained when the pause completes"
            );
        });
}
#[test]
fn phase_one_parser_inserted_connected_stylesheet_queues_load_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"<!doctype html><html><head>
<link rel="stylesheet" href="data:text/css,body%20%7B%20color%3A%20green%3B%20%7D">
</head><body></body></html>"#;
            let page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let head = snapshot.document_head_handle().expect("head handle");
            let link = snapshot
                .child_nodes(head)
                .expect("head children")
                .into_iter()
                .find(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("link"))
                })
                .expect("parser-created stylesheet link");
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "parser insertion should queue connected style/link processing from the runtime mutation owner"
            );
        }));
}
#[test]
fn js_document_fragment_insertion_queues_connected_stylesheet_loads_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        create_connected_html_body_for_test(&mut page_vm);

        page_vm
            .evaluate_expression(
                r#"
function fragmentWithStylesheet(id) {
  const fragment = document.createDocumentFragment();
  const link = document.createElement('link');
  link.id = id;
  link.rel = 'stylesheet';
  link.href = 'data:text/css,body%20%7B%20color%3A%20green%3B%20%7D';
  fragment.appendChild(link);
  return fragment;
}

document.body.appendChild(fragmentWithStylesheet('js-fragment-style-append'));

const reference = document.createElement('span');
reference.id = 'js-fragment-style-reference';
document.body.appendChild(reference);
document.body.insertBefore(
  fragmentWithStylesheet('js-fragment-style-before'),
  reference
);

const oldChild = document.createElement('span');
oldChild.id = 'js-fragment-style-old';
document.body.appendChild(oldChild);
document.body.replaceChild(
  fragmentWithStylesheet('js-fragment-style-replace'),
  oldChild
);
"#,
            )
            .expect("fragment stylesheet insertion JS setup should evaluate");

        let expected = {
            let runtime = &page_vm.vm().document_runtime;
            vec![
                runtime
                    .get_element_by_id("js-fragment-style-append")
                    .expect("append fragment stylesheet should exist"),
                runtime
                    .get_element_by_id("js-fragment-style-before")
                    .expect("insertBefore fragment stylesheet should exist"),
                runtime
                    .get_element_by_id("js-fragment-style-replace")
                    .expect("replaceChild fragment stylesheet should exist"),
            ]
        };

        let runtime = &page_vm.vm().document_runtime;
        assert!(
            expected
                .into_iter()
                .all(|handle| runtime.connected_style_load_is_queued_for_test(handle)),
            "JS DocumentFragment insertion should queue stylesheet loads for hoisted children"
        );
    }));
}
#[test]
fn parser_fragment_append_child_queues_stylesheet_load_from_hoisted_roots() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        let body = create_connected_html_body_for_test(&mut page_vm);

        let (fragment, link) = {
            let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
            let fragment = dom_host.create_document_fragment();
            let link = dom_host.create_parser_element_without_attributes(
                "link".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
            assert!(dom_host.set_attribute(link, "id", "parser-fragment-style-append-link"));
            assert!(dom_host.set_attribute(link, "rel", "stylesheet"));
            assert!(dom_host.set_attribute(
                link,
                "href",
                "data:text/css,body%20%7B%20color%3A%20green%3B%20%7D"
            ));
            assert!(dom_host.append_child(fragment, link));
            (fragment, link)
        };
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .connected_style_load_is_queued_for_test(link),
            "disconnected parser fragment stylesheet setup should not queue style loads"
        );

        let custom_element_reaction_roots = {
            apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: fragment,
                },
                "parser fragment stylesheet appendChild should apply",
            )
        };
        assert!(
            custom_element_reaction_roots.is_empty(),
            "plain stylesheet fragment append should not queue custom element reactions"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .dom_host()
                .child_handles(fragment)
                .count(),
            0,
            "parser DocumentFragment stylesheet append should hoist and empty the fragment"
        );
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .dom_host()
                .child_handles(body)
                .collect::<Vec<_>>(),
            vec![link],
            "parser DocumentFragment stylesheet append should append the hoisted link"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .connected_style_load_is_queued_for_test(link),
            "parser DocumentFragment append should queue stylesheet loads for hoisted children"
        );
    }));
}
#[test]
fn parser_fragment_insert_before_queues_stylesheet_load_from_hoisted_roots() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let (fragment, link, reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-style-ref"));
                assert!(dom_host.append_child(body, reference));

                let fragment = dom_host.create_document_fragment();
                let link = dom_host.create_parser_element_without_attributes(
                    "link".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(link, "id", "parser-fragment-style-link"));
                assert!(dom_host.set_attribute(link, "rel", "stylesheet"));
                assert!(dom_host.set_attribute(
                    link,
                    "href",
                    "data:text/css,body%20%7B%20color%3A%20green%3B%20%7D"
                ));
                assert!(dom_host.append_child(fragment, link));
                (fragment, link, reference)
            };
            assert!(
                !page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "disconnected parser fragment stylesheet setup should not queue style loads"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment stylesheet insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain stylesheet fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment stylesheet insertion should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![link, reference],
                "parser DocumentFragment stylesheet insertion should place the hoisted link before the reference child"
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "parser DocumentFragment insertion should queue stylesheet loads for hoisted children"
            );
        }));
}
#[test]
fn js_replace_child_inserted_image_queues_load_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        create_connected_html_body_for_test(&mut page_vm);
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("context host should be alive");

        page_vm
            .evaluate_expression(
                r#"
const oldImageSlot = document.createElement('span');
oldImageSlot.id = 'js-replace-old-image-slot';
document.body.appendChild(oldImageSlot);

const image = document.createElement('img');
image.id = 'js-replace-inserted-image';
image.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
window.jsReplaceInsertedImage = image;
"#,
            )
            .expect("replaceChild inserted image setup should evaluate");

        page_vm
            .evaluate_expression(
                r#"
document.body.replaceChild(
  window.jsReplaceInsertedImage,
  document.getElementById('js-replace-old-image-slot')
);
"#,
            )
            .expect("replaceChild inserted image should evaluate");

        let image = {
            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body handle");
            snapshot
                .child_nodes(body)
                .expect("body children")
                .into_iter()
                .find(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.attribute("id") == Some("js-replace-inserted-image")
                        })
                })
                .expect("connected replacement image should exist")
        };
        assert!(
            context_host
                .borrow()
                .has_pending_image_load_event_for_test(image),
            "JS replaceChild insertion should queue image load events from the mutation owner"
        );
    }));
}
#[test]
fn js_replace_child_inserted_default_track_queues_load_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
const video = document.createElement('video');
video.id = 'js-replace-track-video';
const oldTrackSlot = document.createElement('span');
oldTrackSlot.id = 'js-replace-old-track-slot';
video.appendChild(oldTrackSlot);
document.body.appendChild(video);

const track = document.createElement('track');
track.id = 'js-replace-inserted-track';
document.body.appendChild(track);
window.jsReplaceInsertedTrack = track;
track.remove();
"#,
                )
                .expect("replaceChild inserted track setup should evaluate");

            let track = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("js-replace-inserted-track")
                .or_else(|| {
                    let dom_host = page_vm.vm().document_runtime.dom_host();
                    dom_host.dom().nodes().iter().enumerate().find_map(|(index, node)| {
                        node.as_element()
                            .is_some_and(|element| {
                                element.attribute("id") == Some("js-replace-inserted-track")
                            })
                            .then_some(DomHandle::new(index))
                    })
                })
                .expect("detached replacement track handle should exist");
            {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                assert!(dom_host.set_attribute(track, "default", ""));
                assert!(dom_host.set_attribute(track, "src", "captions/en.vtt"));
            }
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "native detached default track setup should not queue text-track timers before replaceChild insertion"
            );

            page_vm
                .evaluate_expression(
                    r#"
document.getElementById('js-replace-track-video').replaceChild(
  window.jsReplaceInsertedTrack,
  document.getElementById('js-replace-old-track-slot')
);
"#,
                )
                .expect("replaceChild inserted track should evaluate");

            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "default text-track mode selection must not acquire a PageTimer descriptor"
            );
            let task = take_next_dom_manipulation_task_for_test(&page_vm);
            assert!(
                matches!(
                    task,
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
                ),
                "mutation-owned default-mode work should share the DOM-manipulation source"
            );
        }));
}
#[test]
fn parser_mutation_owner_queues_inserted_default_text_track_without_getter() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        assert_eq!(
            page_vm.vm().ms_to_next_timeout(),
            None,
            "test setup should start without pending timers"
        );

        let (video, track) = {
            let body = create_connected_html_body_for_test(&mut page_vm);
            let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
            let video = dom_host.create_parser_element_without_attributes(
                "video".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
            assert!(dom_host.append_child(body, video));
            let track = dom_host.create_parser_element_without_attributes(
                "track".to_owned(),
                "http://www.w3.org/1999/xhtml".to_owned(),
                None,
            );
            assert!(dom_host.set_attribute(track, "default", ""));
            assert!(dom_host.set_attribute(track, "src", "captions/en.vtt"));
            (video, track)
        };
        assert_eq!(
            page_vm.vm().ms_to_next_timeout(),
            None,
            "creating a disconnected parser track should not queue text-track timers"
        );

        let custom_element_reaction_roots = {
            apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: video,
                    child: track,
                },
                "parser DOM mutation should apply",
            )
        };
        assert!(
            custom_element_reaction_roots.is_empty(),
            "plain track insertion should not queue custom element reactions"
        );
        assert_eq!(
            page_vm.vm().ms_to_next_timeout(),
            None,
            "parser insertion must not represent default-mode work as a timer"
        );
        let task = take_next_dom_manipulation_task_for_test(&page_vm);
        assert!(
            matches!(
                task,
                crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
            ),
            "parser-owned default-mode work should share the DOM-manipulation source"
        );
    }));
}
#[test]
fn phase_one_parser_inserted_image_queues_load_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let html = r#"<!doctype html><html><body>
<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">
</body></html>"#;
        let page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;

        let snapshot = page_vm.vm().snapshot_live_document();
        let body = snapshot.document_body_handle().expect("body handle");
        let image = snapshot
            .child_nodes(body)
            .expect("body children")
            .into_iter()
            .find(|handle| {
                snapshot
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("img"))
            })
            .expect("parser-created image");
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("context host should be alive");
        let pending = context_host
            .borrow()
            .pending_image_load_event(image)
            .expect("parser insertion should queue an image request sequence");
        assert_eq!(
            pending.request_initiator_type(),
            crate::types::SubresourceRequestInitiatorType::Parser,
            "parser insertion should preserve its request initiator through the image owner"
        );
    }));
}
#[test]
fn phase_one_parser_inserted_lazy_media_registers_candidate_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let html = r#"<!doctype html><html><body>
<video controls loading="lazy" src="data:video/mp4;base64,AAAA"></video>
</body></html>"#;
        let page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;

        let snapshot = page_vm.vm().snapshot_live_document();
        let body = snapshot.document_body_handle().expect("body handle");
        let video = snapshot
            .child_nodes(body)
            .expect("body children")
            .into_iter()
            .find(|handle| {
                snapshot
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("video"))
            })
            .expect("parser-created video");
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("context host should be alive");
        assert!(
            context_host
                .borrow()
                .lazy_media_load_candidates()
                .contains(&video),
            "parser insertion should register lazy media candidates from the runtime mutation owner"
        );
    }));
}
#[test]
fn parser_document_fragment_insertion_queues_resource_followups_from_hoisted_roots() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (fragment, _container, image, video) =
                create_parser_resource_fragment_for_test(&mut page_vm, "parser-fragment-resource");
            assert!(
                !context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "disconnected fragment setup should not queue image load events"
            );
            assert!(
                !context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "disconnected fragment setup should not register lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "disconnected fragment setup should not queue text-track timers"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment resource insertion should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain resource fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment resource insertion should hoist and empty the fragment"
            );
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "parser DocumentFragment insertion should queue image load events for hoisted subtree children"
            );
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "parser DocumentFragment insertion should register hoisted lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "parser DocumentFragment insertion must not represent text-track default-mode work as a timer"
            );
            assert!(matches!(
                take_next_dom_manipulation_task_for_test(&page_vm),
                crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(_)
            ));
            assert!(
                matches!(
                    take_next_dom_manipulation_task_for_test(&page_vm),
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
                ),
                "hoisted text track should follow the earlier image in the shared DOM FIFO"
            );
        }));
}
#[test]
fn parser_document_fragment_insert_before_queues_resource_followups_from_hoisted_roots() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            let reference = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-resource-ref"));
                assert!(dom_host.append_child(body, reference));
                reference
            };

            let (fragment, container, image, video) = create_parser_resource_fragment_for_test(
                &mut page_vm,
                "parser-fragment-resource-before",
            );
            assert!(
                !context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "disconnected fragment setup should not queue image load events"
            );
            assert!(
                !context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "disconnected fragment setup should not register lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "disconnected fragment setup should not queue text-track timers"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment resource insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain resource fragment insertBefore should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment resource insertBefore should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![container, reference],
                "parser DocumentFragment resource insertBefore should place the hoisted root before the reference child"
            );
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "parser DocumentFragment insertBefore should queue image load events for hoisted subtree children"
            );
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "parser DocumentFragment insertBefore should register hoisted lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "parser DocumentFragment insertBefore must not represent text-track default-mode work as a timer"
            );
            assert!(matches!(
                take_next_dom_manipulation_task_for_test(&page_vm),
                crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(_)
            ));
            assert!(
                matches!(
                    take_next_dom_manipulation_task_for_test(&page_vm),
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
                ),
                "insertBefore-hoisted text track should follow the image in the shared DOM FIFO"
            );
        }));
}
#[test]
fn js_insert_before_inserted_lazy_media_registers_candidate_from_mutation_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            page_vm
                .evaluate_expression(
                    r#"
const anchor = document.createElement('div');
anchor.id = 'js-lazy-media-anchor';
const video = document.createElement('video');
video.id = 'js-lazy-media-video';
video.setAttribute('controls', '');
video.setAttribute('loading', 'lazy');
video.setAttribute('src', 'data:video/mp4;base64,AAAA');
window.jsLazyMediaAnchor = anchor;
window.jsLazyMediaVideo = video;
document.body.append(anchor, video);
"#,
                )
                .expect("lazy media insertBefore setup should evaluate");

            let video = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("js-lazy-media-video")
                .expect("connected lazy video should exist");
            context_host
                .borrow_mut()
                .remove_lazy_media_load_candidate(video);
            assert!(
                !context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "test setup should clear candidate registered by src attribute or initial append"
            );

            page_vm
                .evaluate_expression(
                    r#"
document.body.removeChild(window.jsLazyMediaVideo);
document.body.insertBefore(window.jsLazyMediaVideo, window.jsLazyMediaAnchor);
"#,
                )
                .expect("lazy media insertBefore mutation should evaluate");

            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "JS insertBefore should register inserted lazy media candidates from the runtime mutation owner"
            );
        }));
}
#[test]
fn lazy_media_stale_candidates_are_cleared_after_js_and_parser_remove() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let mut page_vm = new_phase_one_page_vm_for_test();
        let body = create_connected_html_body_for_test(&mut page_vm);
        let context_host = page_vm
            .vm()
            .context_host_weak_for_test()
            .upgrade()
            .expect("context host should be alive");

        page_vm
            .evaluate_expression(
                r#"
function makeLazyVideo(id) {
  const video = document.createElement('video');
  video.id = id;
  video.setAttribute('controls', '');
  video.setAttribute('loading', 'lazy');
  video.setAttribute('src', 'data:video/mp4;base64,AAAA');
  return video;
}
window.jsRemovedLazyMedia = makeLazyVideo('js-removed-lazy-media');
window.parserRemovedLazyMedia = makeLazyVideo('parser-removed-lazy-media');
document.body.append(window.jsRemovedLazyMedia, window.parserRemovedLazyMedia);
"#,
            )
            .expect("lazy media stale cleanup setup should evaluate");

        let (js_video, parser_video) = {
            let runtime = &page_vm.vm().document_runtime;
            (
                runtime
                    .get_element_by_id("js-removed-lazy-media")
                    .expect("JS removed lazy video should exist"),
                runtime
                    .get_element_by_id("parser-removed-lazy-media")
                    .expect("parser removed lazy video should exist"),
            )
        };
        assert!(
            context_host
                .borrow()
                .lazy_media_load_candidates()
                .contains(&js_video),
            "JS-created lazy media should be registered before removal"
        );
        assert!(
            context_host
                .borrow()
                .lazy_media_load_candidates()
                .contains(&parser_video),
            "parser-remove target lazy media should be registered before removal"
        );

        page_vm
            .evaluate_expression("document.body.removeChild(window.jsRemovedLazyMedia);")
            .expect("JS lazy media removal should evaluate");

        let parser_reaction_roots = {
            apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: body,
                    child: parser_video,
                },
                "parser lazy media removal should apply",
            )
        };
        if !parser_reaction_roots.is_empty() {
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
                    parser_reaction_roots,
                )
                .expect("parser lazy media removal reactions should dispatch");
        }

        page_vm
            .evaluate_expression("window.scrollTo(0, 1);")
            .expect("lazy media reveal scan should evaluate");

        let candidates = context_host.borrow().lazy_media_load_candidates();
        assert!(
            !candidates.contains(&js_video),
            "lazy media reveal scan should clear stale JS-removed candidates"
        );
        assert!(
            !candidates.contains(&parser_video),
            "lazy media reveal scan should clear stale parser-removed candidates"
        );
    }));
}
#[test]
fn parser_removed_image_pending_load_event_clears_when_dom_task_runs() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"<!doctype html><html><body>
<img id="parser-removed-pending-image" src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">
</body></html>"#;
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (body, image) = {
                let snapshot = page_vm.vm().snapshot_live_document();
                let runtime = &page_vm.vm().document_runtime;
                (
                    snapshot
                        .document_body_handle()
                        .expect("parser-created body should exist"),
                    runtime
                        .get_element_by_id("parser-removed-pending-image")
                        .expect("parser-created pending image should exist"),
                )
            };
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "parser insertion should leave an image load event pending before its DOM turn"
            );

            let parser_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: body,
                        child: image,
                    },
                    "parser image removal should apply",
                )
            };
            if !parser_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
                        parser_reaction_roots,
                    )
                    .expect("parser image removal reactions should dispatch");
            }

            let loader = page_vm.main_document_resource_loader();
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        crate::runtime::page_vm::PageSelectedTaskTestSelector::DomManipulation(
                            crate::runtime::page_vm::PageDomManipulationTestFamily::ImageLoadEvent,
                        ),
                        loader.request_client(),
                    )
                    .await
                    .expect("parser image DOM-manipulation task should run")
            );

            assert!(
                !context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "queued image load callback should clear pending state after parser removal"
            );
        }));
}
