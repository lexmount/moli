use super::*;
use crate::runtime::owner_local::RendererLayoutWorkingSetObservation;
use moli_page_types::LayoutPolicy;

const INTERACTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
enum InteractionState {
    Focus,
    Hover,
}

impl InteractionState {
    fn selector(self) -> &'static str {
        match self {
            Self::Focus => ":focus",
            Self::Hover => ":hover",
        }
    }

    fn enter_event(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Hover => "pointerover",
        }
    }

    fn leave_event(self) -> &'static str {
        match self {
            Self::Focus => "blur",
            Self::Hover => "pointerout",
        }
    }
}

async fn interaction_command(page: &RendererPageHandle, expression: &str) -> serde_json::Value {
    let (reply, _) = tokio::time::timeout(
        INTERACTION_TIMEOUT,
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: expression.to_owned(),
            await_promise: false,
        }),
    )
    .await
    .expect("interaction script command must complete")
    .expect("interaction script command must succeed");
    renderer_json_value(reply).expect("script must return a value, not an exception")
}

async fn interaction_observation(page: &RendererPageHandle) -> RendererLayoutWorkingSetObservation {
    tokio::time::timeout(
        INTERACTION_TIMEOUT,
        RendererPageTestingHandle::new_for_testing(page).layout_working_set_observation_async(),
    )
    .await
    .expect("read-only owner observation must complete")
    .expect("the owner must restore its Page after the callback turn")
}

