use super::*;

#[test]
fn custom_state_set_invalidates_shadow_source_scopes() {
    let mut vm = new_storage_test_vm("https://custom-state-shadow-scope.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }

              class ShadowStateTarget extends HTMLElement {
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-shadow-state-target", ShadowStateTarget);
              customElements.define("wpt-shadow-state-host", class extends HTMLElement {
                constructor() {
                  super();
                  const shadow = this.attachShadow({ mode: "open" });
                  const style = document.createElement("style");
                  style.textContent = `
                    :host { color: rgb(255, 0, 0); }
                    :host(:has(wpt-shadow-state-target:state(--green))) { color: rgb(0, 0, 255); }
                    wpt-shadow-state-target { color: rgb(255, 0, 0); }
                    wpt-shadow-state-target:state(--green) { color: rgb(0, 0, 255); }
                    ::slotted(wpt-shadow-state-target:state(--green)) { background-color: rgb(0, 128, 0); }
                  `;
                  shadow.appendChild(style);
                  shadow.appendChild(document.createElement("slot"));
                  this.inner = new ShadowStateTarget();
                  shadow.appendChild(this.inner);
                }
              });

              const host = document.createElement("wpt-shadow-state-host");
              const slotted = new ShadowStateTarget();
              host.appendChild(slotted);
              document.body.appendChild(host);
              const inner = host.shadowRoot.querySelector("wpt-shadow-state-target");
              const snapshot = () => [
                getComputedStyle(host).color,
                getComputedStyle(inner).color,
                getComputedStyle(slotted).backgroundColor,
                inner.matches(":state(--green)"),
                slotted.matches(":state(--green)")
              ].join(",");

              const before = snapshot();
              inner.internals.states.add("--green");
              const afterInner = snapshot();
              slotted.internals.states.add("--green");
              const afterSlotted = snapshot();
              return [before, afterInner, afterSlotted].join("|");
            })()
            "##,
        )
        .expect("shadow custom-state scope probe should evaluate");

    assert_eq!(
        result,
        "rgb(255, 0, 0),rgb(255, 0, 0),rgba(0, 0, 0, 0),false,false|rgb(255, 0, 0),rgb(0, 0, 255),rgba(0, 0, 0, 0),true,false|rgb(0, 0, 255),rgb(0, 0, 255),rgb(0, 128, 0),true,true"
    );
}

#[test]
fn form_associated_custom_element_resolves_form_owner_and_listed_collections() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceOwnerElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.history = [];
                }
                formAssociatedCallback(form) {
                  this.history.push(form ? form.id : null);
                }
                get form() {
                  return this.internals.form;
                }
              }
              customElements.define("wpt-face-owner", FaceOwnerElement);

              const container = document.createElement("div");
              const target = document.body || document.documentElement || document;
              target.appendChild(container);
              container.innerHTML = `
                <fieldset id="fs">
                  <form id="f">
                    <input>
                    <wpt-face-owner id="inside"></wpt-face-owner>
                    <select></select>
                  </form>
                </fieldset>
                <wpt-face-owner id="external" form="f"></wpt-face-owner>
              `;

              const form = container.querySelector("#f");
              const fieldset = container.querySelector("#fs");
              const inside = container.querySelector("#inside");
              const external = container.querySelector("#external");
              const controls = form.elements;
              const initialLength = controls.length;
              const initialInside = controls[1] === inside;
              const initialExternal = controls[3] === external;
              external.setAttribute("form", "missing");

              return [
                inside.form === form,
                external.form === null,
                initialLength,
                initialInside,
                initialExternal,
                fieldset.elements[1] === inside,
                inside.history.join(","),
                external.history.join(",")
              ].join("|");
            })()
            "##,
        )
        .expect("form-associated custom element owner probe should evaluate");

    assert_eq!(result, "true|true|4|true|true|true|f|f,");
}

#[test]
fn form_associated_custom_element_participates_in_label_association() {
    let mut vm = new_storage_test_vm("https://face-labels.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceLabelElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.i = this.attachInternals();
                  this.clicks = 0;
                  this.addEventListener("click", () => this.clicks++);
                }
              }
              customElements.define("face-label-element", FaceLabelElement);
              const root = document.body || document.documentElement || document;
              const container = root.appendChild(document.createElement("div"));
              container.innerHTML = '<label for="face"></label><form><face-label-element id="face"></face-label-element></form>';
              const label = container.querySelector("label");
              const control = container.querySelector("face-label-element");
              const labels = control.i.labels;
              label.click();
              return JSON.stringify({
                control: label.control === control,
                labelForm: label.form === control.i.form,
                labelsBrand: labels instanceof NodeList,
                labelsLength: labels.length,
                labelsItem: labels[0] === label,
                clicks: control.clicks
              });
            })()
            "#,
        )
        .expect("form-associated label association probe should evaluate");

    assert_eq!(
        result,
        r#"{"control":true,"labelForm":true,"labelsBrand":true,"labelsLength":1,"labelsItem":true,"clicks":1}"#
    );
}

#[test]
fn form_reset_invokes_form_associated_custom_element_callback() {
    let mut vm = new_storage_test_vm("https://face-reset.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceResetElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.i = this.attachInternals();
                  this.resetCount = 0;
                }
                formResetCallback() {
                  this.resetCount++;
                  this.outputValue = this.output && this.output.value;
                }
              }
              customElements.define("face-reset-element", FaceResetElement);
              const root = document.body || document.documentElement || document;
              const container = root.appendChild(document.createElement("div"));
              container.insertAdjacentHTML("beforeend",
                "<form><face-reset-element></face-reset-element><output>default</output></form>");
              const form = container.lastChild;
              const custom = form.firstChild;
              const output = form.lastChild;
              output.value = "updated";
              custom.output = output;
              form.reset();
              return JSON.stringify({
                form: custom.i.form === form,
                inElements: form.elements[0] === custom,
                resetCount: custom.resetCount,
                outputValue: custom.outputValue
              });
            })()
            "#,
        )
        .expect("form-associated reset callback probe should evaluate");

    assert_eq!(
        result,
        r#"{"form":true,"inElements":true,"resetCount":1,"outputValue":"default"}"#
    );
}

#[test]
fn element_internals_declared_form_methods_preserve_descriptors_and_validity() {
    let mut vm = new_storage_test_vm("https://element-internals-declared-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceValidationElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("wpt-face-validation", FaceValidationElement);
              const element = new FaceValidationElement();
              const internals = element.internals;
              const methods = ["setFormValue", "setValidity", "checkValidity", "reportValidity"];
              const descriptors = methods.map(name => {
                const descriptor = Object.getOwnPropertyDescriptor(ElementInternals.prototype, name);
                return [
                  name,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              });
              const setFormUndefined = internals.setFormValue("value") === undefined;
              const setValidityUndefined = internals.setValidity({ customError: true }, "bad value") === undefined;
              const invalid = [
                internals.validity.valid,
                internals.validity.customError,
                internals.validationMessage,
                internals.checkValidity(),
                internals.reportValidity()
              ].join("|");
              internals.setValidity({});
              const valid = [
                internals.validity.valid,
                internals.validity.customError,
                internals.validationMessage,
                internals.checkValidity(),
                internals.reportValidity()
              ].join("|");
              return JSON.stringify({
                descriptors,
                setFormUndefined,
                setValidityUndefined,
                invalid,
                valid
              });
            })()
            "#,
        )
        .expect("ElementInternals declared method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["setFormValue:function:setFormValue:1:true:true:true","setValidity:function:setValidity:1:true:true:true","checkValidity:function:checkValidity:0:true:true:true","reportValidity:function:reportValidity:0:true:true:true"],"setFormUndefined":true,"setValidityUndefined":true,"invalid":"false|true|bad value|false|false","valid":"true|false||true|true"}"#
    );
}

