use super::*;

#[test]
fn detached_input_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-input-submit-overrides-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, button];
  const names = ["formAction", "formEnctype", "formMethod", "formTarget", "formNoValidate"];

  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    const value = name === "formNoValidate" ? true : `${name}-value`;
    descriptor.set.call(input, value);
    assert(!Object.prototype.hasOwnProperty.call(input, name), `${name} should stay inherited`);
    assert(typeof descriptor.get.call(input) !== "undefined", `${name} direct getter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached input submitter override receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_input_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-input-reflected-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const input = doc.createElement("input");
  const textarea = doc.createElement("textarea");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, textarea, button];
  const cases = [
    ["accept", "image/png", value => value === "image/png"],
    ["alt", "preview", value => value === "preview"],
    ["disabled", true, value => value === true],
    ["dirName", "field.dir", value => value === "field.dir"],
    ["height", 12, value => value === 12],
    ["maxLength", 10, value => value === 10],
    ["max", "9", value => value === "9"],
    ["minLength", 2, value => value === 2],
    ["min", "1", value => value === "1"],
    ["multiple", true, value => value === true],
    ["pattern", "[a-z]+", value => value === "[a-z]+"],
    ["placeholder", "hint", value => value === "hint"],
    ["readOnly", true, value => value === true],
    ["required", true, value => value === true],
    ["src", "/button.png", value => typeof value === "string" && value.endsWith("/button.png")],
    ["step", "2", value => value === "2"],
    ["width", 20, value => value === 20]
  ];

  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(input, value);
    assert(check(descriptor.get.call(input)), `${name} valid receiver`);
    assert(!own(input, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached input reflected receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_control_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-simple-control-prototypes.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLFieldSetElement.prototype, "disabled", true);
  accessor(HTMLFieldSetElement.prototype, "type", false);
  accessor(HTMLFieldSetElement.prototype, "elements", false);
  accessor(HTMLDataListElement.prototype, "options", false);
  accessor(HTMLLegendElement.prototype, "form", false);
  accessor(HTMLOutputElement.prototype, "type", false);
  for (const name of ["value", "min", "max", "low", "high", "optimum"]) {
    accessor(HTMLMeterElement.prototype, name, true);
  }
  accessor(HTMLProgressElement.prototype, "value", true);
  accessor(HTMLProgressElement.prototype, "max", true);
  accessor(HTMLProgressElement.prototype, "position", false);

  const form = doc.createElement("form");
  const fieldset = doc.createElement("fieldset");
  const legend = doc.createElement("legend");
  const input = doc.createElement("input");
  const datalist = doc.createElement("datalist");
  const option = doc.createElement("option");
  const output = doc.createElement("output");
  const meter = doc.createElement("meter");
  const progress = doc.createElement("progress");
  fieldset.append(legend, input);
  datalist.append(option);
  form.append(fieldset, datalist, output, meter, progress);
  doc.body.append(form);

  const checked = [
    [fieldset, ["disabled", "type", "elements"]],
    [datalist, ["options"]],
    [legend, ["form"]],
    [output, ["type"]],
    [meter, ["value", "min", "max", "low", "high", "optimum"]],
    [progress, ["value", "max", "position"]]
  ];
  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
    }
  }

  fieldset.disabled = true;
  meter.min = 1;
  meter.max = 10;
  meter.low = 2;
  meter.high = 8;
  meter.optimum = 4;
  meter.value = 5;
  progress.max = 10;
  progress.value = 5;

  assert(fieldset.disabled === true && fieldset.hasAttribute("disabled"), "fieldset disabled");
  assert(fieldset.type === "fieldset", "fieldset type");
  assert(fieldset.elements.length === 1 && fieldset.elements[0] === input, "fieldset elements");
  assert(datalist.options.length === 1 && datalist.options[0] === option, "datalist options");
  assert(legend.form === form, "legend form");
  assert(output.type === "output", "output type");
  assert(meter.min === 1 && meter.max === 10 && meter.low === 2, "meter lower values");
  assert(meter.high === 8 && meter.optimum === 4 && meter.value === 5, "meter upper values");
  assert(progress.max === 10 && progress.value === 5 && progress.position === 0.5, "progress values");

  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }

  assert(fieldset.disabled === true, "fieldset disabled after delete");
  assert(fieldset.type === "fieldset", "fieldset type after delete");
  assert(fieldset.elements.length === 1, "fieldset elements after delete");
  assert(datalist.options.length === 1, "datalist options after delete");
  assert(legend.form === form, "legend form after delete");
  assert(output.type === "output", "output type after delete");
  assert(meter.value === 5 && meter.min === 1 && meter.max === 10, "meter after delete");
  assert(progress.value === 5 && progress.max === 10 && progress.position === 0.5, "progress after delete");
  return "ok";
})()
"##,
        )
        .expect("detached simple control owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_control_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-simple-control-receiver-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const form = doc.createElement("form");
  const fieldset = doc.createElement("fieldset");
  const input = doc.createElement("input");
  const meter = doc.createElement("meter");
  const progress = doc.createElement("progress");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  fieldset.append(input);
  form.append(fieldset, meter, progress);
  doc.body.append(form);

  const badReceivers = [{}, text, div, input, doc.createElement("button")];
  const assertGetterRejects = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
    return descriptor;
  };
  const assertSetterRejects = (descriptor, name, value) => {
    assert(typeof descriptor.set === "function", `${name} setter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  };

  const disabled = assertGetterRejects(HTMLFieldSetElement.prototype, "disabled");
  assertSetterRejects(disabled, "disabled", true);
  disabled.set.call(fieldset, true);
  assert(disabled.get.call(fieldset) === true, "fieldset disabled valid receiver");
  assert(!own(fieldset, "disabled"), "fieldset disabled should stay inherited");

  const fieldsetType = assertGetterRejects(HTMLFieldSetElement.prototype, "type");
  assert(fieldsetType.set === undefined, "fieldset type readonly");
  assert(fieldsetType.get.call(fieldset) === "fieldset", "fieldset type valid receiver");

  const elements = assertGetterRejects(HTMLFieldSetElement.prototype, "elements");
  assert(elements.set === undefined, "fieldset elements readonly");
  assert(elements.get.call(fieldset).length === 1, "fieldset elements valid receiver");

  const meterValues = [
    ["min", 1, 1],
    ["max", 10, 10],
    ["low", 2, 2],
    ["high", 8, 8],
    ["optimum", 4, 4],
    ["value", 5, 5]
  ];
  for (const [name, value, expected] of meterValues) {
    const descriptor = assertGetterRejects(HTMLMeterElement.prototype, name);
    assertSetterRejects(descriptor, name, value);
    descriptor.set.call(meter, value);
    assert(descriptor.get.call(meter) === expected, `meter ${name} valid receiver`);
    assert(!own(meter, name), `meter ${name} should stay inherited`);
  }

  const progressValue = assertGetterRejects(HTMLProgressElement.prototype, "value");
  assertSetterRejects(progressValue, "value", 5);
  const progressMax = assertGetterRejects(HTMLProgressElement.prototype, "max");
  assertSetterRejects(progressMax, "max", 10);
  progressMax.set.call(progress, 10);
  progressValue.set.call(progress, 5);
  assert(progressMax.get.call(progress) === 10, "progress max valid receiver");
  assert(progressValue.get.call(progress) === 5, "progress value valid receiver");
  assert(!own(progress, "max") && !own(progress, "value"), "progress writable attrs inherited");

  const position = assertGetterRejects(HTMLProgressElement.prototype, "position");
  assert(position.set === undefined, "progress position readonly");
  assert(position.get.call(progress) === 0.5, "progress position valid receiver");
  assert(!own(progress, "position"), "progress position should stay inherited");

  return "ok";
})()
"#,
        )
        .expect("detached simple control receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_popover_accessor_uses_html_element_prototype() {
    let mut vm = new_storage_test_vm("https://detached-popover-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "popover");
  assert(!!descriptor, "popover descriptor");
  assert(typeof descriptor.get === "function", "popover getter");
  assert(typeof descriptor.set === "function", "popover setter");
  assert(descriptor.enumerable === true, "popover enumerable");
  assert(descriptor.configurable === true, "popover configurable");

  const html = document.implementation.createHTMLDocument("");
  const cases = [
    ["live", document.createElement("div")],
    ["detached", html.createElement("div")]
  ];
  for (const [label, element] of cases) {
    assert(!own(element, "popover"), `${label}.popover should not be own before set`);
    assert(descriptor.get.call(element) === null, `${label}.popover missing`);
    descriptor.set.call(element, "");
    assert(!own(element, "popover"), `${label}.popover should not be own after empty set`);
    assert(element.getAttribute("popover") === "", `${label}.popover empty attr`);
    assert(descriptor.get.call(element) === "auto", `${label}.popover auto`);
    descriptor.set.call(element, "hint");
    assert(element.getAttribute("popover") === "hint", `${label}.popover hint attr`);
    assert(descriptor.get.call(element) === "hint", `${label}.popover hint`);
    descriptor.set.call(element, "invalid");
    assert(element.getAttribute("popover") === "invalid", `${label}.popover invalid attr`);
    assert(descriptor.get.call(element) === "manual", `${label}.popover canonical manual`);
    descriptor.set.call(element, null);
    assert(!element.hasAttribute("popover"), `${label}.popover removed`);
    assert(descriptor.get.call(element) === null, `${label}.popover removed value`);
    assert(delete element.popover, `${label}.popover delete`);
    assert(!own(element, "popover"), `${label}.popover should stay inherited`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached popover prototype accessor should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_popover_methods_use_html_element_prototype_brand_checks() {
    let mut vm = new_storage_test_vm("https://detached-popover-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptorShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.value,
      descriptor.value && descriptor.value.length,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const outcome = (callback) => {
    try {
      const value = callback();
      return `OK:${value === undefined ? "undefined" : String(value)}`;
    } catch (error) {
      return `ERR:${error.name}:${error.code || ""}`;
    }
  };

  const html = document.implementation.createHTMLDocument("");
  const plain = html.createElement("div");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const popover = html.createElement("div");
  popover.setAttribute("popover", "");

  return JSON.stringify({
    shapes: ["showPopover", "hidePopover", "togglePopover"].map(descriptorShape).join("|"),
    own: [
      own(popover, "showPopover"),
      own(popover, "hidePopover"),
      own(popover, "togglePopover")
    ].join(","),
    elementOwn: [
      own(Element.prototype, "showPopover"),
      own(Element.prototype, "hidePopover"),
      own(Element.prototype, "togglePopover")
    ].join(","),
    svgTypes: [
      typeof svg.showPopover,
      typeof svg.hidePopover,
      typeof svg.togglePopover
    ].join(","),
    direct: [
      outcome(() => plain.showPopover()),
      outcome(() => popover.showPopover()),
      outcome(() => popover.hidePopover()),
      outcome(() => popover.togglePopover())
    ].join("|"),
    prototype: [
      outcome(() => HTMLElement.prototype.showPopover.call(plain)),
      outcome(() => HTMLElement.prototype.showPopover.call(popover)),
      outcome(() => HTMLElement.prototype.hidePopover.call(popover)),
      outcome(() => HTMLElement.prototype.togglePopover.call(popover))
    ].join("|")
  });
})()
"#,
        )
        .expect("detached popover method brand checks should evaluate");

    assert_eq!(
        result,
        r#"{"shapes":"true:function:0:true:true|true:function:0:true:true|true:function:0:true:true","own":"false,false,false","elementOwn":"false,false,false","svgTypes":"undefined,undefined,undefined","direct":"ERR:NotSupportedError:9|ERR:InvalidStateError:11|ERR:InvalidStateError:11|ERR:InvalidStateError:11","prototype":"ERR:NotSupportedError:9|ERR:InvalidStateError:11|ERR:InvalidStateError:11|ERR:InvalidStateError:11"}"#
    );
}

#[test]
fn detached_button_and_textarea_accessors_use_owner_prototypes() {
    let mut vm =
        new_storage_test_vm("https://detached-button-textarea-prototypes.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const buttonNames = [
    "disabled",
    "formAction",
    "formEnctype",
    "formMethod",
    "formNoValidate",
    "formTarget",
    "type",
    "commandForElement",
    "popoverTargetElement",
    "popoverTargetAction",
    "interestForElement",
    "value"
  ];
  for (const name of buttonNames) accessor(HTMLButtonElement.prototype, name, true);

  const textareaSetters = [
    "disabled",
    "dirName",
    "maxLength",
    "minLength",
    "required",
    "cols",
    "rows",
    "wrap",
    "placeholder",
    "readOnly",
    "defaultValue",
    "value"
  ];
  for (const name of textareaSetters) accessor(HTMLTextAreaElement.prototype, name, true);
  accessor(HTMLTextAreaElement.prototype, "textLength", false);
  accessor(HTMLTextAreaElement.prototype, "type", false);

  const form = doc.createElement("form");
  const button = doc.createElement("button");
  const target = doc.createElement("div");
  const textarea = doc.createElement("textarea");
  assert(!("required" in button), "button required should not be an IDL property");
  assert(!Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, "required"),
         "HTMLButtonElement.required descriptor should be absent");
  button.setAttribute("required", false);
  assert(button.getAttribute("required") === "false", "button required=false attribute text");
  button.setAttribute("required", true);
  assert(button.getAttribute("required") === "true", "button required=true attribute text");
  target.id = "target";
  form.append(button, textarea);
  doc.body.append(target, form);

  const checked = [
    [button, buttonNames],
    [textarea, [...textareaSetters, "textLength", "type"]]
  ];
  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
    }
  }

  button.disabled = true;
  button.formAction = "/submit";
  button.formEnctype = "text/plain";
  button.formMethod = "post";
  button.formNoValidate = true;
  button.formTarget = "_blank";
  button.type = "reset";
  button.commandForElement = target;
  button.popoverTargetElement = target;
  button.popoverTargetAction = "show";
  button.interestForElement = target;
  button.value = "go";

  textarea.disabled = true;
  textarea.dirName = "comment.dir";
  textarea.maxLength = 12;
  textarea.minLength = 2;
  textarea.required = true;
  textarea.cols = 40;
  textarea.rows = 6;
  textarea.wrap = "hard";
  textarea.placeholder = "hint";
  textarea.readOnly = true;
  textarea.defaultValue = "default";
  textarea.value = "hello";

  assert(button.disabled === true && button.hasAttribute("disabled"), "button disabled");
  assert(button.getAttribute("formaction") === "/submit", "button formAction attribute");
  assert(typeof button.formAction === "string" && button.formAction.length > 0, "button formAction");
  assert(button.formEnctype === "text/plain", "button formEnctype");
  assert(button.formMethod === "post", "button formMethod");
  assert(button.formNoValidate === true, "button formNoValidate");
  assert(button.formTarget === "_blank", "button formTarget");
  assert(button.type === "reset", "button type");
  assert(button.commandForElement === target, "button commandForElement");
  assert(button.popoverTargetElement === target, "button popoverTargetElement");
  assert(button.popoverTargetAction === "show", "button popoverTargetAction");
  assert(button.interestForElement === target, "button interestForElement");
  assert(!("required" in button), "button required should remain absent");
  assert(button.getAttribute("required") === "true", "button required attribute stays textual");
  assert(button.value === "go", "button value");

  assert(textarea.disabled === true, "textarea disabled");
  assert(textarea.dirName === "comment.dir", "textarea dirName");
  assert(textarea.maxLength === 12 && textarea.minLength === 2, "textarea length limits");
  assert(textarea.required === true, "textarea required");
  assert(textarea.cols === 40 && textarea.rows === 6, "textarea dimensions");
  assert(textarea.wrap === "hard", "textarea wrap");
  assert(textarea.placeholder === "hint", "textarea placeholder");
  assert(textarea.readOnly === true, "textarea readOnly");
  assert(textarea.defaultValue === "default", "textarea defaultValue");
  assert(textarea.value === "hello" && textarea.textLength === 5, "textarea value");
  assert(textarea.type === "textarea", "textarea type");

  for (const [element, names] of checked) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }

  assert(button.commandForElement === target, "button commandForElement after delete");
  assert(button.popoverTargetElement === target, "button popoverTargetElement after delete");
  assert(button.popoverTargetAction === "show", "button popoverTargetAction after delete");
  assert(button.interestForElement === target, "button interestForElement after delete");
  assert(textarea.value === "hello" && textarea.textLength === 5, "textarea after delete");
  return "ok";
})()
"##,
        )
        .expect("detached button/textarea owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_button_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-button-submit-overrides-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input];
  const names = ["formAction", "formEnctype", "formMethod", "formTarget", "formNoValidate"];

  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    const value = name === "formNoValidate" ? true : `${name}-value`;
    descriptor.set.call(button, value);
    assert(!Object.prototype.hasOwnProperty.call(button, name), `${name} should stay inherited`);
    assert(typeof descriptor.get.call(button) !== "undefined", `${name} direct getter`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached button submitter override receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_button_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-button-reflected-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input];
  const cases = [
    ["disabled", true, value => value === true],
    ["type", "reset", value => value === "reset"],
    ["value", "go", value => value === "go"]
  ];

  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(button, value);
    assert(check(descriptor.get.call(button)), `${name} valid receiver`);
    assert(!own(button, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached button reflected receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_textarea_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-textarea-reflected-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const textarea = doc.createElement("textarea");
  const input = doc.createElement("input");
  const button = doc.createElement("button");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const badReceivers = [{}, text, div, input, button];
  const cases = [
    ["disabled", true, value => value === true],
    ["required", true, value => value === true],
    ["readOnly", true, value => value === true],
    ["dirName", "posted", value => value === "posted"],
    ["maxLength", 12, value => value === 12],
    ["minLength", 2, value => value === 2],
    ["cols", 12, value => value === 12],
    ["rows", 4, value => value === 4],
    ["wrap", "hard", value => value === "hard"],
    ["placeholder", "enter text", value => value === "enter text"],
    ["defaultValue", "seed", value => value === "seed"],
    ["value", "body", value => value === "body"]
  ];

  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(textarea, value);
    assert(check(descriptor.get.call(textarea)), `${name} valid receiver`);
    assert(!own(textarea, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }

  const readonlyCases = [
    ["textLength", value => value === 4],
    ["type", value => value === "textarea"]
  ];
  for (const [name, check] of readonlyCases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} readonly`);
    assert(check(descriptor.get.call(textarea)), `${name} valid receiver`);
    assert(!own(textarea, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached textarea reflected receiver checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_object_param_and_data_accessors_use_owner_prototypes() {
    let mut vm =
        new_storage_test_vm("https://detached-object-param-data-prototypes.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const object = doc.createElement("object");
  const param = doc.createElement("param");
  const data = doc.createElement("data");
  const div = doc.createElement("div");
  doc.body.append(object, param, data, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const readonlyAccessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} setter absent`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const absent = (prototype, name) => {
    assert(
      Object.getOwnPropertyDescriptor(prototype, name) === undefined,
      `${prototype.constructor.name}.${name} should be absent`
    );
  };

  for (const name of [
    "data",
    "type",
    "archive",
    "code",
    "codeBase",
    "codeType",
    "declare",
    "standby"
  ]) {
    accessor(HTMLObjectElement.prototype, name);
    absent(HTMLElement.prototype, name);
    assert(!own(object, name), `object.${name} should not be own`);
    assert(!(name in div), `div.${name} should be absent`);
  }
  for (const name of ["contentDocument", "contentWindow"]) {
    readonlyAccessor(HTMLObjectElement.prototype, name);
    absent(HTMLElement.prototype, name);
    assert(!own(object, name), `object.${name} should not be own`);
    assert(!(name in div), `div.${name} should be absent`);
    assert(object[name] === null, `detached object.${name} should be null`);
    const getter = Object.getOwnPropertyDescriptor(HTMLObjectElement.prototype, name).get;
    assert(getter.call(object) === null, `borrowed object.${name} getter`);
    for (const receiver of [div, document.createElement("iframe"), {}, null,
                            HTMLObjectElement.prototype,
                            document.createElementNS("http://www.w3.org/2000/svg", "object")]) {
      let rejected = false;
      try { getter.call(receiver); } catch (error) { rejected = error instanceof TypeError; }
      assert(rejected, `${name} must reject an incompatible receiver`);
    }
  }
  for (const name of ["value", "type", "valueType"]) {
    accessor(HTMLParamElement.prototype, name);
    absent(HTMLElement.prototype, name);
    assert(!own(param, name), `param.${name} should not be own`);
    assert(!(name in div), `div.${name} should be absent`);
  }
  accessor(HTMLDataElement.prototype, "value");
  absent(HTMLElement.prototype, "value");
  assert(!own(data, "value"), "data.value should not be own");

  object.data = "https://assets.detached/plugin.bin";
  object.type = "application/x-test";
  object.archive = "archive.jar";
  object.code = "Applet";
  object.codeBase = "https://assets.detached/classes/";
  object.codeType = "application/java";
  object.declare = true;
  object.standby = "Loading";
  assert(object.data === "https://assets.detached/plugin.bin", "object data");
  assert(object.type === "application/x-test", "object type");
  assert(object.archive === "archive.jar", "object archive");
  assert(object.code === "Applet", "object code");
  assert(object.codeBase === "https://assets.detached/classes/", "object codeBase");
  assert(object.codeType === "application/java", "object codeType");
  assert(object.declare === true, "object declare");
  assert(object.hasAttribute("declare"), "object declare attr");
  assert(object.standby === "Loading", "object standby");

  param.value = "param-value";
  param.type = "text/plain";
  param.valueType = "data";
  assert(param.value === "param-value", "param value");
  assert(param.type === "text/plain", "param type");
  assert(param.valueType === "data", "param valueType");

  data.value = "data-value";
  assert(data.value === "data-value", "data value");

  for (const [element, names] of [
    [object, ["data", "type", "archive", "code", "codeBase", "codeType", "declare", "standby"]],
    [param, ["value", "type", "valueType"]],
    [data, ["value"]]
  ]) {
    for (const name of names) {
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }
  assert(object.data === "https://assets.detached/plugin.bin", "object data after delete");
  assert(object.declare === true, "object declare after delete");
  assert(param.valueType === "data", "param valueType after delete");
  assert(data.value === "data-value", "data value after delete");
  return "ok";
})()
"##,
        )
        .expect("detached object/param/data owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_media_quote_mod_time_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-html-media-quote-mod-time.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const html = doc.documentElement;
  const audio = doc.createElement("audio");
  const video = doc.createElement("video");
  const q = doc.createElement("q");
  const blockquote = doc.createElement("blockquote");
  const ins = doc.createElement("ins");
  const del = doc.createElement("del");
  const time = doc.createElement("time");
  const div = doc.createElement("div");
  doc.body.append(audio, video, q, blockquote, ins, del, time, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const absent = (prototype, name) => {
    assert(
      Object.getOwnPropertyDescriptor(prototype, name) === undefined,
      `${prototype.constructor.name}.${name} should be absent`
    );
  };

  accessor(HTMLHtmlElement.prototype, "version");
  accessor(HTMLMediaElement.prototype, "preload");
  accessor(HTMLQuoteElement.prototype, "cite");
  accessor(HTMLModElement.prototype, "cite");
  accessor(HTMLModElement.prototype, "dateTime");
  accessor(HTMLTimeElement.prototype, "dateTime");
  for (const name of ["version", "preload", "cite", "dateTime"]) {
    absent(HTMLElement.prototype, name);
  }
  assert(!own(HTMLAudioElement.prototype, "preload"), "audio should inherit preload");
  assert(!own(HTMLVideoElement.prototype, "preload"), "video should inherit preload");

  for (const [element, names, label] of [
    [html, ["version"], "html"],
    [audio, ["preload"], "audio"],
    [video, ["preload"], "video"],
    [q, ["cite"], "q"],
    [blockquote, ["cite"], "blockquote"],
    [ins, ["cite", "dateTime"], "ins"],
    [del, ["cite", "dateTime"], "del"],
    [time, ["dateTime"], "time"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
  }
  for (const name of ["version", "preload", "cite", "dateTime"]) {
    assert(!(name in div), `div.${name} should be absent`);
  }

  html.version = "4.01";
  audio.preload = "metadata";
  video.preload = "none";
  q.cite = "https://assets.detached/q.html";
  blockquote.cite = "https://assets.detached/quote.html";
  ins.cite = "https://assets.detached/ins.html";
  ins.dateTime = "2026-06-19";
  del.cite = "https://assets.detached/del.html";
  del.dateTime = "2026-06-20";
  time.dateTime = "2026-06-21";

  assert(html.version === "4.01" && html.getAttribute("version") === "4.01", "html version");
  assert(audio.preload === "metadata" && audio.getAttribute("preload") === "metadata", "audio preload");
  assert(video.preload === "none" && video.getAttribute("preload") === "none", "video preload");
  audio.preload = "invalid";
  assert(audio.preload === "auto" && audio.getAttribute("preload") === "invalid", "audio invalid preload");
  assert(q.cite === "https://assets.detached/q.html", "q cite URL");
  assert(blockquote.cite === "https://assets.detached/quote.html", "blockquote cite URL");
  assert(ins.cite === "https://assets.detached/ins.html", "ins cite URL");
  assert(ins.dateTime === "2026-06-19", "ins dateTime");
  assert(del.cite === "https://assets.detached/del.html", "del cite URL");
  assert(del.dateTime === "2026-06-20", "del dateTime");
  assert(time.dateTime === "2026-06-21", "time dateTime");

  for (const [element, names, label] of [
    [html, ["version"], "html"],
    [audio, ["preload"], "audio"],
    [video, ["preload"], "video"],
    [q, ["cite"], "q"],
    [blockquote, ["cite"], "blockquote"],
    [ins, ["cite", "dateTime"], "ins"],
    [del, ["cite", "dateTime"], "del"],
    [time, ["dateTime"], "time"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
  }
  assert(html.version === "4.01", "html version after delete");
  assert(audio.preload === "auto", "audio preload after delete");
  assert(q.cite === "https://assets.detached/q.html", "q cite after delete");
  assert(ins.dateTime === "2026-06-19", "ins dateTime after delete");
  assert(time.dateTime === "2026-06-21", "time dateTime after delete");
  return "ok";
})()
"#,
        )
        .expect("detached HTML/media/quote/mod/time owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_label_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-label-owner-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const optgroup = doc.createElement("optgroup");
  const option = doc.createElement("option");
  const track = doc.createElement("track");
  const div = doc.createElement("div");
  const select = doc.createElement("select");
  option.textContent = "Fallback";
  doc.body.append(optgroup, option, track, div, select);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLOptGroupElement.prototype, "label");
  accessor(HTMLOptionElement.prototype, "label");
  accessor(HTMLTrackElement.prototype, "label");
  assert(!own(HTMLElement.prototype, "label"), "label should not be on HTMLElement.prototype");
  assert(!("label" in div), "div should not expose label");
  assert(!("label" in select), "select should not expose label");

  for (const [element, tag] of [[optgroup, "optgroup"], [option, "option"], [track, "track"]]) {
    assert(!own(element, "label"), `${tag}.label should not be own before set`);
  }
  assert(optgroup.label === "", "optgroup default label");
  assert(option.label === "Fallback", "option label fallback");
  assert(track.label === "", "track default label");

  optgroup.label = "Group";
  option.label = "Explicit";
  track.label = "English";
  assert(optgroup.label === "Group" && optgroup.getAttribute("label") === "Group", "optgroup label");
  assert(option.label === "Explicit" && option.getAttribute("label") === "Explicit", "option label");
  assert(track.label === "English" && track.getAttribute("label") === "English", "track label");

  for (const [element, tag] of [[optgroup, "optgroup"], [option, "option"], [track, "track"]]) {
    assert(!own(element, "label"), `${tag}.label should not be own after set`);
    assert(delete element.label, `${tag}.label delete`);
    assert(!own(element, "label"), `${tag}.label should stay inherited`);
  }
  assert(optgroup.label === "Group", "optgroup label after delete");
  assert(option.label === "Explicit", "option label after delete");
  assert(track.label === "English", "track label after delete");
  option.removeAttribute("label");
  assert(option.label === "Fallback", "option label fallback after attribute removal");
  return "ok";
})()
"#,
        )
        .expect("detached label owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_structural_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-simple-structural-owner-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name, length) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.value.length === length, `${name} length`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const prototypeCases = [
    [HTMLLIElement.prototype, ["value"]],
    [HTMLOListElement.prototype, ["start", "reversed", "type"]],
    [HTMLOptGroupElement.prototype, ["disabled"]],
    [HTMLDetailsElement.prototype, ["open"]],
    [HTMLDialogElement.prototype, ["open", "returnValue"]],
    [HTMLMetaElement.prototype, ["content", "httpEquiv"]],
    [HTMLTitleElement.prototype, ["text"]]
  ];
  for (const [prototype, names] of prototypeCases) {
    for (const name of names) {
      accessor(prototype, name);
      assert(!own(HTMLElement.prototype, name), `${name} should not live on HTMLElement`);
    }
  }
  method(HTMLDialogElement.prototype, "show", 0);
  method(HTMLDialogElement.prototype, "showModal", 0);
  method(HTMLDialogElement.prototype, "close", 1);

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [doc, label] of [[document, "live"], [detachedDoc, "detached"]]) {
    const li = doc.createElement("li");
    const ol = doc.createElement("ol");
    const optgroup = doc.createElement("optgroup");
    const details = doc.createElement("details");
    const dialog = doc.createElement("dialog");
    const meta = doc.createElement("meta");
    const title = doc.createElement("title");

    const cases = [
      [li, "value", 7, "7", "value"],
      [ol, "start", 3, "3", "start"],
      [ol, "reversed", true, "", "reversed"],
      [ol, "type", "A", "A", "type"],
      [optgroup, "disabled", true, "", "disabled"],
      [details, "open", true, "", "open"],
      [dialog, "open", true, "", "open"],
      [meta, "content", "width=device-width", "width=device-width", "content"],
      [meta, "httpEquiv", "refresh", "refresh", "http-equiv"],
      [title, "text", "Page Title", "Page Title", null]
    ];

    for (const [element, name, value, expected, attribute] of cases) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
      element[name] = value;
      assert(element[name] === value || element[name] === expected, `${label}.${name} getter`);
      if (attribute === null) {
        assert(element.textContent === expected, `${label}.${name} text content`);
      } else {
        assert(element.getAttribute(attribute) === expected, `${label}.${name} attribute`);
      }
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
      assert(element[name] === value || element[name] === expected, `${label}.${name} after delete`);
    }

    assert(!own(dialog, "returnValue"), `${label}.returnValue should not be own before set`);
    dialog.returnValue = "done";
    assert(dialog.returnValue === "done", `${label}.returnValue getter`);
    assert(dialog.getAttribute("returnvalue") === null, `${label}.returnValue must not reflect`);
    assert(!own(dialog, "returnValue"), `${label}.returnValue should not be own after set`);
    assert(delete dialog.returnValue, `${label}.returnValue delete`);
    assert(dialog.returnValue === "done", `${label}.returnValue after delete`);

    for (const name of ["show", "showModal", "close"]) {
      assert(!own(dialog, name), `${label}.dialog.${name} should not be own`);
    }
    assert(!Object.prototype.hasOwnProperty.call(dialog, "__moliDialogHandle"), `${label}.dialog private handle should not be own`);
    dialog.show();
    assert(dialog.open === true, `${label}.dialog show behavior`);
    dialog.close("closed");
    assert(dialog.open === false && dialog.returnValue === "closed", `${label}.dialog close behavior`);
    let showModalError;
    try {
      dialog.showModal();
    } catch (error) {
      showModalError = error;
    }
    assert(showModalError instanceof DOMException, `${label}.dialog showModal DOMException`);
    assert(showModalError.name === "InvalidStateError", `${label}.dialog showModal error name`);
    assert(dialog.open === false, `${label}.dialog showModal keeps disconnected dialog closed`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached simple structural owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_simple_specialized_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-simple-specialized-receiver-brand.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };

  const doc = document.implementation.createHTMLDocument("");
  const li = doc.createElement("li");
  const ol = doc.createElement("ol");
  const optgroup = doc.createElement("optgroup");
  const details = doc.createElement("details");
  const meta = doc.createElement("meta");
  const title = doc.createElement("title");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  const elements = [li, ol, optgroup, details, meta, title, div];

  const cases = [
    [HTMLLIElement.prototype, "value", li, 7],
    [HTMLOListElement.prototype, "start", ol, 3],
    [HTMLOListElement.prototype, "reversed", ol, true],
    [HTMLOListElement.prototype, "type", ol, "A"],
    [HTMLOptGroupElement.prototype, "disabled", optgroup, true],
    [HTMLDetailsElement.prototype, "open", details, true],
    [HTMLMetaElement.prototype, "content", meta, "width=device-width"],
    [HTMLMetaElement.prototype, "httpEquiv", meta, "refresh"],
    [HTMLTitleElement.prototype, "text", title, "Page Title"]
  ];

  for (const [prototype, name, element, value] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(typeof descriptor.get.call(element) !== "undefined", `${name} valid getter`);
    descriptor.set.call(element, value);
    assert(!own(element, name), `${name} should stay inherited`);

    for (const receiver of [{}, text, ...elements.filter(candidate => candidate !== element)]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached simple specialized receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_frame_legacy_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-frame-legacy-prototypes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const frame = doc.createElement("frame");
  const iframe = doc.createElement("iframe");
  const img = doc.createElement("img");
  const div = doc.createElement("div");
  doc.body.append(frame, iframe, img, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const frameNames = ["scrolling", "frameBorder", "longDesc", "marginHeight", "marginWidth"];
  for (const name of frameNames) {
    accessor(HTMLFrameElement.prototype, name);
    accessor(HTMLIFrameElement.prototype, name);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }
  accessor(HTMLImageElement.prototype, "longDesc");
  for (const name of ["scrolling", "frameBorder", "marginHeight", "marginWidth"]) {
    assert(!(name in img), `${name} should not be on img`);
  }

  for (const [element, label] of [[frame, "frame"], [iframe, "iframe"]]) {
    for (const name of frameNames) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
    element.scrolling = `${label}-scroll`;
    element.frameBorder = `${label}-border`;
    element.longDesc = `https://assets.example/${label}-desc`;
    element.marginHeight = null;
    element.marginWidth = `${label}-width`;
    assert(element.scrolling === `${label}-scroll`, `${label} scrolling`);
    assert(element.getAttribute("scrolling") === `${label}-scroll`, `${label} scrolling attr`);
    assert(element.frameBorder === `${label}-border`, `${label} frameBorder`);
    assert(element.getAttribute("frameborder") === `${label}-border`, `${label} frameBorder attr`);
    assert(element.longDesc === `https://assets.example/${label}-desc`, `${label} longDesc`);
    assert(element.getAttribute("longdesc") === `https://assets.example/${label}-desc`, `${label} longDesc attr`);
    assert(element.marginHeight === "", `${label} marginHeight null`);
    assert(element.getAttribute("marginheight") === "", `${label} marginHeight attr`);
    assert(element.marginWidth === `${label}-width`, `${label} marginWidth`);
    assert(element.getAttribute("marginwidth") === `${label}-width`, `${label} marginWidth attr`);
    for (const name of frameNames) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
  }

  assert(!own(img, "longDesc"), "img.longDesc should not be own before set");
  img.longDesc = "https://assets.example/image-desc";
  assert(img.longDesc === "https://assets.example/image-desc", "image longDesc");
  assert(img.getAttribute("longdesc") === "https://assets.example/image-desc", "image longDesc attr");
  assert(!own(img, "longDesc"), "img.longDesc should not be own after set");
  assert(delete img.longDesc, "img.longDesc delete");
  assert(!own(img, "longDesc"), "img.longDesc should stay inherited");
  assert(img.longDesc === "https://assets.example/image-desc", "image longDesc after delete");
  return "ok";
})()
"#,
        )
        .expect("detached frame legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_resource_legacy_accessors_use_owner_prototypes() {
    let mut vm =
        new_storage_test_vm("https://detached-resource-legacy-prototypes.test/base/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const area = doc.createElement("area");
  const image = doc.createElement("img");
  const source = doc.createElement("source");
  const object = doc.createElement("object");
  const div = doc.createElement("div");
  doc.body.append(area, image, source, object, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  accessor(HTMLAreaElement.prototype, "alt");
  accessor(HTMLImageElement.prototype, "alt");
  accessor(HTMLImageElement.prototype, "useMap");
  accessor(HTMLImageElement.prototype, "srcset");
  accessor(HTMLImageElement.prototype, "lowsrc");
  accessor(HTMLImageElement.prototype, "decoding");
  accessor(HTMLSourceElement.prototype, "srcset");
  accessor(HTMLObjectElement.prototype, "useMap");
  for (const name of ["alt", "useMap", "srcset", "lowsrc", "decoding"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }

  for (const [element, names, label] of [
    [area, ["alt"], "area"],
    [image, ["alt", "useMap", "srcset", "lowsrc", "decoding"], "image"],
    [source, ["srcset"], "source"],
    [object, ["useMap"], "object"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
  }

  area.alt = "map alt";
  image.alt = "image alt";
  image.useMap = "#main-map";
  image.srcset = "small.png 1x, large.png 2x";
  image.lowsrc = "https://assets.example/low.png";
  image.decoding = "ASYNC";
  source.srcset = "source-small.png 1x";
  object.useMap = "#object-map";

  assert(area.alt === "map alt" && area.getAttribute("alt") === "map alt", "area alt");
  assert(image.alt === "image alt" && image.getAttribute("alt") === "image alt", "image alt");
  assert(image.useMap === "#main-map" && image.getAttribute("usemap") === "#main-map", "image useMap");
  assert(image.srcset === "small.png 1x, large.png 2x", "image srcset");
  assert(image.lowsrc === "https://assets.example/low.png", "image lowsrc");
  assert(image.decoding === "async" && image.getAttribute("decoding") === "ASYNC", "image decoding canonical");
  image.decoding = "invalid";
  assert(image.decoding === "auto", "image decoding invalid");
  assert(source.srcset === "source-small.png 1x" && source.getAttribute("srcset") === "source-small.png 1x", "source srcset");
  assert(object.useMap === "#object-map" && object.getAttribute("usemap") === "#object-map", "object useMap");

  for (const [element, names, label] of [
    [area, ["alt"], "area"],
    [image, ["alt", "useMap", "srcset", "lowsrc", "decoding"], "image"],
    [source, ["srcset"], "source"],
    [object, ["useMap"], "object"]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
  }
  assert(image.useMap === "#main-map", "image useMap after delete");
  assert(image.decoding === "auto", "image decoding after delete");
  assert(source.srcset === "source-small.png 1x", "source srcset after delete");
  assert(object.useMap === "#object-map", "object useMap after delete");
  return "ok";
})()
"##,
        )
        .expect("detached resource legacy owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_legacy_dimension_and_color_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-legacy-dimension-color-prototypes.test/");

    let result = vm
        .eval(
            r##"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const table = doc.createElement("table");
  const row = doc.createElement("tr");
  const cell = doc.createElement("td");
  const image = doc.createElement("img");
  const object = doc.createElement("object");
  const hr = doc.createElement("hr");
  const font = doc.createElement("font");
  const marquee = doc.createElement("marquee");
  const div = doc.createElement("div");
  doc.body.append(table, row, cell, image, object, hr, font, marquee, div);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const ownerChecks = [
    [HTMLBodyElement.prototype, "bgColor"],
    [HTMLTableElement.prototype, "bgColor"],
    [HTMLTableRowElement.prototype, "bgColor"],
    [HTMLTableCellElement.prototype, "bgColor"],
    [HTMLMarqueeElement.prototype, "bgColor"],
    [HTMLTableElement.prototype, "border"],
    [HTMLImageElement.prototype, "border"],
    [HTMLObjectElement.prototype, "border"],
    [HTMLHRElement.prototype, "color"],
    [HTMLFontElement.prototype, "color"],
    [HTMLImageElement.prototype, "hspace"],
    [HTMLImageElement.prototype, "vspace"],
    [HTMLObjectElement.prototype, "hspace"],
    [HTMLObjectElement.prototype, "vspace"],
    [HTMLMarqueeElement.prototype, "hspace"],
    [HTMLMarqueeElement.prototype, "vspace"]
  ];
  for (const [prototype, name] of ownerChecks) accessor(prototype, name);
  for (const name of ["bgColor", "border", "color", "hspace", "vspace"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!(name in div), `${name} should not be on div`);
  }

  for (const [element, name] of [
    [table, "bgColor"], [row, "bgColor"], [cell, "bgColor"], [marquee, "bgColor"],
    [table, "border"], [image, "border"], [object, "border"],
    [hr, "color"], [font, "color"],
    [image, "hspace"], [image, "vspace"], [object, "hspace"], [object, "vspace"],
    [marquee, "hspace"], [marquee, "vspace"]
  ]) {
    assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
  }

  table.bgColor = "red";
  row.bgColor = "green";
  cell.bgColor = null;
  marquee.bgColor = "blue";
  table.border = "3";
  image.border = null;
  object.border = null;
  hr.color = "black";
  font.color = null;
  image.hspace = 7;
  image.vspace = 8;
  object.hspace = 9;
  object.vspace = 10;
  marquee.hspace = 11;
  marquee.vspace = 12;

  assert(table.bgColor === "red" && table.getAttribute("bgcolor") === "red", "table bgColor");
  assert(row.bgColor === "green" && row.getAttribute("bgcolor") === "green", "row bgColor");
  assert(cell.bgColor === "" && cell.getAttribute("bgcolor") === "", "cell bgColor null");
  assert(marquee.bgColor === "blue" && marquee.getAttribute("bgcolor") === "blue", "marquee bgColor");
  assert(table.border === "3" && table.getAttribute("border") === "3", "table border");
  assert(image.border === "" && image.getAttribute("border") === "", "image border null");
  assert(object.border === "" && object.getAttribute("border") === "", "object border null");
  assert(hr.color === "black" && hr.getAttribute("color") === "black", "hr color");
  assert(font.color === "" && font.getAttribute("color") === "", "font color null");
  assert(image.hspace === 7 && image.getAttribute("hspace") === "7", "image hspace");
  assert(image.vspace === 8 && image.getAttribute("vspace") === "8", "image vspace");
  assert(object.hspace === 9 && object.getAttribute("hspace") === "9", "object hspace");
  assert(object.vspace === 10 && object.getAttribute("vspace") === "10", "object vspace");
  assert(marquee.hspace === 11 && marquee.getAttribute("hspace") === "11", "marquee hspace");
  assert(marquee.vspace === 12 && marquee.getAttribute("vspace") === "12", "marquee vspace");

  for (const [element, name] of [
    [table, "bgColor"], [row, "bgColor"], [cell, "bgColor"], [marquee, "bgColor"],
    [table, "border"], [image, "border"], [object, "border"],
    [hr, "color"], [font, "color"],
    [image, "hspace"], [image, "vspace"], [object, "hspace"], [object, "vspace"],
    [marquee, "hspace"], [marquee, "vspace"]
  ]) {
    assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
    assert(delete element[name], `${element.localName}.${name} delete`);
    assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
  }
  assert(table.bgColor === "red", "table bgColor after delete");
  assert(cell.bgColor === "", "cell bgColor after delete");
  assert(font.color === "", "font color after delete");
  assert(image.hspace === 7 && marquee.vspace === 12, "unsigned after delete");
  return "ok";
})()
"##,
        )
        .expect("detached legacy dimension and color owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}