async fn next_interaction_resize(
    output: &mut RendererExternalActivityTestReceiver,
    page: &RendererPageHandle,
    expected_phase: &str,
) -> serde_json::Value {
    let witness = tokio::time::timeout(INTERACTION_TIMEOUT, async {
        loop {
            let publication = output.recv().await.expect("owner output remains open");
            if !publication_is_for_page(&publication, page) {
                continue;
            }
            let mut payloads = Vec::new();
            let mut errors = Vec::new();
            // Consume the entire publication before accepting its witness;
            // a later record in this same batch may be another RO or an error.
            for record in publication.into_records() {
                match record.into_parts().1 {
                    RendererOutputItem::Observation(
                        RendererProtocolObservation::RuntimeBinding(call),
                    ) if call.name == "interactionWitness" => payloads.push(call.payload),
                    RendererOutputItem::Observation(
                        RendererProtocolObservation::RuntimeLifecycleError {
                            text,
                            execution_context_id,
                        },
                    ) => errors.push(format!("{execution_context_id:?}: {text}")),
                    RendererOutputItem::Observation(
                        RendererProtocolObservation::RuntimeInspector(batch),
                    ) => {
                        for message in batch.messages {
                            if let RendererRuntimeInspectorMessage::Protocol(message) = message
                                && message.get("method").and_then(serde_json::Value::as_str)
                                    == Some("Runtime.exceptionThrown")
                            {
                                errors.push(format!("{message:?}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
            assert!(errors.is_empty(), "observed runtime errors: {errors:?}");
            if payloads.is_empty() {
                continue;
            }
            assert_eq!(payloads.len(), 1, "one witness per phase: {payloads:?}");
            return serde_json::from_str::<serde_json::Value>(&payloads[0])
                .expect("fixture binding carries JSON");
        }
    })
    .await
    .expect("RO must publish its own witness without another Page command or screenshot");
    assert_eq!(witness["errors"], serde_json::json!([]), "{witness}");
    assert_eq!(witness["phase"], expected_phase, "{witness}");
    witness
}

async fn assert_clean_interaction_read(page: &RendererPageHandle, width: i64) {
    // The binding is published by the normal RO/frame turn. This owner command
    // ACK establishes a resident Page boundary; it neither pumps a frame nor
    // reads geometry. The following getters are deliberately after the witness.
    let before = interaction_observation(page).await;
    assert!(before.frame_idle, "{before:#?}");
    assert!(
        before.screen.3.is_some(),
        "screen geometry must already be published"
    );
    assert_eq!(before.last_pass_paint_operations, Some(0));
    let result = interaction_command(
        page,
        "JSON.stringify([interaction.held.width, interaction.target.offsetWidth, interaction.target.offsetWidth])",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(result.as_str().unwrap()).unwrap();
    assert_eq!(
        result,
        serde_json::json!([format!("{width}px"), width, width])
    );
    let after = interaction_observation(page).await;
    assert_eq!(
        after.builds, before.builds,
        "clean geometry must reuse the published result"
    );
    assert_eq!(
        after.screen.2, before.screen.2,
        "clean reads must not republish layout"
    );
    assert_eq!(after.screen.3, before.screen.3);
    assert_eq!(after.last_pass_paint_operations, Some(0));
    assert!(
        after.frame_idle,
        "clean reads must not schedule a new frame: {after:#?}"
    );
}

async fn hover_at(page: &RendererPageHandle, y: f64) {
    let (reply, _) = tokio::time::timeout(
        INTERACTION_TIMEOUT,
        page.run_async_command(RendererPageCommand::DispatchMouseEventAtPoint {
            x: 10.0,
            y,
            event_name: "mousemove".to_owned(),
            button: -1,
            buttons: Some(0),
            click_count: 0,
            delta_x: 0.0,
            delta_y: 0.0,
            pointer: RendererPointerEventProperties::default(),
            modifiers: 0,
        }),
    )
    .await
    .expect("native pointer command must complete")
    .expect("native pointer command must succeed");
    assert!(matches!(reply, RendererPageReply::InputDispatchOutcome(outcome) if outcome.handled));
}

async fn assert_interaction_drives_current_geometry_and_resize(state: InteractionState) {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::Realtime)
        .unwrap();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let html = format!(
        "<!doctype html><style>\
         html,body{{margin:0;padding:0;border:0}}\
         #target,#away{{display:block;box-sizing:content-box;width:100px;height:20px;padding:0;border:0}}\
         #away{{margin-top:20px}}#target{}{{width:200px}}\
         </style><div id=target tabindex=0></div><div id=away tabindex=0></div>",
        state.selector()
    );
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://realtime-interaction.test/").unwrap(),
        &html,
    )
    .await;
    add_runtime_binding_for_test(&page, "interactionWitness", None, None)
        .await
        .unwrap();
    let setup = r#"
        (() => {
          const target = document.getElementById('target');
          const held = getComputedStyle(target);
          const probe = window.interaction = {
            target, held, phase:'initial', widths:[], events:[], errors:[], returned:null,
            initial:[held.width,target.offsetWidth]
          };
          const emit = () => interactionWitness(JSON.stringify({
            phase:probe.phase,widths:probe.widths,events:probe.events,
            errors:probe.errors,initial:probe.initial,returned:probe.returned
          }));
          addEventListener('error', event => { probe.errors.push(String(event.message)); emit(); });
          addEventListener('unhandledrejection', event => { probe.errors.push(String(event.reason)); emit(); });
          target.addEventListener('__ENTER_EVENT__', event => {
            // First dimensional observation after native state commit happens
            // inside the first target event, using the already-held CSSOM object.
            const width = held.width;
            const offset = target.offsetWidth;
            const again = target.offsetWidth;
            probe.events.push([event.type,target.matches('__SELECTOR__'),width,offset,again]);
          });
          target.addEventListener('__LEAVE_EVENT__', event => {
            // Deliberately no style or geometry read on the return transition.
            probe.events.push([event.type,target.matches('__SELECTOR__')]);
          });
          probe.observer = new ResizeObserver(entries => {
            const entry = entries.find(entry => entry.target === target);
            if (!entry) { probe.errors.push('missing target entry'); emit(); return; }
            probe.widths.push(entry.contentRect.width);
            if (probe.phase === 'left') probe.observer.disconnect();
            emit();
          });
          probe.observer.observe(target);
          return 'installed';
        })()
    "#
    .replace("__ENTER_EVENT__", state.enter_event())
    .replace("__LEAVE_EVENT__", state.leave_event())
    .replace("__SELECTOR__", state.selector());
    assert_eq!(interaction_command(&page, &setup).await, "installed");
    let initial = next_interaction_resize(&mut output_rx, &page, "initial").await;
    assert_eq!(
        initial["initial"],
        serde_json::json!(["100px", 100]),
        "{initial}"
    );
    assert_eq!(initial["widths"], serde_json::json!([100]), "{initial}");
    assert_eq!(initial["events"], serde_json::json!([]), "{initial}");
    assert_clean_interaction_read(&page, 100).await;

    match state {
        InteractionState::Focus => {
            assert_eq!(
                interaction_command(&page, "interaction.phase='entered'; interaction.target.focus({preventScroll:true}); interaction.returned=[interaction.held.width,interaction.target.offsetWidth]; 'entered'").await,
                "entered"
            );
        }
        InteractionState::Hover => {
            assert_eq!(
                interaction_command(&page, "interaction.phase='entered'").await,
                "entered"
            );
            hover_at(&page, 10.0).await;
        }
    }
    // Only consume asynchronous owner output here. In particular, no evaluate
    // promise, testing frame opportunity, layout helper or capture advances RO.
    let entered = next_interaction_resize(&mut output_rx, &page, "entered").await;
    assert_eq!(
        entered["widths"],
        serde_json::json!([100, 200]),
        "{entered}"
    );
    assert_eq!(
        entered["events"],
        serde_json::json!([[state.enter_event(), true, "200px", 200, 200]]),
        "{entered}"
    );
    if matches!(state, InteractionState::Focus) {
        assert_eq!(
            entered["returned"],
            serde_json::json!(["200px", 200]),
            "{entered}"
        );
    }
    assert_clean_interaction_read(&page, 200).await;

    match state {
        InteractionState::Focus => {
            assert_eq!(interaction_command(&page, "interaction.phase='left'; document.getElementById('away').focus({preventScroll:true}); 'left'").await, "left");
        }
        InteractionState::Hover => {
            assert_eq!(
                interaction_command(&page, "interaction.phase='left'").await,
                "left"
            );
            hover_at(&page, 50.0).await;
        }
    }
    let left = next_interaction_resize(&mut output_rx, &page, "left").await;
    assert_eq!(left["widths"], serde_json::json!([100, 200, 100]), "{left}");
    assert_eq!(
        left["events"],
        serde_json::json!([
            [state.enter_event(), true, "200px", 200, 200],
            [state.leave_event(), false]
        ]),
        "{left}"
    );
    assert_clean_interaction_read(&page, 100).await;
    page.close_async().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_focus_commits_before_event_reads_and_drives_resize_without_queries() {
    assert_interaction_drives_current_geometry_and_resize(InteractionState::Focus).await;
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_native_hover_commits_before_event_reads_and_drives_resize_without_queries() {
    assert_interaction_drives_current_geometry_and_resize(InteractionState::Hover).await;
}