#[test]
fn form_associated_custom_element_refreshes_later_form_owner() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceLaterFormElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.history = [];
                }
                formAssociatedCallback(form) {
                  this.history.push(form ? form.id : null);
                }
                get form() {
                  return this.internals.form;
                }
              }
              customElements.define("wpt-face-later-form", FaceLaterFormElement);

              const container = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(container);
              container.innerHTML = `
                <fieldset id="fs">
                  <wpt-face-later-form id="inside" form="target"></wpt-face-later-form>
                  <form id="target"><input></form>
                </fieldset>
                <wpt-face-later-form id="outside" form="target"></wpt-face-later-form>
              `;

              const form = container.querySelector("#target");
              const inside = container.querySelector("#inside");
              const outside = container.querySelector("#outside");
              const fieldset = container.querySelector("#fs");
              const initial = [
                inside.form === form,
                outside.form === form,
                form.elements[0] === inside,
                form.elements[2] === outside,
                fieldset.elements[0] === inside,
                inside.history.join(","),
                outside.history.join(",")
              ].join("|");

              form.remove();
              const removed = [
                inside.form === null,
                outside.form === null,
                inside.history.join(","),
                outside.history.join(",")
              ].join("|");

              container.appendChild(form);
              const reinserted = [
                inside.form === form,
                outside.form === form,
                inside.history.join(","),
                outside.history.join(",")
              ].join("|");

              return `${initial}#${removed}#${reinserted}`;
            })()
            "##,
        )
        .expect("later form-associated custom element owner probe should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|true|target|target#true|true|target,|target,#true|true|target,,target|target,,target"
    );
}

#[test]
fn parser_form_pointer_associates_builtin_controls_not_face() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        r##"
        <!doctype html>
        <body>
        <table>
          <fieldset id="fs">
            <form id="f">
              <tr><td><select id="select"></select></tr>
              <tr><td><wpt-face-parser-form id="face"></wpt-face-parser-form></tr>
              <tr><td><input id="input"></tr>
            </form>
          </fieldset>
        </table>
        </body>
        "##,
    );

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceParserFormElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.history = [];
                }
                formAssociatedCallback(form) {
                  this.history.push(form ? form.id : null);
                }
                get form() {
                  return this.internals.form;
                }
              }
              customElements.define("wpt-face-parser-form", FaceParserFormElement);

              customElements.upgrade(document.body);
              const form = document.querySelector("#f");
              const face = document.querySelector("#face");
              const controls = form.elements;
              return [
                controls.length,
                controls[0] && controls[0].id,
                controls[1] && controls[1].id,
                face.form === null,
                face.history.join(","),
                document.querySelector("#fs").elements.length
              ].join("|");
            })()
            "##,
        )
        .expect("parser form-pointer association probe should evaluate");

    assert_eq!(result, "2|select|input|true||0");
}

#[test]
fn parser_created_form_associated_custom_element_runs_initial_form_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class ParserDefinedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals_ = this.attachInternals();
                  this.formHistory_ = [];
                }
                formAssociatedCallback(form) {
                  this.formHistory_.push(form ? form.id : null);
                }
                get form() { return this.internals_.form; }
                formHistory() { return this.formHistory_; }
              }
              customElements.define('parser-defined-face', ParserDefinedFace);
              const container = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(container);
              container.innerHTML = `
                <fieldset id="fs1">
                  <form id="form1">
                    <input>
                    <parser-defined-face id="pd1"></parser-defined-face>
                    <select></select>
                  </form>
                </fieldset>
                <fieldset id="fs2">
                  <parser-defined-face id="pd2" form="form2"></parser-defined-face>
                  <form id="form2">
                    <input>
                    <select></select>
                  </form>
                </fieldset>
                <parser-defined-face id="pd3" form="form2"></parser-defined-face>
              `;
              const ids = ["pd1", "pd2", "pd3"];
              return ids.map(id => {
                const element = document.getElementById(id);
                return `${id}:${element.form && element.form.id}:${element.formHistory().map(form => form && form.id || form).join(",")}`;
              }).join("|");
            })()
            "##,
        )
        .expect("parser-created FACE initial form callback probe should evaluate");

    assert_eq!(result, "pd1:form1:form1|pd2:form2:form2|pd3:form2:form2");
}

#[test]
fn custom_element_constructor_error_reports_to_definition_window() {
    let mut vm = new_storage_test_vm("https://ce-error-realm.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frameA = document.createElement("iframe");
              const frameB = document.createElement("iframe");
              const root = document.documentElement || document.appendChild(document.createElement("html"));
              const body = document.body || root.appendChild(document.createElement("body"));
              body.append(frameA, frameB);
              const w = frameA.contentWindow;
              const w2 = frameB.contentWindow;
              w.eval("self.__repeatedEvalProbe = 1");
              w.eval("self.MyElement = class extends HTMLElement { constructor() { throw new Error('boom'); } }");
              const events = [];
              const listener = event => {
                events.push({
                  targetA: event.target === w,
                  targetB: event.target === w2,
                  hasError: !!event.error,
                  errorIsDefinitionRealm: !!event.error && event.error.constructor === w.Error,
                  errorCtorType: typeof w.Error
                });
                event.preventDefault();
              };
              for (const current of [window, w, w2]) current.addEventListener("error", listener);
              w2.customElements.define("realm-error-element", w.MyElement);
              const element = w2.document.createElement("realm-error-element");
              return JSON.stringify({
                unknown: element instanceof w2.HTMLUnknownElement,
                events
              });
            })()
            "#,
        )
        .expect("custom element constructor error realm probe should evaluate");

    assert_eq!(
        result,
        r#"{"unknown":true,"events":[{"targetA":true,"targetB":false,"hasError":true,"errorIsDefinitionRealm":true,"errorCtorType":"function"}]}"#
    );
}

#[test]
fn failed_existing_upgrade_before_super_keeps_original_prototype() {
    let mut vm = new_storage_test_vm("https://ce-before-super.test/");
    let result = vm.eval(r#"
        (() => {
          window.addEventListener("error", event => event.preventDefault());
          const html = document.documentElement || document.appendChild(document.createElement("html"));
          const body = document.body || html.appendChild(document.createElement("body"));
          const frame = body.appendChild(document.createElement("iframe"));
          const results = [];
          for (const [realm, w] of [["main", window], ["child", frame.contentWindow]]) {
            for (const failure of ["throw", "return-object"]) {
              const doc = w.document;
              const name = `before-super-${failure}`;
              const element = doc.createElement(name);
              element.setAttribute("data-value", "before");
              (doc.body || doc.documentElement || doc).appendChild(element);
              const original = Object.getPrototypeOf(element);
              const log = [];
              class FailedBeforeSuper extends w.HTMLElement {
                constructor() {
                  log.push("constructor");
                  if (failure === "throw") throw new Error("before super");
                  return {};
                }
                static get observedAttributes() { return ["data-value"]; }
                attributeChangedCallback() { log.push("attribute"); }
                connectedCallback() { log.push("connected"); }
              }
              w.customElements.define(name, FailedBeforeSuper);
              results.push({realm, failure,
                unchanged: Object.getPrototypeOf(element) === original,
                custom: element instanceof FailedBeforeSuper, log});
            }
          }
          return JSON.stringify(results);
        })()
    "#).expect("failed upgrades before super should report their original prototypes");
    let results: serde_json::Value = serde_json::from_str(&result).expect("upgrade results");
    let results = results.as_array().expect("four upgrade cases");
    assert_eq!(results.len(), 4);
    for result in results {
        assert_eq!(result["unchanged"], true, "{result}");
        assert_eq!(result["custom"], false, "{result}");
        assert_eq!(
            result["log"],
            serde_json::json!(["constructor"]),
            "{result}"
        );
    }
}

#[test]
fn failed_existing_upgrade_preserves_definition_prototype_and_clears_reactions() {
    let mut vm = new_storage_test_vm("https://ce-failed-upgrade.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              window.addEventListener("error", event => event.preventDefault());
              const root = document.body || document.documentElement || document;
              const frame = root.appendChild(document.createElement("iframe"));
              const childWindow = frame.contentWindow;
              const childDocument = childWindow.document;
              childDocument.write('<failed-upgrade-probe id="some" class="foo"></failed-upgrade-probe>');
              const element = childDocument.querySelector("failed-upgrade-probe");
              const before = Object.getPrototypeOf(element) === childWindow.HTMLElement.prototype;
              const log = [];
              class FailedUpgradeProbe extends childWindow.HTMLElement {
                constructor() {
                  super();
                  log.push("constructor");
                  throw new Error("boom");
                }
                connectedCallback() {
                  log.push("connected");
                }
                attributeChangedCallback() {
                  log.push("attribute");
                }
                static get observedAttributes() { return ["id", "class"]; }
              }
              childWindow.customElements.define("failed-upgrade-probe", FailedUpgradeProbe);
              return JSON.stringify({
                before,
                after: Object.getPrototypeOf(element) === FailedUpgradeProbe.prototype,
                log
              });
            })()
            "#,
        )
        .expect("failed existing custom element upgrade probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":true,"after":true,"log":["constructor"]}"#
    );
}

