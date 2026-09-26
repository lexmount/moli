use super::*;

#[test]
fn parser_created_custom_element_direct_constructs_before_token_attributes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.ceEvents = [];
window.WptTokenTiming = class extends HTMLElement {
  constructor() {
    super();
    let writeResult = 'missing';
    try {
      document.write('<b id="bad-write">bad</b>');
      writeResult = 'ok';
    } catch (error) {
      writeResult = error.name;
    }
    window.ceEvents.push([
      this.hasAttribute('data-token'),
      !!document.getElementById('after'),
      this.isConnected,
      writeResult
    ].join('|'));
  }
  connectedCallback() {
    window.ceEvents.push([
      'connected',
      this.getAttribute('data-token'),
      !!document.getElementById('after'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-token-timing', window.WptTokenTiming);
</script>
<wpt-token-timing data-token="owned"></wpt-token-timing><span id="after"></span>
<script>
const element = document.querySelector('wpt-token-timing');
document.body.setAttribute('data-first-event', window.ceEvents[0] || '');
document.body.setAttribute('data-second-event', window.ceEvents[1] || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-after-visible', String(!!document.getElementById('after')));
document.body.setAttribute('data-instance', String(element instanceof window.WptTokenTiming));
document.body.setAttribute('data-bad-write', String(!!document.getElementById('bad-write')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|false|false|InvalidStateError"),
                "constructor must run before parser token attributes and following siblings are visible"
            );
            assert_eq!(
                body_element.attribute("data-second-event"),
                Some("connected|owned|false|true"),
                "connectedCallback must run after parser insertion but before following parser tokens are appended"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-after-visible"), Some("true"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-bad-write"), Some("false"));
        }));
}
#[test]
fn parser_created_custom_element_uses_constructor_returned_element() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let page_vm = parse_phase_one_html_into_page_vm_for_test(
            r#"<!doctype html><html><body>
<script>
let shouldCreateAnotherInstance = true;
let anotherInstance = undefined;
let firstInstance = undefined;
class ReturnsAnotherInstance extends HTMLElement {
  constructor() {
    super();
    if (shouldCreateAnotherInstance) {
      shouldCreateAnotherInstance = false;
      firstInstance = this;
      anotherInstance = new ReturnsAnotherInstance();
      return anotherInstance;
    }
  }
}
customElements.define('returns-another-instance', ReturnsAnotherInstance);
</script>
<returns-another-instance></returns-another-instance>
<script>
const instance = document.querySelector('returns-another-instance');
document.body.setAttribute('data-parser-returned', [
  instance instanceof ReturnsAnotherInstance,
  instance === anotherInstance,
  instance !== firstInstance,
  firstInstance.parentNode === null,
  anotherInstance.parentNode === document.body
].join('|'));
</script>
</body></html>"#,
        )
        .await;

        let snapshot = page_vm.vm().snapshot_live_document();
        let body = snapshot.document_body_handle().expect("body");
        let body_element = snapshot
            .node(body)
            .and_then(Node::as_element)
            .expect("body element");
        assert_eq!(
            body_element.attribute("data-parser-returned"),
            Some("true|true|true|true|true"),
            "parser synchronous construction must insert the element returned by the constructor"
        );
    }));
}
#[test]
fn parser_created_custom_element_token_attributes_queue_initial_reactions() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.ceEvents = [];
window.ceCallbackState = {};
window.WptTokenAttributes = class extends HTMLElement {
  static get observedAttributes() { return ['data-token', 'data-extra']; }
  constructor() {
    super();
    window.ceEvents.push('ctor-has=' + this.hasAttribute('data-token'));
    new MutationObserver((records) => {
      for (const record of records) {
        window.ceEvents.push(
          'mo:' + record.attributeName + ':' +
          this.getAttribute(record.attributeName)
        );
      }
    }).observe(this, { attributes: true });
  }
  attributeChangedCallback(name, oldValue, newValue) {
    window.ceCallbackState[name] = {
      token: this.getAttribute('data-token'),
      extra: this.getAttribute('data-extra')
    };
    window.ceEvents.push('attr:' + name + ':' + oldValue + ':' + newValue);
    Promise.resolve().then(() => {
      window.ceEvents.push('promise:' + this.getAttribute(name));
    });
  }
  connectedCallback() {
    window.ceEvents.push('connected:' + this.getAttribute('data-token'));
  }
};
customElements.define('wpt-token-attributes', window.WptTokenAttributes);
</script>
<wpt-token-attributes data-token="owned" data-extra="extra"></wpt-token-attributes>
<script>
document.body.setAttribute('data-events', window.ceEvents.join('|'));
document.body.setAttribute(
  'data-token-callback-saw-extra',
  window.ceCallbackState['data-token'].extra || ''
);
document.body.setAttribute(
  'data-extra-callback-saw-token',
  window.ceCallbackState['data-extra'].token || ''
);
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element token attrs reaction channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-token-callback-saw-extra"),
                Some("extra"),
                "all parser token attributes must be appended before the first queued attribute reaction is flushed"
            );
            assert_eq!(
                body_element.attribute("data-extra-callback-saw-token"),
                Some("owned"),
                "later parser token attribute reactions should observe earlier token attributes"
            );
        }));
}
#[test]
fn parser_created_custom_element_direct_constructs_before_declarative_shadow() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.shadowTiming = [];
class ShadowHostElement extends HTMLElement {
  constructor() {
    super();
    window.shadowTiming.push([
      'constructor',
      !!this.shadowRoot,
      this.childNodes.length,
      !!this.querySelector('template'),
      !!document.getElementById('after-shadow-host')
    ].join('|'));
  }
  connectedCallback() {
    window.shadowTiming.push([
      'connected',
      !!this.shadowRoot,
      this.childNodes.length,
      !!this.querySelector('template'),
      !!document.getElementById('after-shadow-host')
    ].join('|'));
  }
}
customElements.define('shadow-host-element', ShadowHostElement);
</script>
<shadow-host-element><template shadowrootmode="open"><span>Shadow Content</span></template><p>Light Content</p></shadow-host-element><span id="after-shadow-host"></span>
<script>
const element = document.querySelector('shadow-host-element');
document.body.setAttribute('data-constructor-event', window.shadowTiming[0] || '');
document.body.setAttribute('data-connected-event', window.shadowTiming[1] || '');
document.body.setAttribute('data-shadow-ready', [
  element instanceof ShadowHostElement,
  !!element.shadowRoot,
  element.shadowRoot && element.shadowRoot.textContent.trim(),
  !!element.querySelector('template'),
  element.textContent.trim(),
  !!document.getElementById('after-shadow-host')
].join('|'));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element DSD timing local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-constructor-event"),
                Some("constructor|false|0|false|false"),
                "constructor must run before declarative shadow template contents and following siblings are parsed"
            );
            assert_eq!(
                body_element.attribute("data-connected-event"),
                Some("connected|false|0|false|false"),
                "connectedCallback is delivered before declarative shadow and child tokens are appended"
            );
            assert_eq!(
                body_element.attribute("data-shadow-ready"),
                Some("true|true|Shadow Content|false|Light Content|true"),
                "declarative shadow root should attach after construction while light DOM and following siblings continue parsing"
            );
        }));
}
#[test]
fn parser_connected_form_associated_custom_element_dispatches_form_reactions() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script>
window.parserFaceEvents = [];
class ParserFaceElement extends HTMLElement {
  static formAssociated = true;
  constructor() {
    super();
    this.internals = this.attachInternals();
  }
  connectedCallback() {
    window.parserFaceEvents.push(`connected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceEvents.push(`form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceEvents.push(`disabled:${disabled}`);
  }
}
customElements.define('parser-face-element', ParserFaceElement);
</script>
<form id="parser-form"><fieldset disabled><parser-face-element id="parser-face"></parser-face-element></fieldset></form>
<script>
const face = document.getElementById('parser-face');
document.body.setAttribute('data-face-events', window.parserFaceEvents.join('|'));
document.body.setAttribute('data-face-form', face.internals.form && face.internals.form.id);
document.body.setAttribute('data-face-disabled', String(face.matches(':disabled')));
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-face-events"),
                Some("connected:true|form:parser-form|disabled:true"),
                "parser insertion should dispatch connected, form-associated, and form-disabled reactions after the parser step"
            );
            assert_eq!(
                body_element.attribute("data-face-form"),
                Some("parser-form")
            );
            assert_eq!(body_element.attribute("data-face-disabled"), Some("true"));
        }));
}
#[test]
fn parser_created_custom_element_direct_waits_for_head_reprocess_body() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><head>
<script>
window.headCeEvents = [];
window.WptHeadTiming = class extends HTMLElement {
  constructor() {
    super();
    window.headCeEvents.push([
      this.hasAttribute('data-token'),
      !!document.body,
      !!document.getElementById('after-head'),
      this.isConnected
    ].join('|'));
  }
  connectedCallback() {
    window.headCeEvents.push([
      'connected',
      this.getAttribute('data-token'),
      this.parentElement && this.parentElement.localName,
      !!document.body,
      !!document.getElementById('after-head'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-head-timing', window.WptHeadTiming);
</script>
<wpt-head-timing data-token="owned"></wpt-head-timing><meta id="after-head">
</head><body><span id="after-body"></span>
<script>
const element = document.querySelector('wpt-head-timing');
document.body.setAttribute('data-first-event', window.headCeEvents[0] || '');
document.body.setAttribute('data-second-event', window.headCeEvents[1] || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-parent', element.parentElement && element.parentElement.localName);
document.body.setAttribute('data-instance', String(element instanceof window.WptHeadTiming));
document.body.setAttribute('data-after-head-visible', String(!!document.getElementById('after-head')));
document.body.setAttribute('data-after-body-visible', String(!!document.getElementById('after-body')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser head reprocess custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|true|false|false"),
                "head reprocess must create body before direct construction but before token attributes and following siblings"
            );
            assert_eq!(
                body_element.attribute("data-second-event"),
                Some("connected|owned|body|true|false|true"),
                "reprocessed custom element must connect under body before following parser tokens"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-parent"), Some("body"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(
                body_element.attribute("data-after-head-visible"),
                Some("true")
            );
            assert_eq!(
                body_element.attribute("data-after-body-visible"),
                Some("true")
            );
        }));
}
#[test]
fn parser_created_custom_element_direct_skips_template_contents() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),

                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.templateConstructed = 0;
window.WptTemplateTiming = class extends HTMLElement {
  constructor() {
    super();
    window.templateConstructed += 1;
  }
};
customElements.define('wpt-template-timing', window.WptTemplateTiming);
</script>
<template id="template-root"><wpt-template-timing data-token="owned"></wpt-template-timing></template>
<script>
const instance = document.getElementById('template-root').content.firstElementChild;
document.body.setAttribute('data-constructed', String(window.templateConstructed));
document.body.setAttribute('data-html-element', String(instance instanceof HTMLElement));
document.body.setAttribute('data-instance', String(instance instanceof window.WptTemplateTiming));
document.body.setAttribute('data-token', instance.getAttribute('data-token') || '');
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser template custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-constructed"),
                Some("0"),
                "parser must not instantiate custom elements inside template contents"
            );
            assert_eq!(body_element.attribute("data-html-element"), Some("true"));
            assert_eq!(body_element.attribute("data-instance"), Some("false"));
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
        }));
}
