use super::*;
use crate::document_runtime::DevToolsDomMutationFact;
use crate::live_document_parser::LiveDocumentParserStepOutcome;

#[test]
fn parser_implicit_body_insertion_does_not_commit_following_character_fragments() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(concat!(
            r#"<!doctype html><html><head><script>
window.textRecords = [];
window.textObserver = new MutationObserver(records => textRecords.push(...records));
textObserver.observe(document.documentElement, {
  subtree: true, childList: true, characterData: true, characterDataOldValue: true
});
</script></head>"#,
            " \na&amp;b&amp;c",
        ))
        .await;
        let actual = page_vm.vm_mut().eval(
            "JSON.stringify({ \
               children: [...document.documentElement.childNodes].map(n => [n.nodeName, n.nodeValue]), \
               text: document.body.textContent, \
               old: textRecords.concat(textObserver.takeRecords()) \
                 .filter(r => r.type === 'characterData' && r.target.parentNode === document.body) \
                 .map(r => r.oldValue) \
             })",
        ).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            serde_json::json!({
                "children": [["HEAD", null], ["#text", " \n"], ["BODY", null]],
                "text": "a&b&c",
                "old": []
            })
        );
    }));
}

#[test]
fn parser_custom_element_callbacks_see_text_committed_for_their_insertion() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(tokio::task::LocalSet::new().run_until(async {
        let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><head><script>
window.insertionReads = [];
customElements.define('x-probe', class extends HTMLElement {
  constructor() {
    super();
    insertionReads.push(['constructor', document.querySelector('main').textContent]);
  }
  connectedCallback() {
    insertionReads.push(['connected', this.previousSibling.data, this.textContent]);
  }
});
</script></head><body><main>a&amp;b<x-probe>inside</x-probe>c</main></body></html>"#,
        )
        .await;
        let actual = page_vm.vm_mut().eval(
            "JSON.stringify(insertionReads.concat([['final', document.querySelector('main').textContent]]))",
        ).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
            serde_json::json!([
                ["constructor", "a&b"],
                ["connected", "a&b", ""],
                ["final", "a&binsidec"]
            ])
        );
    }));
}

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
                    // Publish before the next chunk, just as chromedp's
                    // DOM.getDocument does while a navigation is still streaming.
                    page_vm
                        .document_node_snapshot_for_document(Some("inspector"), -1, false)
                        .unwrap();
                    page_vm
                        .vm_mut()
                        .document_runtime
                        .take_pending_devtools_dom_mutations();
                    let input = if raw_text { "x<" } else { "x&amp;" }.repeat(2_048) + "x";
                    let expected =
                        seed.to_owned() + &if raw_text { "x<" } else { "x&" }.repeat(2_048) + "x";
                    assert!(matches!(
                        driver.pump_parse_step_with_signals(&mut page_vm, owner, &input),
                        LiveDocumentParserStepOutcome::InputBoundary
                    ));
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
                    if with_window_realm {
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