#[test]
fn form_associated_custom_element_disabled_state_and_form_value() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceDisabledElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                  this.log = [];
                  this.internals.setFormValue("face-value");
                }
                formDisabledCallback(disabled) {
                  this.log.push(disabled);
                }
              }
              customElements.define("wpt-face-disabled", FaceDisabledElement);

              const form = document.createElement("form");
              form.innerHTML = `
                <fieldset id="fs">
                  <legend><wpt-face-disabled id="legend" name="legend" tabindex="0"></wpt-face-disabled></legend>
                  <wpt-face-disabled id="inside" name="inside" tabindex="0"></wpt-face-disabled>
                </fieldset>
                <wpt-face-disabled id="outside" name="outside" tabindex="0"></wpt-face-disabled>
              `;
              (document.body || document.documentElement || document).appendChild(form);

              const fs = form.querySelector("#fs");
              const legend = form.querySelector("#legend");
              const inside = form.querySelector("#inside");
              const outside = form.querySelector("#outside");
              const formEntries = () => Array.from(new FormData(form))
                .map(([name, value]) => `${name}=${value}`);

              const initial = {
                outsideEnabled: outside.matches(":enabled"),
                outsideDisabled: outside.matches(":disabled"),
                insideEnabled: inside.matches(":enabled"),
                entries: formEntries()
              };

              outside.setAttribute("disabled", "");
              outside.focus();
              const ownDisabled = {
                log: outside.log.slice(),
                disabled: outside.matches(":disabled"),
                omitted: new FormData(form).get("outside") === null,
                focused: document.activeElement === outside
              };

              outside.removeAttribute("disabled");
              outside.focus();
              const ownEnabled = {
                log: outside.log.slice(),
                enabled: outside.matches(":enabled"),
                value: new FormData(form).get("outside"),
                focused: document.activeElement === outside
              };

              fs.setAttribute("disabled", "");
              const fieldsetDisabled = {
                insideLog: inside.log.slice(),
                legendLog: legend.log.slice(),
                insideDisabled: inside.matches(":disabled"),
                legendEnabled: legend.matches(":enabled"),
                insideOmitted: new FormData(form).get("inside") === null,
                legendValue: new FormData(form).get("legend")
              };

              inside.setAttribute("disabled", "");
              inside.removeAttribute("disabled");
              const dedupedInsideLog = inside.log.slice();

              fs.removeAttribute("disabled");
              const fieldsetEnabled = {
                insideLog: inside.log.slice(),
                insideEnabled: inside.matches(":enabled"),
                entries: formEntries()
              };

              const detachedContainer = document.createElement("fieldset");
              detachedContainer.innerHTML = "<fieldset><fieldset><wpt-face-disabled></wpt-face-disabled></fieldset></fieldset>";
              const detachedMiddleFieldset = detachedContainer.firstChild;
              const detachedControl = detachedContainer.querySelector("wpt-face-disabled");
              detachedMiddleFieldset.disabled = true;
              detachedMiddleFieldset.disabled = false;
              detachedContainer.disabled = true;
              detachedControl.remove();
              detachedMiddleFieldset.appendChild(detachedControl);
              const detachedRelationship = {
                log: detachedControl.log.slice(),
                disabled: detachedControl.matches(":disabled")
              };

              return JSON.stringify({
                initial,
                ownDisabled,
                ownEnabled,
                fieldsetDisabled,
                dedupedInsideLog,
                fieldsetEnabled,
                detachedRelationship
              });
            })()
            "##,
        )
        .expect("form-associated custom element disabled-state probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":{"outsideEnabled":true,"outsideDisabled":false,"insideEnabled":true,"entries":["legend=face-value","inside=face-value","outside=face-value"]},"ownDisabled":{"log":[true],"disabled":true,"omitted":true,"focused":false},"ownEnabled":{"log":[true,false],"enabled":true,"value":"face-value","focused":true},"fieldsetDisabled":{"insideLog":[true],"legendLog":[],"insideDisabled":true,"legendEnabled":true,"insideOmitted":true,"legendValue":"face-value"},"dedupedInsideLog":[true],"fieldsetEnabled":{"insideLog":[true,false],"insideEnabled":true,"entries":["legend=face-value","inside=face-value","outside=face-value"]},"detachedRelationship":{"log":[true,false,true,false,true],"disabled":true}}"#
    );
}

#[test]
fn form_associated_custom_element_form_data_value_preserves_entry_list_snapshot() {
    let mut vm = new_storage_test_vm("https://face-form-data.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class FaceFormDataElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.internals = this.attachInternals();
                }
              }
              customElements.define("face-form-data-element", FaceFormDataElement);

              const form = document.createElement("form");
              const control = document.createElement("face-form-data-element");
              control.name = "ignored-owner-name";
              const supplied = new FormData();
              supplied.append("line", "alpha");
              supplied.append("line", "beta");
              supplied.append("meta", "first");
              control.internals.setFormValue(supplied);
              supplied.append("line", "late-mutation");
              supplied.delete("meta");
              form.append(control);
              (document.body || document.documentElement || document).append(form);

              return JSON.stringify(Array.from(new FormData(form)));
            })()
            "#,
        )
        .expect("form-associated FormData entry-list snapshot should evaluate");

    assert_eq!(
        result,
        r#"[["line","alpha"],["line","beta"],["meta","first"]]"#
    );
}

#[test]
fn form_associated_callbacks_distinguish_detached_insertion_from_connected_upgrade() {
    let mut vm = new_storage_test_vm("https://face-reaction-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const insertionHistory = [];
              class InsertedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.attachInternals();
                }
                connectedCallback() { insertionHistory.push("connected"); }
                formAssociatedCallback(form) {
                  insertionHistory.push(`form:${form?.id ?? "null"}`);
                }
              }
              customElements.define("inserted-reaction-face", InsertedFace);
              const insertedForm = document.createElement("form");
              insertedForm.id = "inserted-form";
              insertedForm.append(document.createElement("inserted-reaction-face"));
              const afterDetachedInsertion = insertionHistory.slice();
              (document.body || document.documentElement || document).append(insertedForm);

              const upgradeHistory = [];
              const upgradeForm = document.createElement("form");
              upgradeForm.id = "upgrade-form";
              upgradeForm.innerHTML = "<upgraded-reaction-face></upgraded-reaction-face>";
              (document.body || document.documentElement || document).append(upgradeForm);
              class UpgradedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.attachInternals();
                }
                connectedCallback() { upgradeHistory.push("connected"); }
                formAssociatedCallback(form) {
                  upgradeHistory.push(`form:${form?.id ?? "null"}`);
                }
              }
              customElements.define("upgraded-reaction-face", UpgradedFace);

              return JSON.stringify({
                afterDetachedInsertion,
                insertionHistory,
                upgradeHistory
              });
            })()
            "#,
        )
        .expect("FACE connection reaction order probe should evaluate");

    assert_eq!(
        result,
        r#"{"afterDetachedInsertion":["form:inserted-form"],"insertionHistory":["form:inserted-form","connected"],"upgradeHistory":["connected","form:upgrade-form"]}"#
    );
}

