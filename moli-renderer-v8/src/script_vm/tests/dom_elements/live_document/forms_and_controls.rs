use super::*;

#[test]
fn html_name_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const owners = [
                [HTMLAnchorElement.prototype, document.createElement("a"), "anchor"],
                [HTMLButtonElement.prototype, document.createElement("button"), "button"],
                [HTMLDetailsElement.prototype, document.createElement("details"), "details"],
                [HTMLEmbedElement.prototype, document.createElement("embed"), "embed"],
                [HTMLFieldSetElement.prototype, document.createElement("fieldset"), "fieldset"],
                [HTMLFormElement.prototype, document.createElement("form"), "form"],
                [HTMLFrameElement.prototype, document.createElement("frame"), "frame"],
                [HTMLIFrameElement.prototype, document.createElement("iframe"), "iframe"],
                [HTMLImageElement.prototype, document.createElement("img"), "image"],
                [HTMLInputElement.prototype, document.createElement("input"), "input"],
                [HTMLMapElement.prototype, document.createElement("map"), "map"],
                [HTMLMetaElement.prototype, document.createElement("meta"), "meta"],
                [HTMLObjectElement.prototype, document.createElement("object"), "object"],
                [HTMLOutputElement.prototype, document.createElement("output"), "output"],
                [HTMLParamElement.prototype, document.createElement("param"), "param"],
                [HTMLSelectElement.prototype, document.createElement("select"), "select"],
                [HTMLSlotElement.prototype, document.createElement("slot"), "slot"],
                [HTMLTextAreaElement.prototype, document.createElement("textarea"), "textarea"]
              ];

              for (const [prototype, element, label] of owners) {
                accessor(prototype, "name");
                assert(!own(element, "name"), `${label} name should not be own`);
                element.name = `${label}-name`;
                assert(element.name === `${label}-name`, `${label} name behavior`);
                assert(element.getAttribute("name") === `${label}-name`, `${label} name reflection`);
              }

              const div = document.createElement("div");
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              assert(Object.getOwnPropertyDescriptor(Element.prototype, "name") === undefined, "Element name absent");
              assert(Object.getOwnPropertyDescriptor(HTMLElement.prototype, "name") === undefined, "HTMLElement name absent");
              assert(!("name" in div), "plain HTMLElement name absent");
              assert(!("name" in svg), "SVGElement name absent");
              return "ok";
            })()
            "#,
        )
        .expect("HTML name accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_value_accessor_live_on_owner_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://button-value-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, "value");
  assert(!!descriptor, "HTMLButtonElement.value descriptor missing");
  assert(typeof descriptor.get === "function", "value getter");
  assert(typeof descriptor.set === "function", "value setter");
  assert(descriptor.enumerable === true, "value enumerable");
  assert(descriptor.configurable === true, "value configurable");
  assert(!own(HTMLElement.prototype, "value"), "value should not live on HTMLElement");
  assert(!("value" in document.createElement("div")), "value should not be on div");

  const button = document.createElement("button");
  document.body.append(button);
  assert(!own(button, "value"), "button.value should not be own before set");
  button.value = "go";
  assert(button.value === "go", "button.value getter");
  assert(button.getAttribute("value") === "go", "button value attr");
  assert(!own(button, "value"), "button.value should not be own after set");
  assert(delete button.value, "delete button.value");
  assert(!own(button, "value"), "button.value should stay inherited");
  assert(button.value === "go", "button.value after delete");
  return "ok";
})()
"#,
        )
        .expect("button value owner prototype accessor should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn form_control_value_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://form-control-values-owner-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
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

  accessor(HTMLInputElement.prototype, "value", true);
  accessor(HTMLInputElement.prototype, "defaultValue", true);
  accessor(HTMLTextAreaElement.prototype, "value", true);
  accessor(HTMLTextAreaElement.prototype, "defaultValue", true);
  accessor(HTMLOutputElement.prototype, "value", true);
  accessor(HTMLOutputElement.prototype, "defaultValue", true);
  accessor(HTMLOptionElement.prototype, "value", true);
  accessor(HTMLOptionElement.prototype, "text", true);
  accessor(HTMLOptionElement.prototype, "defaultSelected", true);
  accessor(HTMLOptionElement.prototype, "disabled", true);
  accessor(HTMLOptionElement.prototype, "form", false);
  accessor(HTMLOptionElement.prototype, "index", false);
  assert(!Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, "name"), "HTMLOptionElement.name should be absent");
  accessor(HTMLOptionElement.prototype, "selected", true);
  accessor(HTMLSelectElement.prototype, "length", true);
  accessor(HTMLSelectElement.prototype, "options", false);
  accessor(HTMLSelectElement.prototype, "selectedOptions", false);
  accessor(HTMLSelectElement.prototype, "selectedIndex", true);
  accessor(HTMLSelectElement.prototype, "value", true);
  accessor(HTMLSelectElement.prototype, "disabled", true);
  accessor(HTMLSelectElement.prototype, "multiple", true);
  accessor(HTMLSelectElement.prototype, "required", true);
  accessor(HTMLSelectElement.prototype, "size", true);

  const input = document.createElement("input");
  const textarea = document.createElement("textarea");
  const output = document.createElement("output");
  const form = document.createElement("form");
  const select = document.createElement("select");
  const option = document.createElement("option");
  select.append(option);
  form.append(select);
  document.body.append(input, textarea, output, form);

  for (const [element, names] of [
    [input, ["value", "defaultValue"]],
    [textarea, ["value", "defaultValue"]],
    [output, ["value", "defaultValue"]],
    [option, ["value", "text", "defaultSelected", "disabled", "form", "index", "selected"]],
    [select, ["length", "options", "selectedOptions", "selectedIndex", "value", "disabled", "multiple", "required", "size"]]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
    }
  }

  input.value = "typed";
  input.defaultValue = "seed";
  textarea.value = "body";
  textarea.defaultValue = "default body";
  output.value = "shown";
  output.defaultValue = "fallback";
  option.value = "choice";
  option.text = "Choice";
  option.defaultSelected = true;
  option.disabled = true;
  option.selected = true;
  select.disabled = true;
  select.multiple = true;
  select.required = true;
  select.size = 4;
  select.value = "choice";
  select.selectedIndex = 0;
  select.length = 2;

  assert(input.value === "typed", "input value");
  assert(input.defaultValue === "seed", "input defaultValue");
  assert(textarea.value === "body", "textarea value");
  assert(textarea.defaultValue === "default body", "textarea defaultValue");
  assert(output.value === "shown", "output value");
  assert(output.defaultValue === "fallback", "output defaultValue");
  assert(option.value === "choice", "option value");
  assert(option.text === "Choice", "option text");
  assert(option.defaultSelected === true && option.hasAttribute("selected"), "option defaultSelected");
  assert(option.disabled === true && option.hasAttribute("disabled"), "option disabled");
  assert(option.form === form, "option form");
  assert(option.index === 0, "option index");
  assert(option.selected === true, "option selected");
  assert(select.value === "choice", "select value");
  assert(select.selectedIndex === 0, "select selectedIndex");
  assert(select.length === 2, "select length");
  assert(select.disabled === true && select.hasAttribute("disabled"), "select disabled");
  assert(select.multiple === true && select.hasAttribute("multiple"), "select multiple");
  assert(select.required === true && select.hasAttribute("required"), "select required");
  assert(select.size === 4, "select size");

  for (const [element, names] of [
    [input, ["value", "defaultValue"]],
    [textarea, ["value", "defaultValue"]],
    [output, ["value", "defaultValue"]],
    [option, ["value", "text", "defaultSelected", "disabled", "form", "index", "selected"]],
    [select, ["length", "options", "selectedOptions", "selectedIndex", "value", "disabled", "multiple", "required", "size"]]
  ]) {
    for (const name of names) {
      assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
      assert(delete element[name], `delete ${element.localName}.${name}`);
      assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
    }
  }

  assert(input.value === "typed", "input value after delete");
  assert(input.defaultValue === "seed", "input defaultValue after delete");
  assert(textarea.value === "body", "textarea value after delete");
  assert(textarea.defaultValue === "default body", "textarea defaultValue after delete");
  assert(output.value === "shown", "output value after delete");
  assert(output.defaultValue === "fallback", "output defaultValue after delete");
  assert(option.value === "choice", "option value after delete");
  assert(option.text === "Choice", "option text after delete");
  assert(option.defaultSelected === true, "option defaultSelected after delete");
  assert(option.disabled === true, "option disabled after delete");
  assert(option.form === form, "option form after delete");
  assert(option.index === 0, "option index after delete");
  assert(option.selected === true, "option selected after delete");
  assert(select.value === "choice", "select value after delete");
  assert(select.selectedIndex === 0, "select selectedIndex after delete");
  assert(select.length === 2, "select length after delete");
  assert(select.disabled === true, "select disabled after delete");
  assert(select.multiple === true, "select multiple after delete");
  assert(select.required === true, "select required after delete");
  assert(select.size === 4, "select size after delete");
  return "ok";
})()
"#,
        )
        .expect("form control value owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn select_element_members_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://select-receiver-brand.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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
  const makeOption = (id, value) => {
    const option = document.createElement("option");
    option.id = id;
    option.value = value;
    option.text = value;
    return option;
  };

  const select = document.createElement("select");
  const first = makeOption("first", "a");
  const second = makeOption("second", "b");
  select.append(first, second);
  document.body.append(select);

  const input = document.createElement("input");
  const option = document.createElement("option");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
  const badReceivers = [{}, text, div, input, option];

  const cases = [
    ["disabled", true, value => value === true],
    ["multiple", true, value => value === true],
    ["required", true, value => value === true],
    ["size", 3, value => value === 3],
    ["length", 2, value => value === 2],
    ["selectedIndex", 0, value => value === 0],
    ["value", "a", value => value === "a"]
  ];
  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(select, value);
    assert(check(descriptor.get.call(select)), `${name} valid receiver`);
    assert(!own(select, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }

  for (const [name, check] of [
    ["options", value => value.length === 2 && value[0] === first],
    ["selectedOptions", value => value.length === 1 && value[0] === first]
  ]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} readonly`);
    assert(check(descriptor.get.call(select)), `${name} valid receiver`);
    assert(!own(select, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
  }

  const third = makeOption("third", "c");
  const methods = [
    ["item", [0], value => value === first],
    ["namedItem", ["first"], value => value === first],
    ["add", [third], value => value === undefined && select.length === 3],
    ["remove", [2], value => value === undefined && select.length === 2]
  ];
  for (const [name, args, check] of methods) {
    const method = HTMLSelectElement.prototype[name];
    assert(typeof method === "function", `${name} method`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
    assert(check(method.call(select, ...args)), `${name} valid receiver`);
    assert(!own(select, name), `${name} should stay inherited`);
  }
  return "ok";
})()
"#,
        )
        .expect("select receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn option_element_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://option-receiver-brand.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const form = document.createElement("form");
  const select = document.createElement("select");
  const option = document.createElement("option");
  select.append(option);
  form.append(select);
  document.body.append(form);

  const input = document.createElement("input");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
  const badReceivers = [{}, text, div, input, select];
  const cases = [
    ["value", "choice", value => value === "choice"],
    ["text", "Choice", value => value === "Choice"],
    ["defaultSelected", true, value => value === true],
    ["disabled", true, value => value === true],
    ["label", "Label", value => value === "Label"],
    ["selected", true, value => value === true]
  ];
  for (const [name, value, check] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    descriptor.set.call(option, value);
    assert(check(descriptor.get.call(option)), `${name} valid receiver`);
    assert(!own(option, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, value)), `${name} setter receiver`);
    }
  }

  for (const [name, check] of [
    ["form", value => value === form],
    ["index", value => value === 0]
  ]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLOptionElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.set === undefined, `${name} readonly`);
    assert(check(descriptor.get.call(option)), `${name} valid receiver`);
    assert(!own(option, name), `${name} should stay inherited`);
    for (const receiver of badReceivers) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("option receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn input_element_accessors_live_on_owner_prototype() {
    let mut vm = new_parsed_test_vm(
        "https://input-accessor-owner-prototypes.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `HTMLInputElement.${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const names = [
    ["accept", true],
    ["alt", true],
    ["defaultChecked", true],
    ["defaultValue", true],
    ["disabled", true],
    ["dirName", true],
    ["files", true],
    ["formAction", true],
    ["formEnctype", true],
    ["formMethod", true],
    ["formNoValidate", true],
    ["formTarget", true],
    ["height", true],
    ["list", false],
    ["maxLength", true],
    ["max", true],
    ["minLength", true],
    ["min", true],
    ["multiple", true],
    ["pattern", true],
    ["placeholder", true],
    ["readOnly", true],
    ["required", true],
    ["size", true],
    ["src", true],
    ["step", true],
    ["type", true],
    ["valueAsDate", true],
    ["valueAsNumber", true],
    ["value", true],
    ["width", true],
    ["checked", true],
    ["indeterminate", true]
  ];
  for (const [name, hasSetter] of names) {
    accessor(HTMLInputElement.prototype, name, hasSetter);
  }

  const input = document.createElement("input");
  const datalist = document.createElement("datalist");
  datalist.id = "choices";
  document.body.append(input, datalist);
  input.setAttribute("list", "choices");
  for (const [name] of names) {
    assert(!own(input, name), `${name} should not be own before set`);
  }

  input.accept = "image/png";
  input.alt = "preview";
  input.defaultChecked = true;
  input.defaultValue = "seed";
  input.disabled = true;
  input.dirName = "field.dir";
  input.formAction = "/submit";
  input.formEnctype = "multipart/form-data";
  input.formMethod = "post";
  input.formNoValidate = true;
  input.formTarget = "frame";
  input.height = 12;
  input.maxLength = 10;
  input.max = "9";
  input.minLength = 2;
  input.min = "1";
  input.multiple = true;
  input.pattern = "[a-z]+";
  input.placeholder = "hint";
  input.readOnly = true;
  input.required = true;
  input.size = 7;
  input.src = "/button.png";
  input.step = "2";
  input.type = "number";
  input.value = "4";
  input.valueAsNumber = 6.5;
  input.width = 20;
  input.checked = true;
  input.indeterminate = true;

  assert(input.accept === "image/png", "accept");
  assert(input.alt === "preview", "alt");
  assert(input.defaultChecked === true && input.hasAttribute("checked"), "defaultChecked");
  assert(input.defaultValue === "seed", "defaultValue");
  assert(input.disabled === true && input.hasAttribute("disabled"), "disabled");
  assert(input.dirName === "field.dir", "dirName");
  assert(input.files === null, "files on non-file input");
  assert(input.formAction === "https://input-accessor-owner-prototypes.test/submit", "formAction");
  assert(input.formEnctype === "multipart/form-data", "formEnctype");
  assert(input.formMethod === "post", "formMethod");
  assert(input.formNoValidate === true && input.hasAttribute("formnovalidate"), "formNoValidate");
  assert(input.formTarget === "frame", "formTarget");
  assert(input.height === 12, "height");
  assert(input.list === datalist, "list");
  assert(input.maxLength === 10, "maxLength");
  assert(input.max === "9", "max");
  assert(input.minLength === 2, "minLength");
  assert(input.min === "1", "min");
  assert(input.multiple === true && input.hasAttribute("multiple"), "multiple");
  assert(input.pattern === "[a-z]+", "pattern");
  assert(input.placeholder === "hint", "placeholder");
  assert(input.readOnly === true && input.hasAttribute("readonly"), "readOnly");
  assert(input.required === true && input.hasAttribute("required"), "required");
  assert(input.size === 7, "size");
  assert(input.src === "https://input-accessor-owner-prototypes.test/button.png", "src");
  assert(input.step === "2", "step");
  assert(input.type === "number", "type number");
  assert(input.value === "6.5", "valueAsNumber writes value");
  assert(input.valueAsNumber === 6.5, "valueAsNumber");
  assert(input.width === 20, "width");
  assert(input.checked === true, "checked");
  assert(input.indeterminate === true, "indeterminate");

  input.type = "date";
  input.valueAsDate = new Date(Date.UTC(2020, 0, 2));
  assert(input.type === "date", "type date");
  assert(input.value === "2020-01-02", "valueAsDate writes value");
  assert(input.valueAsDate instanceof Date, "valueAsDate getter");

  const fileInput = document.createElement("input");
  fileInput.type = "file";
  document.body.append(fileInput);
  assert(!own(fileInput, "files"), "file input files should not be own");
  assert(fileInput.files !== null, "file input files getter");

  for (const [name] of names) {
    assert(!own(input, name), `${name} should not be own after set`);
    assert(delete input[name], `delete ${name}`);
    assert(!own(input, name), `${name} should stay inherited`);
  }
  assert(input.accept === "image/png", "accept after delete");
  assert(input.defaultValue === "seed", "defaultValue after delete");
  assert(input.disabled === true, "disabled after delete");
  assert(input.list === datalist, "list after delete");
  assert(input.value === "2020-01-02", "value after delete");
  assert(input.width === 20, "width after delete");
  assert(input.checked === true, "checked after delete");
  assert(input.indeterminate === true, "indeterminate after delete");
  return "ok";
})()
"#,
        )
        .expect("input owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn input_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://input-submit-overrides-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
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
  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("input submitter override receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn input_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://input-reflected-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const input = document.createElement("input");
  const textarea = document.createElement("textarea");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("input reflected receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn simple_control_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://simple-control-owner-prototypes.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
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

  const form = document.createElement("form");
  const fieldset = document.createElement("fieldset");
  const legend = document.createElement("legend");
  const input = document.createElement("input");
  const datalist = document.createElement("datalist");
  const option = document.createElement("option");
  const output = document.createElement("output");
  const meter = document.createElement("meter");
  const progress = document.createElement("progress");
  fieldset.append(legend, input);
  datalist.append(option);
  form.append(fieldset, datalist, output, meter, progress);
  document.body.append(form);

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
"#,
        )
        .expect("simple control owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn simple_control_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://simple-control-receiver-brand.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const form = document.createElement("form");
  const fieldset = document.createElement("fieldset");
  const input = document.createElement("input");
  const meter = document.createElement("meter");
  const progress = document.createElement("progress");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
  fieldset.append(input);
  form.append(fieldset, meter, progress);
  document.body.append(form);

  const badReceivers = [{}, text, div, input, document.createElement("button")];
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
        .expect("simple control receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_and_textarea_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://button-textarea-owner-prototypes.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
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

  const form = document.createElement("form");
  const button = document.createElement("button");
  const target = document.createElement("div");
  const textarea = document.createElement("textarea");
  assert(!("required" in button), "button required should not be an IDL property");
  assert(!Object.getOwnPropertyDescriptor(HTMLButtonElement.prototype, "required"),
         "HTMLButtonElement.required descriptor should be absent");
  button.setAttribute("required", false);
  assert(button.getAttribute("required") === "false", "button required=false attribute text");
  button.setAttribute("required", true);
  assert(button.getAttribute("required") === "true", "button required=true attribute text");
  target.id = "target";
  form.append(button, textarea);
  document.body.append(target, form);

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
  assert(button.formAction === new URL("/submit", document.URL).href, "button formAction");
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
"#,
        )
        .expect("button/textarea owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_element_reflections_track_content_attribute_mutations() {
    let mut vm = new_parsed_test_vm(
        "https://button-element-reflection.test/",
        r#"<!doctype html><button id="button" popovertarget="target"></button><div id="target"></div>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const button = document.getElementById("button");
  const target = document.getElementById("target");

  assert(button.popoverTargetElement === target, "content attribute target");
  button.popoverTargetElement = null;
  assert(!button.hasAttribute("popovertarget"), "null removes content attribute");
  assert(button.popoverTargetElement === null, "null clears explicit target");

  for (const [property, attribute] of [
    ["commandForElement", "commandfor"],
    ["interestForElement", "interestfor"],
    ["popoverTargetElement", "popovertarget"]
  ]) {
    button[property] = target;
    assert(button.getAttribute(attribute) === "", `${property} writes empty content attribute`);
    assert(button[property] === target, `${property} retains explicit target`);

    button.setAttribute(attribute, "missing");
    assert(button[property] === null, `${attribute} mutation clears explicit target`);

    button[property] = target;
    button.setAttribute(attribute, "");
    assert(button[property] === null, `${attribute} same-value mutation clears explicit target`);

    button.setAttribute(attribute, "target");
    assert(button[property] === target, `${property} falls back to ID lookup`);

    let threw = false;
    try {
      button[property] = {};
    } catch (error) {
      threw = error instanceof TypeError;
    }
    assert(threw, `${property} rejects non-Element values`);
    assert(button.getAttribute(attribute) === "target", `${property} conversion precedes mutation`);

    button.removeAttribute(attribute);
    assert(button[property] === null, `${attribute} removal clears target`);
  }
  return "ok";
})()
"#,
        )
        .expect("button element reflections should remain synchronized with their attributes");

    assert_eq!(result, "ok");
}

#[test]
fn button_submitter_override_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://button-submit-overrides-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
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
  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("button submitter override receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn button_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://button-reflected-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("button reflected receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn textarea_reflected_accessors_reject_incompatible_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://textarea-reflected-brand.test/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

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

  const textarea = document.createElement("textarea");
  const input = document.createElement("input");
  const button = document.createElement("button");
  const div = document.createElement("div");
  const text = document.createTextNode("x");
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
        .expect("textarea reflected receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn object_param_and_data_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/base/page.html",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
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

              const object = document.createElement("object");
              const param = document.createElement("param");
              const data = document.createElement("data");
              const div = document.createElement("div");
              document.body.append(object, param, data, div);

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

              object.data = "https://assets.example/plugin.bin";
              object.type = "application/x-test";
              object.archive = "archive.jar";
              object.code = "Applet";
              object.codeBase = "https://assets.example/classes/";
              object.codeType = "application/java";
              object.declare = true;
              object.standby = "Loading";
              assert(object.data === "https://assets.example/plugin.bin", "object data");
              assert(object.type === "application/x-test", "object type");
              assert(object.archive === "archive.jar", "object archive");
              assert(object.code === "Applet", "object code");
              assert(object.codeBase === "https://assets.example/classes/", "object codeBase");
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
              assert(object.data === "https://assets.example/plugin.bin", "object data after delete");
              assert(object.declare === true, "object declare after delete");
              assert(param.valueType === "data", "param valueType after delete");
              assert(data.value === "data-value", "data value after delete");
              return "ok";
            })()
            "#,
        )
        .expect("object/param/data owner prototype accessor probe should evaluate");

    assert_eq!(result, "ok");
}
