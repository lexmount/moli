use super::*;
use anyhow::{Context, ensure};
use futures_util::FutureExt;
use moli_page_types::LayoutPolicy;
use std::panic::AssertUnwindSafe;

const WORLD_TIMEOUT: Duration = Duration::from_secs(10);

const CHILD: &str = r#"<!doctype html><style>
html,body{margin:0;padding:0;border:0;overflow:hidden}
#target{display:block;width:100px;height:20px;padding:0;border:0}
@media(min-width:300px){#target{width:200px}}
#sentinel{position:absolute;left:300px;top:0;width:10px;height:10px}
</style><div id=target></div><div id=sentinel></div>"#;

const INSTALL: &str = r#"(() => {
  globalThis.environmentRealm = '__ROLE__';
  const target = document.getElementById('target');
  const sentinel = document.getElementById('sentinel');
  const p = globalThis.environmentProbe = {
    role:environmentRealm, armed:false, ready:false, finished:false,
    ro:[], io:[], media:[], resize:[], raf:[], errors:[]
  };
  p.query = matchMedia('(min-width:300px)');
  p.initialMatches = p.query.matches;
  p.snapshot = phase => ({phase, role:p.role, initialMatches:p.initialMatches,
    ro:p.ro,io:p.io,media:p.media,resize:p.resize,raf:p.raf,errors:p.errors});
  const emit = phase => environmentWitness(JSON.stringify(p.snapshot(phase)));
  const fail = text => { p.errors.push(String(text)); emit('error'); };
  const realm = () => environmentRealm === p.role && target.ownerDocument === document;
  const maybe = () => {
    if (!p.ready && p.ro.length && p.io.length && p.raf.length) {
      p.ready = true;
      emit('initial');
    }
    if (p.armed && !p.finished && p.ro.length === 2 && p.io.length === 2 &&
        p.media.length === 1 && p.resize.length === 1 && p.raf.length === 2) {
      p.finished = true;
      p.roObserver.disconnect();
      p.ioObserver.disconnect();
      removeEventListener('resize', resized);
      p.query.removeEventListener('change', changed);
      emit('changed');
    }
  };
  function resized(event) {
    if (!p.armed) return;
    p.resize.push([event.type, this === window, event.currentTarget === window, realm()]);
    requestAnimationFrame(stamp => { p.raf.push([stamp,realm()]); maybe(); });
    maybe();
  }
  function changed(event) {
    p.media.push([event.matches, this === p.query, event.currentTarget === p.query, realm()]);
    maybe();
  }
  addEventListener('error', event => fail(event.message));
  addEventListener('unhandledrejection', event => fail(event.reason));
  addEventListener('resize', resized);
  p.query.addEventListener('change', changed);
  p.roObserver = new ResizeObserver(entries => {
    if (entries.length !== 1 || entries[0].target !== target || !realm()) {
      fail('RO target/Realm mismatch'); return;
    }
    p.ro.push(entries[0].contentRect.width);
    maybe();
  });
  p.ioObserver = new IntersectionObserver(entries => {
    if (entries.length !== 1 || entries[0].target !== sentinel || !realm()) {
      const entry = entries[0];
      fail('IO target/Realm mismatch: '+JSON.stringify({
        count:entries.length, sameTarget:entry?.target===sentinel, realm:realm(),
        targetId:entry?.target?.id??null,
        ownerDocument:entry?.target?.ownerDocument===document,
        arrayRealm:Object.getPrototypeOf(entries)===Array.prototype,
        entryRealm:entry?Object.getPrototypeOf(entry)===IntersectionObserverEntry.prototype:null
      })); return;
    }
    p.io.push(entries[0].isIntersecting);
    maybe();
  });
  p.roObserver.observe(target);
  p.ioObserver.observe(sentinel);
  requestAnimationFrame(stamp => { p.raf.push([stamp,realm()]); maybe(); });
  return JSON.stringify({role:p.role, initialMatches:p.initialMatches});
})()"#;

async fn world_json(
    page: &RendererPageHandle,
    context: Option<i64>,
    expression: String,
) -> anyhow::Result<serde_json::Value> {
    let command = match context {
        Some(execution_context_id) => RendererPageCommand::EvaluateExpressionInExecutionContext {
            execution_context_id,
            expression,
            await_promise: false,
        },
        None => RendererPageCommand::EvaluateExpression {
            expression,
            await_promise: false,
        },
    };
    let (reply, _) = page.run_async_command(command).await?;
    let RendererPageReply::RuntimeEvaluationResult(result) = reply else {
        anyhow::bail!("expected owned runtime evaluation result");
    };
    let payload = result.into_protocol_payload();
    ensure!(
        payload.get("exception").is_none() && payload.get("exceptionDetails").is_none(),
        "script exception: {payload}"
    );
    serde_json::from_str(
        payload["value"]
            .as_str()
            .context("expected primitive JSON string")?,
    )
    .context("invalid stored witness")
}