#[test]
fn document_adopt_node_defers_disconnected_reaction_until_owner_retarget() {
    let mut vm = new_storage_test_vm("https://face-adoption-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              class AdoptedFace extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.attachInternals();
                  this.history = [];
                }
                connectedCallback() {
                  this.history.push(`connected:${this.ownerDocument === document ? "main" : "other"}`);
                }
                disconnectedCallback() {
                  this.history.push(`disconnected:${this.ownerDocument === document ? "main" : "other"}`);
                }
                adoptedCallback(oldDocument, newDocument) {
                  this.history.push(`adopted:${oldDocument === document ? "main" : "other"}->${newDocument === document ? "main" : "other"}`);
                }
                formAssociatedCallback(form) {
                  this.history.push(`form:${form?.id ?? "null"}`);
                }
              }
              customElements.define("adopted-reaction-face", AdoptedFace);

              const first = document.createElement("form");
              first.id = "first";
              const face = document.createElement("adopted-reaction-face");
              first.append(face);
              (document.body || document.documentElement || document).append(first);
              const other = document.implementation.createHTMLDocument("other");
              other.adoptNode(face);
              other.body.append(face);
              document.adoptNode(face);
              const second = document.createElement("form");
              second.id = "second";
              second.append(face);
              (document.body || document.documentElement || document).append(second);
              return face.history.join("|");
            })()
            "#,
        )
        .expect("adoptNode reaction ordering probe should evaluate");

    assert_eq!(
        result,
        "form:first|connected:main|disconnected:other|form:null|adopted:main->other|connected:other|disconnected:main|adopted:other->main|form:second|connected:main"
    );
}

#[test]
fn form_associated_custom_element_validation_participates_in_forms() {
    let mut vm = new_storage_html_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              class FaceValidityElement extends HTMLElement {
                static formAssociated = true;
                constructor() {
                  super();
                  this.i = this.attachInternals();
                }
              }
              customElements.define("wpt-face-validity", FaceValidityElement);

              const probe = fn => {
                try {
                  fn();
                  return "ok";
                } catch (error) {
                  return `${error.name}:${error.code || 0}`;
                }
              };

              const control = document.createElement("wpt-face-validity");
              const root = document.body || document.documentElement || document;
              root.appendChild(control);
              const missingMessage = probe(() => control.i.setValidity({ valueMissing: true }));
              control.i.setValidity({ valueMissing: true }, "missing");
              const invalidEvents = [];
              control.addEventListener("invalid", event => {
                invalidEvents.push(`${event.cancelable}:${event.target === control}`);
              });
              const directCheck = control.i.checkValidity();
              const directReport = control.i.reportValidity();

              const outside = document.createElement("div");
              root.appendChild(outside);
              const outsideAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", outside);
              });
              const foreign = document.createElementNS("urn:not-html", "foo");
              control.appendChild(foreign);
              const foreignAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", foreign);
              });
              foreign.remove();
              const lightChild = document.createElement("span");
              control.appendChild(lightChild);
              const lightAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", lightChild);
              });
              const shadow = control.attachShadow({ mode: "open" });
              const shadowChild = document.createElement("span");
              shadow.appendChild(shadowChild);
              const shadowAnchor = probe(() => {
                control.i.setValidity({ valueMissing: true }, "x", shadowChild);
              });
              control.remove();

              const container = document.createElement("div");
              container.innerHTML = `
                <form>
                  <fieldset>
                    <wpt-face-validity></wpt-face-validity>
                    <input type="submit">
                  </fieldset>
                </form>
              `;
              root.appendChild(container);
              const form = container.querySelector("form");
              const fieldset = container.querySelector("fieldset");
              const face = container.querySelector("wpt-face-validity");
              let aggregateInvalids = 0;
              face.addEventListener("invalid", () => ++aggregateInvalids);

              const initialCss = `${face.matches(":valid")}/${form.matches(":valid")}/${fieldset.matches(":valid")}`;
              face.i.setValidity({ customError: true }, "bad");
              const invalidCss = `${face.matches(":invalid")}/${form.matches(":invalid")}/${fieldset.matches(":invalid")}`;
              const formCheck = form.checkValidity();
              const formReport = form.reportValidity();
              container.querySelector("input").click();
              const aggregateCount = aggregateInvalids;
              face.remove();
              const detachedCss = `${face.matches(":invalid")}/${form.matches(":valid")}/${fieldset.matches(":valid")}`;
              fieldset.appendChild(face);
              const reattachedCss = `${form.matches(":invalid")}/${fieldset.matches(":invalid")}`;
              face.i.setValidity({});
              const clearedCss = `${face.matches(":valid")}/${form.matches(":valid")}/${fieldset.matches(":valid")}`;

              return JSON.stringify({
                missingMessage,
                directCheck,
                directReport,
                invalidEvents,
                outsideAnchor,
                foreignAnchor,
                lightAnchor,
                shadowAnchor,
                initialCss,
                invalidCss,
                formCheck,
                formReport,
                aggregateCount,
                detachedCss,
                reattachedCss,
                clearedCss
              });
            })()
            "##,
        )
        .expect("form-associated custom element validation probe should evaluate");

    assert_eq!(
        result,
        r#"{"missingMessage":"TypeError:0","directCheck":false,"directReport":false,"invalidEvents":["true:true","true:true"],"outsideAnchor":"NotFoundError:8","foreignAnchor":"TypeError:0","lightAnchor":"ok","shadowAnchor":"ok","initialCss":"true/true/true","invalidCss":"true/true/true","formCheck":false,"formReport":false,"aggregateCount":3,"detachedCss":"true/true/true","reattachedCss":"true/true","clearedCss":"true/true/true"}"#
    );
}

#[test]
fn custom_element_lifecycle_runs_in_detached_document_trees() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class DetachedLifecycleElement extends HTMLElement {
                connectedCallback() { calls.push("connected:" + this.localName); }
                disconnectedCallback() { calls.push("disconnected:" + this.localName); }
              }
              customElements.define("wpt-detached-lifecycle", DetachedLifecycleElement);

              const doc = document.implementation.createHTMLDocument("");
              const direct = document.createElement("wpt-detached-lifecycle");
              doc.documentElement.appendChild(direct);
              doc.documentElement.removeChild(direct);

              const host = doc.createElement("div");
              const shadowRoot = host.attachShadow({ mode: "closed" });
              const shadowChild = document.createElement("wpt-detached-lifecycle");
              shadowRoot.appendChild(shadowChild);
              calls.push("detached:" + calls.length);
              doc.documentElement.appendChild(host);
              doc.documentElement.removeChild(host);

              const template = document.createElement("template");
              const templateDoc = template.content.ownerDocument;
              templateDoc.appendChild(templateDoc.createElement("html"));
              const templateChild = document.createElement("wpt-detached-lifecycle");
              templateDoc.documentElement.appendChild(templateChild);
              templateDoc.documentElement.removeChild(templateChild);

              const clonedDoc = document.cloneNode(false);
              clonedDoc.appendChild(clonedDoc.createElement("html"));
              const clonedChild = document.createElement("wpt-detached-lifecycle");
              clonedDoc.documentElement.appendChild(clonedChild);
              clonedDoc.documentElement.removeChild(clonedChild);

              return calls.join("|");
            })()
            "#,
        )
        .expect("detached document custom element lifecycle probe should evaluate");

    assert_eq!(
        result,
        "connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle|detached:2|connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle|connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle|connected:wpt-detached-lifecycle|disconnected:wpt-detached-lifecycle"
    );
}

#[test]
fn detached_native_attribute_node_mutation_flushes_reactions_after_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              let target;
              let attribute;
              class DetachedAttributeElement extends HTMLElement {
                static get observedAttributes() { return ["data-probe", "class", "id"]; }
                attributeChangedCallback(name, oldValue, newValue) {
                  calls.push([
                    name,
                    oldValue,
                    newValue,
                    attribute.ownerElement === target,
                    this.getAttribute(name)
                  ].join(":"));
                }
              }
              const registry = new CustomElementRegistry();
              registry.define("wpt-detached-attr-reaction", DetachedAttributeElement);

              const doc = document.implementation.createHTMLDocument("");
              target = doc.createElement("wpt-detached-attr-reaction");
              registry.initialize(target);
              attribute = doc.createAttribute("data-probe");
              attribute.value = "one";
              target.setAttributeNode(attribute);
              calls.push("after:" + (attribute.ownerElement === target));
              target.dataset.probe = "two";
              calls.push("after-dataset");
              target.classList.add("ready");
              calls.push("after-class");
              attribute.value = "three";
              calls.push("after-attr-value");
              target.id = "native-id";
              calls.push("after-id");
              return calls.join("|");
            })()
            "#,
        )
        .expect("detached native attribute-node reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "data-probe::one:true:one|after:true|data-probe:one:two:true:two|after-dataset|class::ready:true:ready|after-class|data-probe:two:three:true:three|after-attr-value|id::native-id:true:native-id|after-id"
    );
}

