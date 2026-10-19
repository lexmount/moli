use super::*;
#[cfg(feature = "devtools")]
use crate::document_runtime::DevToolsDomMutationFact;
use crate::live_document_parser::LiveDocumentParserStepOutcome;

#[test]
fn parser_text_batches_before_inspector_and_mutation_observers() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
        for with_window_realm in [false, true] {
            for raw_text in [false, true] {
                for seed in ["seed", " "] {
                    let PhaseOnePageVmHarness {
                        mut page_vm,
                        loader,
                        state,
                    } = new_phase_one_page_vm_harness_for_test();
                    let mut driver = ParserDriver {
                        loader,
                        final_url: &state.final_url,
                        parser_session: &mut state.parser_session,
                        scheduler: &mut state.scheduler,
                        buffered_document_preloads: &mut state.buffered_document_preloads,
                        service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                        input_closed: &state.input_closed,
                        image_document: &mut state.image_document,
                    };
                    let owner = page_vm.vm().current_main_document_task_owner().unwrap();
                    let element = if raw_text {
                        "<script type=application/json>"
                    } else {
                        "<main>"
                    };
                    let prefix = format!("<!doctype html><html><head></head><body>{element}{seed}");
                    assert!(matches!(
                        driver.pump_parse_step_with_signals(&mut page_vm, owner, &prefix),
                        LiveDocumentParserStepOutcome::InputBoundary
                    ));
                    if with_window_realm {
                        page_vm.vm_mut().eval(
                            "globalThis.__tree_buffer_old_values = []; \
                             globalThis.__tree_buffer_observer = new MutationObserver(records => { \
                               __tree_buffer_old_values.push(...records.map(record => record.oldValue)); \
                             }); \
                             __tree_buffer_observer.observe(document.querySelector('main, script').firstChild, \
                               {characterData:true, characterDataOldValue:true});",
                        ).unwrap();
                    }
                    #[cfg(feature = "devtools")]
                    {
                    // Publish before the next chunk, just as chromedp's
                    // DOM.getDocument does while a navigation is still streaming.
                    page_vm
                        .document_node_snapshot_for_document(Some("inspector"), -1, false)
                        .unwrap();
                    page_vm
                        .vm_mut()
                        .document_runtime
                        .take_pending_devtools_dom_mutations();
                    }
                    let input = if raw_text { "x<" } else { "x&amp;" }.repeat(2_048) + "x";
                    let expected =
                        seed.to_owned() + &if raw_text { "x<" } else { "x&" }.repeat(2_048) + "x";
                    assert!(matches!(
                        driver.pump_parse_step_with_signals(&mut page_vm, owner, &input),
                        LiveDocumentParserStepOutcome::InputBoundary
                    ));
                    #[cfg(feature = "devtools")]
                    {
                    let facts = page_vm
                        .vm_mut()
                        .document_runtime
                        .take_pending_devtools_dom_mutations();
                    let mut retained_text_bytes = 0;
                    let mut final_value = None;
                    let mut character_data_count = 0;
                    for fact in &facts {
                        if let DevToolsDomMutationFact::CharacterData {
                            old_value,
                            value,
                            snapshot,
                            ..
                        } = fact
                        {
                            character_data_count += 1;
                            assert_eq!(old_value, seed);
                            retained_text_bytes +=
                                old_value.len() + value.len() + snapshot.node_value.len();
                            final_value = Some(value.as_str());
                            assert_eq!(value, &snapshot.node_value);
                        }
                    }
                    assert_eq!(final_value, Some(expected.as_str()));
                    assert_eq!(character_data_count, 1);
                    assert!(
                        retained_text_bytes <= 4 * expected.len(),
                        "Inspector retained {retained_text_bytes} bytes for {} text bytes \
                         (realm={with_window_realm}, raw_text={raw_text})",
                        expected.len()
                    );
                    }
                    if with_window_realm {
                        let actual = page_vm.vm_mut().eval("document.querySelector('main, script').textContent").unwrap();
                        assert_eq!(actual, expected);
                        let old_values = page_vm.vm_mut().eval(
                            "JSON.stringify(__tree_buffer_old_values.concat( \
                               __tree_buffer_observer.takeRecords().map(record => record.oldValue)))",
                        ).unwrap();
                        assert_eq!(old_values, serde_json::to_string(&[seed]).unwrap());
                    }
                }
            }
        }
    }));
}