async fn world_witnesses(
    output: &mut RendererExternalActivityTestReceiver,
    page: &RendererPageHandle,
    contexts: &[(i64, &str)],
    phase: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut witnesses = std::collections::BTreeMap::new();
    while witnesses.len() != contexts.len() {
        let publication = output.recv().await.context("renderer output closed")?;
        if !publication_is_for_page(&publication, page) {
            continue;
        }
        // Inspect the complete publication. A later record may contain a
        // duplicate witness or an exception after the desired binding call.
        for record in publication.into_records() {
            match record.into_parts().1 {
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeBinding(
                    call,
                )) if call.name == "environmentWitness" => {
                    let (_, role) = contexts
                        .iter()
                        .find(|(id, _)| *id == call.execution_context_id)
                        .context("binding was published by the wrong execution context")?;
                    let value: serde_json::Value = serde_json::from_str(&call.payload)?;
                    ensure!(value["role"] == *role && value["phase"] == phase, "{value}");
                    ensure!(value["errors"] == serde_json::json!([]), "{value}");
                    ensure!(
                        witnesses.insert(call.execution_context_id, value).is_none(),
                        "duplicate {phase} witness in execution context {}",
                        call.execution_context_id,
                    );
                }
                RendererOutputItem::Observation(
                    RendererProtocolObservation::RuntimeLifecycleError {
                        text,
                        execution_context_id,
                    },
                ) => anyhow::bail!("runtime lifecycle error {execution_context_id:?}: {text}"),
                RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(
                    batch,
                )) => {
                    for message in batch.messages {
                        if let RendererRuntimeInspectorMessage::Protocol(message) = message {
                            ensure!(
                                message.get("method").and_then(serde_json::Value::as_str)
                                    != Some("Runtime.exceptionThrown"),
                                "observed runtime exception: {message:?}",
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(contexts
        .iter()
        .map(|(id, _)| witnesses.remove(id).unwrap())
        .collect())
}

fn check_witness(value: &serde_json::Value, changed: bool) -> anyhow::Result<()> {
    ensure!(value["initialMatches"] == false, "{value}");
    ensure!(
        value["ro"]
            == if changed {
                serde_json::json!([100, 200])
            } else {
                serde_json::json!([100])
            },
        "{value}",
    );
    ensure!(
        value["io"]
            == if changed {
                serde_json::json!([false, true])
            } else {
                serde_json::json!([false])
            },
        "{value}",
    );
    ensure!(
        value["media"]
            == if changed {
                serde_json::json!([[true, true, true, true]])
            } else {
                serde_json::json!([])
            },
        "{value}",
    );
    ensure!(
        value["resize"]
            == if changed {
                serde_json::json!([["resize", true, true, true]])
            } else {
                serde_json::json!([])
            },
        "{value}",
    );
    let raf = value["raf"].as_array().context("rAF witness array")?;
    ensure!(raf.len() == if changed { 2 } else { 1 }, "{value}");
    for item in raf {
        ensure!(
            item[0].as_f64().is_some_and(f64::is_finite) && item[1] == true,
            "{value}"
        );
    }
    if changed {
        ensure!(raf[1][0].as_f64() >= raf[0][0].as_f64(), "{value}");
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_child_viewport_notifies_default_and_isolated_worlds_without_queries() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(LayoutPolicy::Realtime)
        .unwrap();
    let (sender, mut output) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(sender);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let escaped = CHILD.replace('&', "&amp;").replace('"', "&quot;");
    let html = format!(
        "<!doctype html><body style='margin:0'><iframe id=frame style='display:block;width:200px;height:100px;border:0;padding:0' srcdoc=\"{escaped}\"></iframe></body>"
    );
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://realtime-world-environment.test/").unwrap(),
        &html,
    )
    .await;
    let mut diagnostic_contexts = Vec::new();
    let case = AssertUnwindSafe(tokio::time::timeout(WORLD_TIMEOUT, async {
        let defaults = child_default_context_ids_for_test(&page).await?;
        ensure!(defaults.len() == 1, "one loaded child default context: {defaults:?}");
        let default = defaults[0];
        let frame = child_frame_id_for_default_context_id_for_test(&page, default).await?;
        let isolated = create_isolated_world_for_frame_for_test(&page, &frame, "environment-utility").await?;
        ensure!(default != isolated, "isolated world needs its own execution context");
        let contexts = [(default, "default"), (isolated, "isolated")];
        diagnostic_contexts.extend([default, isolated]);
        for (context, role) in contexts {
            add_runtime_binding_for_test(&page, "environmentWitness", None, Some(context)).await?;
            let installed = world_json(&page, Some(context), INSTALL.replace("__ROLE__", role)).await?;
            ensure!(installed == serde_json::json!({"role":role,"initialMatches":false}), "{installed}");
        }
        // Initial rAF/RO/IO callbacks are a positive control for each actual
        // Realm and the normal owner frame/IO tasks, before the resize input.
        for witness in world_witnesses(&mut output, &page, &contexts, "initial").await? {
            check_witness(&witness, false)?;
        }
        let initial = RendererPageTestingHandle::new_for_testing(&page)
            .layout_working_set_observation_async().await?;
        ensure!(initial.frame_idle, "initial lifecycle must have settled: {initial:#?}");
        ensure!(initial.last_pass_paint_operations == Some(0), "{initial:#?}");
        world_json(&page, None, r#"(() => {
          window.environmentControl={resize:0,media:[]};
          addEventListener('resize',()=>environmentControl.resize++);
          environmentControl.query=matchMedia('(min-width:300px)');
          environmentControl.query.addEventListener('change',e=>environmentControl.media.push(e.matches));
          return JSON.stringify({defaultMarker:globalThis.environmentRealm??null});
        })()"#.to_owned()).await.and_then(|value| {
            ensure!(value == serde_json::json!({"defaultMarker":null}), "{value}"); Ok(())
        })?;
        for (context, _) in contexts {
            world_json(&page, Some(context), "environmentProbe.armed=true;JSON.stringify(true)".to_owned()).await?;
        }
        // This is the only resize input. It performs no geometric/style read.
        // No further Page command is issued until both callback witnesses exist.
        let changed = world_json(&page, None,
            "document.getElementById('frame').style.width='400px';JSON.stringify('changed')".to_owned()).await?;
        ensure!(changed == "changed", "{changed}");
        for witness in world_witnesses(&mut output, &page, &contexts, "changed").await? {
            check_witness(&witness, true)?;
        }
        let final_state = RendererPageTestingHandle::new_for_testing(&page)
            .layout_working_set_observation_async().await?;
        ensure!(final_state.frame_idle, "completed finite callbacks must not renew frames: {final_state:#?}");
        ensure!(final_state.last_pass_paint_operations == Some(0), "{final_state:#?}");
        let control = world_json(&page, None,
            "JSON.stringify({resize:environmentControl.resize,media:environmentControl.media,marker:globalThis.environmentRealm??null})".to_owned()).await?;
        ensure!(control == serde_json::json!({"resize":0,"media":[],"marker":null}), "{control}");
        for (context, role) in contexts {
            let stored = world_json(&page, Some(context),
                "JSON.stringify(environmentProbe.snapshot('changed'))".to_owned()).await?;
            ensure!(stored["role"] == role, "{stored}");
            check_witness(&stored, true)?;
        }
        Ok::<(), anyhow::Error>(())
    })).catch_unwind().await;

    // Failed callback waits stay failed. These bounded, stored-only reads are
    // diagnostics after the case terminates, never a way to rescue its result.
    if !matches!(&case, Ok(Ok(Ok(())))) {
        for context in diagnostic_contexts {
            let stored = tokio::time::timeout(
                Duration::from_secs(5),
                world_json(
                    &page,
                    Some(context),
                    "JSON.stringify(globalThis.environmentProbe?.snapshot('diagnostic')??null)"
                        .to_owned(),
                ),
            )
            .await;
            eprintln!("environment context {context} stored witness: {stored:#?}");
        }
    }
    let closed = tokio::time::timeout(Duration::from_secs(5), page.close_async()).await;
    match case {
        Err(panic) => {
            eprintln!("environment case panicked; Page close: {closed:#?}");
            std::panic::resume_unwind(panic);
        }
        Ok(Err(error)) => panic!("environment callback deadline: {error}; Page close: {closed:#?}"),
        Ok(Ok(Err(error))) => {
            panic!("environment callback failure: {error:#}; Page close: {closed:#?}")
        }
        Ok(Ok(Ok(()))) => {
            closed
                .expect("Page close deadline")
                .expect("Page should close normally");
        }
    }
}