#[test]
fn detached_native_tree_mutation_flushes_reactions_after_operation_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const calls = [];
              const childSummary = (parent) => Array.from(parent.childNodes).map((node) => {
                return node.nodeType === Node.TEXT_NODE ? "#text" : node.id;
              }).join(",");
              class DetachedTreeReactionElement extends HTMLElement {
                connectedCallback() {
                  calls.push([
                    "connected",
                    this.id,
                    this.parentNode && this.parentNode.childNodes.length,
                    childSummary(this.parentNode)
                  ].join(":"));
                }
                disconnectedCallback() {
                  calls.push([
                    "disconnected",
                    this.id,
                    doc.body.childNodes.length,
                    doc.body.firstElementChild && doc.body.firstElementChild.id
                  ].join(":"));
                }
              }
              customElements.define("wpt-detached-tree-reaction", DetachedTreeReactionElement);

              const doc = document.implementation.createHTMLDocument("");
              const first = document.createElement("wpt-detached-tree-reaction");
              first.id = "first";
              const second = document.createElement("wpt-detached-tree-reaction");
              second.id = "second";
              doc.body.append(first, "middle", second);
              calls.push("after-append:" + doc.body.childNodes.length);

              const replacement = document.createElement("wpt-detached-tree-reaction");
              replacement.id = "replacement";
              doc.body.replaceChildren(replacement);
              calls.push(
                "after-replace:" +
                doc.body.childNodes.length +
                ":" +
                doc.body.firstElementChild.id
              );
              return calls.join("|");
            })()
            "##,
        )
        .expect("detached native tree mutation reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "connected:first:3:first,#text,second|connected:second:3:first,#text,second|after-append:3|disconnected:first:1:replacement|disconnected:second:1:replacement|connected:replacement:1:replacement|after-replace:1:replacement"
    );
}

#[test]
fn detached_native_text_content_flushes_reactions_after_replacement_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const calls = [];
              class DetachedTextContentReactionElement extends HTMLElement {
                disconnectedCallback() {
                  calls.push([
                    "disconnected",
                    this.id,
                    doc.body.childNodes.length,
                    doc.body.firstChild && doc.body.firstChild.nodeValue
                  ].join(":"));
                }
              }
              customElements.define(
                "wpt-detached-textcontent-reaction",
                DetachedTextContentReactionElement
              );

              const doc = document.implementation.createHTMLDocument("");
              const first = document.createElement("wpt-detached-textcontent-reaction");
              first.id = "first";
              const second = document.createElement("wpt-detached-textcontent-reaction");
              second.id = "second";
              doc.body.append(first, second);

              doc.body.textContent = "fresh";
              calls.push(
                "after-set:" +
                doc.body.childNodes.length +
                ":" +
                doc.body.firstChild.nodeValue
              );
              return calls.join("|");
            })()
            "##,
        )
        .expect("detached native textContent reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "disconnected:first:1:fresh|disconnected:second:1:fresh|after-set:1:fresh"
    );
}

#[test]
fn detached_native_inner_html_flushes_reactions_after_replacement_state_is_stable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const calls = [];
              class DetachedInnerHTMLReactionElement extends HTMLElement {
                disconnectedCallback() {
                  calls.push([
                    "disconnected",
                    this.id,
                    doc.body.childNodes.length,
                    doc.body.firstElementChild && doc.body.firstElementChild.localName,
                    doc.body.textContent
                  ].join(":"));
                }
              }
              customElements.define(
                "wpt-detached-innerhtml-reaction",
                DetachedInnerHTMLReactionElement
              );

              const doc = document.implementation.createHTMLDocument("");
              const first = document.createElement("wpt-detached-innerhtml-reaction");
              first.id = "first";
              const second = document.createElement("wpt-detached-innerhtml-reaction");
              second.id = "second";
              doc.body.append(first, second);

              doc.body.innerHTML = "<p>fresh</p>";
              calls.push(
                "after-set:" +
                doc.body.childNodes.length +
                ":" +
                doc.body.firstElementChild.localName +
                ":" +
                doc.body.textContent
              );
              return calls.join("|");
            })()
            "##,
        )
        .expect("detached native innerHTML reaction boundary probe should evaluate");

    assert_eq!(
        result,
        "disconnected:first:1:p:fresh|disconnected:second:1:p:fresh|after-set:1:p:fresh"
    );
}

#[test]
fn custom_element_adopted_callback_runs_before_connected_in_new_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const calls = [];
              class AdoptedLifecycleElement extends HTMLElement {
                connectedCallback() {
                  calls.push([
                    "connected",
                    this.ownerDocument === globalThis.__adoptTarget,
                    this.isConnected
                  ].join(":"));
                }
                disconnectedCallback() { calls.push(`disconnected:${this.isConnected}`); }
                adoptedCallback(oldDocument, newDocument) {
                  calls.push([
                    "adopted",
                    oldDocument === document,
                    newDocument === globalThis.__adoptTarget
                  ].join(":"));
                }
              }
              customElements.define("wpt-adopted-lifecycle", AdoptedLifecycleElement);

              const detached = document.implementation.createHTMLDocument("");
              globalThis.__adoptTarget = detached;
              const first = document.createElement("wpt-adopted-lifecycle");
              detached.documentElement.appendChild(first);

              const second = document.createElement("wpt-adopted-lifecycle");
              (document.body || document.documentElement || document).appendChild(second);
              detached.documentElement.appendChild(second);

              const cloned = document.cloneNode(false);
              cloned.appendChild(cloned.createElement("html"));
              globalThis.__adoptTarget = cloned;
              const third = document.createElement("wpt-adopted-lifecycle");
              cloned.documentElement.appendChild(third);

              return calls.join("|");
            })()
            "#,
        )
        .expect("custom element adoptedCallback probe should evaluate");

    assert_eq!(
        result,
        "adopted:true:true|connected:true:true|connected:false:true|disconnected:true|adopted:true:true|connected:true:true|adopted:true:true|connected:true:true"
    );
}

#[test]
fn nested_disconnected_reaction_flushes_pending_connected_before_remove_returns() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.body || document.documentElement || document;
              const logs = [];

              class ParentElement extends HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.remove();
                  logs.push("end");
                }
              }
              customElements.define("wpt-nested-reaction-parent", ParentElement);

              class ChildElement extends HTMLElement {
                connectedCallback() { logs.push("connected"); }
                disconnectedCallback() { logs.push("disconnected"); }
              }
              customElements.define("wpt-nested-reaction-child", ChildElement);

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              target.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("nested disconnected custom element reaction probe should evaluate");

    assert_eq!(result, "begin|connected|disconnected|end");
}

#[test]
fn nested_removal_without_disconnected_callback_leaves_pending_connected_in_outer_queue() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.body || document.documentElement || document;
              const logs = [];

              class ParentElement extends HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.remove();
                  logs.push("end");
                }
              }
              customElements.define("wpt-nested-no-disconnected-parent", ParentElement);

              class ChildElement extends HTMLElement {
                connectedCallback() { logs.push("connected"); }
              }
              customElements.define("wpt-nested-no-disconnected-child", ChildElement);

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              target.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("nested removal without disconnected callback probe should evaluate");

    assert_eq!(result, "begin|end|connected");
}

