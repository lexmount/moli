use super::*;
use crate::custom_elements::CustomElementRegistryKey;

mod insertion;
mod parser;
mod upgrade_reentry;

async fn assert_popup_custom_element_microtask_order(javascript_url: bool) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://popup-custom-element-microtasks.test/",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.__popupCeLog = [];
        globalThis.PopupElement = class extends HTMLElement {
          constructor() {
            super();
            __popupCeLog.push("constructor");
            Promise.resolve().then(() => {
              __popupCeLog.push("constructor-microtask");
              this.setAttribute("data-constructed", "yes");
            });
          }
        };
        customElements.define("popup-microtask-element", PopupElement);
        "ready";
        "#,
    )
    .expect("popup custom element definition should register");

    let source = r#"
        Promise.resolve().then(() => opener.__popupCeLog.push("earlier-microtask"));
        const element = opener.document.createElement("popup-microtask-element");
        opener.__popupCeElement = element;
        opener.__popupCeLog.push("after-create", element instanceof opener.PopupElement,
                                element.hasAttribute("data-constructed"));
        void 0;
    "#;
    let navigation = if javascript_url {
        format!("open({:?})", format!("javascript:{source}"))
    } else {
        let html = format!("<!doctype html><script>{source}</script>");
        format!("open(URL.createObjectURL(new Blob([{html:?}], {{type: 'text/html'}})))")
    };
    vm.eval(&format!(
        "globalThis.__popupCeWindow = {navigation}; 'queued'"
    ))
    .expect("host-owned popup execution should queue");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupCeLog.includes('constructor-microtask'))",
        "true",
        "popup custom element constructor microtask",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify({log: __popupCeLog, custom: __popupCeElement instanceof PopupElement, value: __popupCeElement.getAttribute('data-constructed')})")
            .expect("popup custom element result should be observable"),
        r#"{"log":["constructor","after-create",true,false,"earlier-microtask","constructor-microtask"],"custom":true,"value":"yes"}"#,
        "popup source must finish before constructor microtasks (javascript URL: {javascript_url})",
    );
}

mod construction_and_document_upgrade;
mod custom_state_and_reaction_order;
mod detached_documents_and_reactions;
mod registry_reactions_and_builtins;
mod registry_scoping_and_fragments;