#[test]
fn nested_observed_attribute_reaction_flushes_pending_connected_before_set_attribute_returns() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.body || document.documentElement || document;
              const logs = [];

              class ParentElement extends HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.setAttribute("title", "foo");
                  logs.push("end");
                }
              }
              customElements.define("wpt-nested-attribute-parent", ParentElement);

              class ChildElement extends HTMLElement {
                static get observedAttributes() { return ["title"]; }
                connectedCallback() { logs.push("connected"); }
                attributeChangedCallback() { logs.push("attributeChanged"); }
              }
              customElements.define("wpt-nested-attribute-child", ChildElement);

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              target.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("nested observed attribute custom element reaction probe should evaluate");

    assert_eq!(result, "begin|connected|attributeChanged|end");
}

#[test]
fn child_window_nested_observed_attribute_reaction_flushes_pending_connected() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const target = document.body || document.documentElement || document;
              target.appendChild(frame);
              const { contentWindow, contentDocument } = frame;
              const logs = [];

              class ParentElement extends contentWindow.HTMLElement {
                connectedCallback() {
                  logs.push("begin");
                  this.firstChild.setAttribute("title", "foo");
                  logs.push("end");
                }
              }
              contentWindow.customElements.define(
                "wpt-child-nested-attribute-parent",
                ParentElement
              );

              class ChildElement extends contentWindow.HTMLElement {
                static get observedAttributes() { return ["title"]; }
                connectedCallback() { logs.push("connected"); }
                attributeChangedCallback() { logs.push("attributeChanged"); }
              }
              contentWindow.customElements.define(
                "wpt-child-nested-attribute-child",
                ChildElement
              );

              const parent = new ParentElement();
              const child = new ChildElement();
              parent.appendChild(child);
              contentDocument.body.appendChild(parent);

              return logs.join("|");
            })()
            "#,
        )
        .expect("child-window nested observed attribute reaction probe should evaluate");

    assert_eq!(result, "begin|connected|attributeChanged|end");
}

#[test]
fn html_fragment_parser_upgrade_reaction_runs_before_descendant_connected_callback() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const logs = [];
              let childConnected = 0;

              class Parenter extends HTMLElement {
                connectedCallback() {
                  logs.push("parent:connected");
                  const child = this.firstChild;
                  this.removeChild(child);
                  logs.push("parent:removed");
                  this.appendChild(child);
                  logs.push("parent:appended");
                }
              }
              customElements.define("wpt-fragment-reaction-parent", Parenter);

              class Child extends HTMLElement {
                connectedCallback() {
                  childConnected++;
                  logs.push(`child:connected:${childConnected}`);
                }
              }
              customElements.define("wpt-fragment-reaction-child", Child);

              const target = document.createElement("section");
              document.appendChild(target);
              target.innerHTML =
                "<wpt-fragment-reaction-parent><wpt-fragment-reaction-child></wpt-fragment-reaction-child></wpt-fragment-reaction-parent>";

              return [
                childConnected,
                logs.join("|"),
                document.querySelector("wpt-fragment-reaction-child") instanceof Child
              ].join(",");
            })()
            "#,
        )
        .expect("HTML fragment custom element upgrade reaction probe should evaluate");

    // The child is still awaiting upgrade when the parent reconnects it. The
    // nested appendChild reaction scope must upgrade it before returning.
    assert_eq!(
        result,
        "1,parent:connected|parent:removed|child:connected:1|parent:appended,true"
    );
}

#[test]
fn html_fragment_parser_upgrade_reaction_survives_move_to_new_document() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              const targetDocument = new Document();
              targetDocument.appendChild(targetDocument.createElement("html"));
              const logs = [];
              let childConnected = 0;

              class Parenter extends HTMLElement {
                connectedCallback() {
                  logs.push("parent:connected");
                  const child = this.firstChild;
                  this.removeChild(child);
                  logs.push("parent:removed");
                  this.appendChild(child);
                  logs.push("parent:appended");
                }
              }
              customElements.define("wpt-fragment-move-parent", Parenter);

              class Child extends HTMLElement {
                connectedCallback() {
                  childConnected++;
                  logs.push(`child:connected:${childConnected}`);
                }
              }
              customElements.define("wpt-fragment-move-child", Child);

              document.documentElement.innerHTML =
                "<wpt-fragment-move-parent><wpt-fragment-move-child></wpt-fragment-move-child></wpt-fragment-move-parent>";
              targetDocument.documentElement.appendChild(document.documentElement.firstChild);

              return [childConnected, logs.join("|")].join(",");
            })()
            "#,
        )
        .expect("HTML fragment custom element move probe should evaluate");

    assert_eq!(
        result,
        "1,parent:connected|parent:removed|child:connected:1|parent:appended"
    );
}

#[test]
fn document_element_inner_html_fragment_preserves_html_context_wrappers() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              document.documentElement.innerHTML =
                "<wpt-html-context-parent><wpt-html-context-child></wpt-html-context-child></wpt-html-context-parent>";
              return Array.from(document.documentElement.childNodes)
                .map(node => node.nodeName)
                .join("|");
            })()
            "#,
        )
        .expect("documentElement innerHTML html-context probe should evaluate");

    assert_eq!(result, "HEAD|BODY");
}

#[test]
fn empty_detached_html_inner_html_creates_head_and_body() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const html = document.createElement("html");
              html.innerHTML = "";
              return Array.from(html.childNodes)
                .map(node => node.nodeName)
                .join("|");
            })()
            "#,
        )
        .expect("empty detached HTML innerHTML probe should evaluate");

    assert_eq!(result, "HEAD|BODY");
}

#[test]
fn child_window_text_replacement_setters_dispatch_disconnected_callbacks() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const target = document.body || document.documentElement || document;
              target.appendChild(frame);
              const { contentWindow, contentDocument } = frame;
              const logs = [];

              class ChildElement extends contentWindow.HTMLElement {
                connectedCallback() { logs.push("connected"); }
                disconnectedCallback() { logs.push("disconnected"); }
              }
              contentWindow.customElements.define(
                "wpt-child-text-replacement-child",
                ChildElement
              );

              const probe = (html, selector, setup, apply) => {
                logs.length = 0;
                contentDocument.body.innerHTML = html;
                const element = contentDocument.querySelector(selector);
                setup(element);
                const before = element.innerHTML;
                const initialLog = logs.join("|");
                logs.length = 0;
                apply(element);
                return [
                  before,
                  element.innerHTML,
                  initialLog,
                  logs.join("|")
                ].join(",");
              };

              return [
                probe(
                  "<a><wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child></a>",
                  "a",
                  () => {},
                  element => { element.text = "world"; }
                ),
                probe(
                  "<select><option></option></select>",
                  "option",
                  element => {
                    const child = contentDocument.createElement("wpt-child-text-replacement-child");
                    element.appendChild(child);
                    child.textContent = "hello";
                  },
                  element => { element.text = "world"; }
                ),
                probe(
                  "<output><wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child></output>",
                  "output",
                  () => {},
                  element => { element.value = "world"; }
                ),
                probe(
                  "<output><wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child></output>",
                  "output",
                  () => {},
                  element => { element.defaultValue = "world"; }
                )
              ].join(";");
            })()
            "#,
        )
        .expect("child window text replacement setter probe should evaluate");

    assert_eq!(
        result,
        "<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected;<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected;<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected;<wpt-child-text-replacement-child>hello</wpt-child-text-replacement-child>,world,connected,disconnected"
    );
}

#[test]
fn custom_elements_when_defined_resolves_with_constructor() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              class PendingElement extends HTMLElement {}
              globalThis.__pendingElementCtor = PendingElement;
              globalThis.__whenDefinedResolved = null;
              customElements.whenDefined("wpt-script-vm-pending").then((value) => {
                globalThis.__whenDefinedResolved = value;
              });
              customElements.define("wpt-script-vm-pending", PendingElement);
              return "setup";
            })()
            "#,
    )
    .expect("customElements.whenDefined setup should evaluate");

    let result = vm
        .eval(
            r#"
            (() => {
              return [
                globalThis.__whenDefinedResolved === globalThis.__pendingElementCtor,
                customElements.get("wpt-script-vm-pending") === globalThis.__pendingElementCtor
              ].join("|");
            })()
            "#,
        )
        .expect("customElements.whenDefined should resolve with the registered constructor");

    assert_eq!(result, "true|true");
}

#[test]
fn detached_template_shadow_root_reflection_setters_update_native_attributes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const template = doc.createElement("template");
              const prototypeSetter = (name, value) => {
                Object
                  .getOwnPropertyDescriptor(HTMLTemplateElement.prototype, name)
                  .set.call(template, value);
              };

              template.shadowRootMode = "closed";
              template.shadowRootSlotAssignment = "manual";
              template.shadowRootAdoptedStyleSheets = "sheet-1";
              template.shadowRootDelegatesFocus = true;
              template.shadowRootClonable = true;
              template.shadowRootSerializable = true;
              template.shadowRootCustomElementRegistry = "first";
              const direct = [
                template.shadowRootMode,
                template.getAttribute("shadowrootmode"),
                template.shadowRootSlotAssignment,
                template.getAttribute("shadowrootslotassignment"),
                template.shadowRootAdoptedStyleSheets,
                template.getAttribute("shadowrootadoptedstylesheets"),
                template.shadowRootDelegatesFocus,
                template.hasAttribute("shadowrootdelegatesfocus"),
                template.shadowRootClonable,
                template.hasAttribute("shadowrootclonable"),
                template.shadowRootSerializable,
                template.hasAttribute("shadowrootserializable"),
                template.shadowRootCustomElementRegistry,
                template.getAttribute("shadowrootcustomelementregistry")
              ].join(":");

              prototypeSetter("shadowRootMode", "open");
              prototypeSetter("shadowRootSlotAssignment", "named");
              prototypeSetter("shadowRootAdoptedStyleSheets", "sheet-2");
              prototypeSetter("shadowRootDelegatesFocus", false);
              prototypeSetter("shadowRootClonable", false);
              prototypeSetter("shadowRootSerializable", false);
              prototypeSetter("shadowRootCustomElementRegistry", "second");
              const prototype = [
                template.shadowRootMode,
                template.getAttribute("shadowrootmode"),
                template.shadowRootSlotAssignment,
                template.getAttribute("shadowrootslotassignment"),
                template.shadowRootAdoptedStyleSheets,
                template.getAttribute("shadowrootadoptedstylesheets"),
                template.shadowRootDelegatesFocus,
                template.hasAttribute("shadowrootdelegatesfocus"),
                template.shadowRootClonable,
                template.hasAttribute("shadowrootclonable"),
                template.shadowRootSerializable,
                template.hasAttribute("shadowrootserializable"),
                template.shadowRootCustomElementRegistry,
                template.getAttribute("shadowrootcustomelementregistry")
              ].join(":");

              return direct + "|" + prototype;
            })()
            "#,
        )
        .expect("detached template shadowRoot reflection setter probe should evaluate");

    assert_eq!(
        result,
        "closed:closed:manual:manual:sheet-1:sheet-1:true:true:true:true:true:true:first:first|open:open:named:named:sheet-2:sheet-2:false:false:false:false:false:false:second:second"
    );
}

#[test]
fn parser_created_customelementregistry_sets_null_registry_once() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const documentElement = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                documentElement.appendChild(document.createElement("body"));
              const frame = document.createElement("iframe");
              body.appendChild(frame);
              const doc = frame.contentDocument;
              const win = frame.contentWindow;
              win.customElements.define("wpt-null-attr", class extends win.HTMLElement {});

              doc.documentElement.setHTMLUnsafe(
                "<div id='builtin' customelementregistry></div>" +
                "<wpt-null-attr id='defined' customelementregistry></wpt-null-attr>" +
                "<a-b id='candidate' customelementregistry></a-b>"
              );
              const builtin = doc.getElementById("builtin");
              const defined = doc.getElementById("defined");
              const candidate = doc.getElementById("candidate");

              const dynamic = doc.createElement("div");
              dynamic.setAttribute("customelementregistry", "");

              const explicitNull = doc.createElement("div", {
                customElementRegistry: null
              });
              const registryOptionSetterHits = [];
              Object.defineProperty(Object.prototype, "customElementRegistry", {
                configurable: true,
                get() { return undefined; },
                set(value) {
                  const receiverKind = this instanceof win.Element ? "element" : "plain";
                  registryOptionSetterHits.push(receiverKind);
                  Object.defineProperty(this, "customElementRegistry", {
                    configurable: true,
                    enumerable: true,
                    writable: true,
                    value
                  });
                }
              });
              let explicitNullClone;
              let parserClone;
              try {
                explicitNullClone = explicitNull.cloneNode(false);
                parserClone = builtin.cloneNode(true);
              } finally {
                delete Object.prototype.customElementRegistry;
              }

              const container = doc.createElement("div", {
                customElementRegistry: null
              });
              container.innerHTML =
                "<a-b id='nested'><wpt-null-attr id='nested-defined'></wpt-null-attr></a-b>";
              const nested = container.querySelector("#nested");
              const nestedDefined = container.querySelector("#nested-defined");

              return JSON.stringify({
                builtin: builtin.customElementRegistry,
                defined: defined.customElementRegistry,
                candidate: candidate.customElementRegistry,
                dynamicUsesDefault:
                  dynamic.customElementRegistry === win.customElements,
                explicitNull: explicitNull.customElementRegistry,
                explicitNullClone: explicitNullClone.customElementRegistry,
                parserClone: parserClone.customElementRegistry,
                registryOptionSetterHits: registryOptionSetterHits
                  .filter(hit => hit === "plain"),
                nested: nested.customElementRegistry,
                nestedDefined: nestedDefined.customElementRegistry
              });
            })()
            "##,
        )
        .expect("customelementregistry parser attribute probe should evaluate");

    assert_eq!(
        result,
        r#"{"builtin":null,"defined":null,"candidate":null,"dynamicUsesDefault":true,"explicitNull":null,"explicitNullClone":null,"parserClone":null,"registryOptionSetterHits":[],"nested":null,"nestedDefined":null}"#
    );
}

#[test]
fn custom_elements_when_defined_rejects_invalid_name() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              globalThis.__whenDefinedRejected = "pending";
              customElements.whenDefined("notcustom").then(
                () => {
                  globalThis.__whenDefinedRejected = "fulfilled";
                },
                (error) => {
                  globalThis.__whenDefinedRejected = [
                    error && error.name,
                    error instanceof DOMException
                  ].join("|");
                }
              );
              return "setup";
            })()
            "#,
    )
    .expect("customElements.whenDefined invalid-name setup should evaluate");

    let result = vm
        .eval(
            r#"
            (() => globalThis.__whenDefinedRejected)()
            "#,
        )
        .expect("customElements.whenDefined invalid-name rejection should settle");

    assert_eq!(result, "SyntaxError|true");
}

#[tokio::test(flavor = "current_thread")]
async fn child_document_write_custom_element_reaction_queue_wpt_shape() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://custom-element-reaction-queue.test/",
        &loader,
    );

    vm.eval(
        r#"
            (() => {
              globalThis.__reactionQueueResult = "pending";
              new Promise((resolve) => {
                const frame = document.createElement("iframe");
                frame.srcdoc = "";
                frame.onload = () => resolve(frame.contentWindow);
                (document.body || document.documentElement || document).appendChild(frame);
              }).then((contentWindow) => {
                try {
                  const contentDocument = contentWindow.document;
                  contentDocument.write('<test-element id="first-element">');
                  contentDocument.write('<test-element id="second-element">');

                  const element1 = contentDocument.getElementById("first-element");
                  const element2 = contentDocument.getElementById("second-element");
                  const log = [];

                  class TestElement extends contentWindow.HTMLElement {
                    constructor() {
                      super();
                      log.push(`constructed:${this.id}`);
                    }
                    connectedCallback() {
                      log.push(`connected:${this.id}`);
                    }
                    attributeChangedCallback(name, oldValue, newValue, namespace) {
                      log.push([
                        "attribute",
                        this.id,
                        name,
                        oldValue,
                        newValue,
                        this.getAttributeNS(namespace, name)
                      ].join(":"));
                    }
                    static get observedAttributes() { return ["id"]; }
                  }

                  contentWindow.customElements.define("test-element", TestElement);
                  globalThis.__reactionQueueResult = JSON.stringify({
                    element1: element1 && element1.localName,
                    element2: element2 && element2.localName,
                    element1ProtoAfterUpgrade:
                      element1 && Object.getPrototypeOf(element1) === TestElement.prototype,
                    element2ProtoAfterUpgrade:
                      element2 && Object.getPrototypeOf(element2) === TestElement.prototype,
                    log
                  });
                } catch (error) {
                  globalThis.__reactionQueueResult =
                    "throw:" + error.name + ":" + error.message;
                }
              }, (error) => {
                globalThis.__reactionQueueResult =
                  "reject:" + error.name + ":" + error.message;
              });
              return "scheduled";
            })()
            "#,
    )
    .expect("child document.write custom element WPT shape should evaluate");

    let expected = r#"{"element1":"test-element","element2":"test-element","element1ProtoAfterUpgrade":true,"element2ProtoAfterUpgrade":true,"log":["constructed:first-element","attribute:first-element:id::first-element:first-element","connected:first-element","constructed:second-element","attribute:second-element:id::second-element:second-element","connected:second-element"]}"#;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__reactionQueueResult",
        expected,
        "child document.write custom-element reaction queue",
    )
    .await;

    let result = vm
        .eval("globalThis.__reactionQueueResult")
        .expect("child document.write custom element WPT shape result should evaluate");

    assert_eq!(result, expected);
}

#[test]
fn child_parser_checkpoints_once_before_adoption_agency_custom_element_construction() {
    let mut vm = new_storage_test_vm("https://parser-checkpoint.test/");

    vm.eval(
        r##"
            (() => {
              globalThis.__childParserCheckpoint = "pending";
              const frame = document.createElement("iframe");
              frame.srcdoc = `<!doctype html><body><script>
                class ParserCheckpointElement extends HTMLElement {
                  constructor() {
                    super();
                    const nodeLabel = node => node.nodeType === Node.TEXT_NODE
                      ? "#text:" + node.data
                      : node.localName;
                    top.__childParserCheckpoint = JSON.stringify(
                      recordsList.map(records => records.map(record => [
                        nodeLabel(record.target),
                        Array.prototype.map.call(record.addedNodes, nodeLabel).join(",")
                      ]))
                    );
                  }
                }
                customElements.define(
                  "parser-checkpoint-element",
                  ParserCheckpointElement
                );
                const recordsList = [];
                new MutationObserver(records => recordsList.push(records)).observe(
                  document.body,
                  { childList: true, subtree: true }
                );
              </script><b><i>hello</b><parser-checkpoint-element>`;
              (document.body || document.documentElement || document).appendChild(frame);
              return "scheduled";
            })()
            "##,
    )
    .expect("child parser checkpoint setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval("globalThis.__childParserCheckpoint")
        .expect("child parser checkpoint result should evaluate");

    assert_eq!(
        result,
        r##"[[["body","b"],["b","i"],["i","#text:hello"],["body","i"]]]"##
    );
}

#[test]
fn child_document_write_constructs_and_connects_predefined_custom_elements_without_a_body() {
    let mut vm = new_storage_html_test_vm("https://document-write-custom-elements.test/");

    vm.eval(
        r#"
            (() => {
              const target = document.body || document.documentElement || document;
              window.__writeCustomElementFrames = ["write", "writeln"].map(() => {
                const frame = document.createElement("iframe");
                target.appendChild(frame);
                frame.srcdoc = "";
                return frame;
              });
              return "scheduled";
            })()
            "#,
    )
    .expect("child document.write predefined custom element setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const exercise = (frame, method, name) => {
                const childWindow = frame.contentWindow;
                const childDocument = frame.contentDocument;
                const registry = childWindow.customElements;
                let constructorCount = 0;
                let connectedCount = 0;
                let errorName = null;
                childWindow.addEventListener("error", event => {
                  errorName = event.error && event.error.name;
                  event.preventDefault();
                }, { once: true });
                class DefinedElement extends childWindow.HTMLElement {
                  constructor() {
                    super();
                    constructorCount++;
                  }
                  connectedCallback() {
                    connectedCount++;
                  }
                }
                registry.define(name, DefinedElement);
                const definitionBeforeWrite = registry.get(name) === DefinedElement;

                childDocument[method](`<${name}></${name}>`);
                const element = childDocument.querySelector(name);
                return {
                  registryPreserved: childWindow.customElements === registry,
                  definitionBeforeWrite,
                  definitionAfterWrite: registry.get(name) === DefinedElement,
                  constructorCount,
                  connectedCount,
                  errorName,
                  htmlElement: element instanceof childWindow.HTMLElement,
                  customElement: element instanceof DefinedElement,
                  ownerDocument: element.ownerDocument === childDocument
                };
              };

              const [writeFrame, writelnFrame] = window.__writeCustomElementFrames;
              return JSON.stringify({
                write: exercise(writeFrame, "write", "write-defined-element"),
                writeln: exercise(writelnFrame, "writeln", "writeln-defined-element")
              });
            })()
            "#,
        )
        .expect("child document.write predefined custom element result should evaluate");

    assert_eq!(
        result,
        r#"{"write":{"registryPreserved":true,"definitionBeforeWrite":true,"definitionAfterWrite":true,"constructorCount":1,"connectedCount":1,"errorName":null,"htmlElement":true,"customElement":true,"ownerDocument":true},"writeln":{"registryPreserved":true,"definitionBeforeWrite":true,"definitionAfterWrite":true,"constructorCount":1,"connectedCount":1,"errorName":null,"htmlElement":true,"customElement":true,"ownerDocument":true}}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_dynamic_markup_counter_exceptions_use_document_realm() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://dynamic-markup-exception-realm.test/",
        &loader,
    );

    vm.eval(
        r#"
            (() => {
              globalThis.__dynamicMarkupExceptionRealm = "pending";
              new Promise((resolve) => {
                const frame = document.createElement("iframe");
                frame.srcdoc = "";
                frame.onload = () => resolve(frame.contentWindow);
                (document.body || document.documentElement || document).appendChild(frame);
              }).then((childWindow) => {
                const childDocument = childWindow.document;
                childDocument.open();
                const results = [];
                class DynamicMarkupProbe extends childWindow.HTMLElement {
                  constructor() {
                    super();
                    const probe = (label, callback) => {
                      try {
                        callback();
                        results.push([label, "no throw"]);
                      } catch (error) {
                        results.push([
                          label,
                          error.name,
                          error.code,
                          error instanceof childWindow.DOMException,
                          error instanceof DOMException
                        ]);
                      }
                    };
                    probe("open", () => childDocument.open());
                    probe("open-type", () => childDocument.open("text/html"));
                    probe("close", () => childDocument.close());
                    probe("write", () => childDocument.write("<b>write</b>"));
                    probe("writeln", () => childDocument.writeln("<b>writeln</b>"));
                    globalThis.__dynamicMarkupExceptionRealm = JSON.stringify({
                      distinctConstructors: childWindow.DOMException !== DOMException,
                      results
                    });
                  }
                }
                childWindow.customElements.define(
                  "dynamic-markup-probe",
                  DynamicMarkupProbe
                );
                childDocument.write(
                  "<!doctype html><body><dynamic-markup-probe></dynamic-markup-probe>"
                );
                childDocument.close();
              }, (error) => {
                globalThis.__dynamicMarkupExceptionRealm =
                  "reject:" + error.name + ":" + error.message;
              });
              return "scheduled";
            })()
            "#,
    )
    .expect("dynamic markup exception realm setup should evaluate");

    let expected = r#"{"distinctConstructors":true,"results":[["open","InvalidStateError",11,true,false],["open-type","InvalidStateError",11,true,false],["close","InvalidStateError",11,true,false],["write","InvalidStateError",11,true,false],["writeln","InvalidStateError",11,true,false]]}"#;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "globalThis.__dynamicMarkupExceptionRealm",
        expected,
        "child dynamic-markup exception realm",
    )
    .await;

    let result = vm
        .eval("globalThis.__dynamicMarkupExceptionRealm")
        .expect("dynamic markup exception realm result should evaluate");

    assert_eq!(result, expected);
}
