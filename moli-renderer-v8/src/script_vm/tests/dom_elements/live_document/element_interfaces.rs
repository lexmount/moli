use super::*;

#[test]
fn xml_documents_preserve_native_element_interfaces() {
    let mut vm = new_parsed_test_vm("https://xml-interfaces.test/", "<html><body></body></html>");
    assert_eq!(
        vm.eval(include_str!("xml_element_interfaces.js"))
            .expect("XML element interface fixture should evaluate"),
        "true"
    );
}

#[test]
fn specialized_element_methods_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://example.com/",
        "<!doctype html><html><head></head><body><form id='f'><input name='q'><select><option>a</option></select><table><tbody><tr></tr></tbody></table></form></body></html>",
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
              const method = (prototype, name, length, enumerable = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.value === "function", `${name} function`);
                assert(descriptor.value.length === length, `${name} length`);
                assert(descriptor.enumerable === enumerable, `${name} enumerable`);
                assert(descriptor.writable === true, `${name} writable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(descriptor.set === undefined, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              for (const [name, length] of [
                ["requestSubmit", 1],
                ["submit", 0],
                ["reset", 0],
                ["checkValidity", 0],
                ["reportValidity", 0]
              ]) {
                method(HTMLFormElement.prototype, name, length);
              }
              for (const [name, length] of [
                ["play", 0],
                ["pause", 0],
                ["load", 0],
                ["canPlayType", 1],
                ["addTextTrack", 1]
              ]) {
                method(HTMLMediaElement.prototype, name, length);
                assert(!own(HTMLAudioElement.prototype, name), `${name} duplicated on audio prototype`);
                assert(!own(HTMLVideoElement.prototype, name), `${name} duplicated on video prototype`);
              }
              method(HTMLImageElement.prototype, "decode", 0);
              for (const [name, length] of [
                ["showPicker", 0],
                ["stepUp", 0],
                ["stepDown", 0]
              ]) {
                method(HTMLInputElement.prototype, name, length);
              }
              for (const prototype of [HTMLInputElement.prototype, HTMLTextAreaElement.prototype]) {
                for (const [name, length] of [
                  ["setSelectionRange", 2],
                  ["setRangeText", 1],
                  ["select", 0]
                ]) {
                  method(prototype, name, length);
                }
              }
              for (const [name, length] of [
                ["add", 1],
                ["item", 1],
                ["namedItem", 1],
                ["remove", 0],
                ["showPicker", 0]
              ]) {
                method(HTMLSelectElement.prototype, name, length);
              }
              for (const prototype of [
                HTMLButtonElement.prototype,
                HTMLInputElement.prototype,
                HTMLMeterElement.prototype,
                HTMLOutputElement.prototype,
                HTMLProgressElement.prototype,
                HTMLSelectElement.prototype,
                HTMLTextAreaElement.prototype
              ]) {
                accessor(prototype, "labels");
              }
              for (const prototype of [
                HTMLButtonElement.prototype,
                HTMLFieldSetElement.prototype,
                HTMLInputElement.prototype,
                HTMLObjectElement.prototype,
                HTMLOutputElement.prototype,
                HTMLSelectElement.prototype,
                HTMLTextAreaElement.prototype
              ]) {
                for (const name of ["validity", "validationMessage", "willValidate"]) {
                  accessor(prototype, name);
                }
                for (const [name, length] of [
                  ["checkValidity", 0],
                  ["reportValidity", 0],
                  ["setCustomValidity", 1]
                ]) {
                  method(prototype, name, length);
                }
              }
              for (const [name, length] of [
                ["insertRow", 0],
                ["deleteRow", 1]
              ]) {
                method(HTMLTableSectionElement.prototype, name, length);
              }
              for (const [name, length] of [
                ["insertCell", 0],
                ["deleteCell", 1]
              ]) {
                method(HTMLTableRowElement.prototype, name, length);
              }

              const form = document.querySelector("form");
              const button = document.createElement("button");
              const fieldset = document.createElement("fieldset");
              const input = document.querySelector("input");
              const objectElement = document.createElement("object");
              const output = document.createElement("output");
              const textarea = document.createElement("textarea");
              const meter = document.createElement("meter");
              const progress = document.createElement("progress");
              const select = document.querySelector("select");
              const tbody = document.querySelector("tbody");
              const row = document.querySelector("tr");
              const image = document.createElement("img");
              const audio = document.createElement("audio");
              const video = document.createElement("video");
              form.append(button, fieldset, objectElement, output, textarea, meter, progress);

              for (const [object, names] of [
                [form, ["requestSubmit", "submit", "reset", "checkValidity", "reportValidity"]],
                [button, ["labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [fieldset, ["validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [input, ["showPicker", "stepUp", "stepDown", "setSelectionRange", "setRangeText", "select", "labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [objectElement, ["validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [output, ["labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [textarea, ["setSelectionRange", "setRangeText", "select", "labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [meter, ["labels"]],
                [progress, ["labels"]],
                [select, ["add", "item", "namedItem", "remove", "showPicker", "labels", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"]],
                [tbody, ["insertRow", "deleteRow"]],
                [row, ["insertCell", "deleteCell"]],
                [image, ["decode"]],
                [audio, ["play", "pause", "load", "canPlayType", "addTextTrack"]],
                [video, ["play", "pause", "load", "canPlayType", "addTextTrack"]]
              ]) {
                for (const name of names) {
                  assert(!own(object, name), `${name} should not be own on instance`);
                }
              }

              input.type = "number";
              input.value = "2";
              input.stepUp();
              assert(input.value === "3", "input stepUp behavior");
              textarea.value = "abcd";
              textarea.setSelectionRange(1, 3);
              textarea.setRangeText("XY");
              assert(textarea.value === "aXYd", "text control behavior");
              const option = document.createElement("option");
              option.value = "b";
              option.text = "b";
              select.add(option);
              assert(select.item(1) === option && select.namedItem("q") === null, "select methods behavior");
              select.remove(1);
              assert(select.length === 1, "select remove behavior");
              const detachedSelectDocument = new DOMParser().parseFromString(
                "<select><option id='first' value='a'>A</option></select>",
                "text/html"
              );
              const detachedSelect = detachedSelectDocument.querySelector("select");
              for (const name of ["length", "options", "selectedOptions", "selectedIndex", "value", "add", "item", "namedItem", "remove"]) {
                assert(!own(detachedSelect, name), `${name} should not be own on detached select`);
              }
              const detachedOption = detachedSelectDocument.createElement("option");
              detachedOption.id = "second";
              detachedOption.setAttribute("value", "b");
              detachedOption.text = "B";
              detachedSelect.add(detachedOption);
              detachedSelect.value = "b";
              assert(detachedSelect.length === 2, "detached select length behavior");
              assert(detachedSelect.options.length === 2, "detached select options behavior");
              const detachedItem = detachedSelect.item(1);
              assert(detachedItem?.id === "second" && detachedItem?.value === "b", "detached select item behavior");
              assert(detachedSelect.namedItem("second")?.value === "b", "detached select namedItem behavior");
              assert(detachedSelect.selectedIndex === 1 && detachedSelect.selectedOptions.length === 1, "detached select selected behavior");
              detachedSelect.remove(0);
              assert(detachedSelect.length === 1 && detachedSelect.item(0)?.id === "second", "detached select remove behavior");
              const insertedRow = tbody.insertRow();
              assert(insertedRow.parentNode === tbody, "section insertRow behavior");
              tbody.deleteRow(1);
              const cell = row.insertCell();
              assert(cell.parentNode === row, "row insertCell behavior");
              row.deleteCell(0);
              assert(row.cells.length === 0, "row deleteCell behavior");
              assert(form.checkValidity() === true && input.checkValidity() === true, "validation behavior");
              assert(input.validity.valid === true && input.validationMessage === "" && input.willValidate === true, "validation accessor behavior");
              assert(input.labels.length === 0 && meter.labels.length === 0, "labels behavior");
              assert(typeof audio.canPlayType("audio/mpeg") === "string" && typeof image.decode().then === "function", "media/image behavior");
              return "ok";
            })()
            "#,
        )
        .expect("specialized element method prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_specialized_element_surfaces_are_inherited() {
    let mut vm = new_parsed_test_vm(
        "https://detached-specialized-owner-prototypes.test/base/page.html",
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
  const probeInherited = (element, names, label) => {
    for (const name of names) {
      assert(name in element, `${label}.${name} missing`);
      assert(!own(element, name), `${label}.${name} should not be own before access`);
      void element[name];
      assert(!own(element, name), `${label}.${name} should not be own after access`);
    }
  };

  const doc = new DOMParser().parseFromString(`
    <!doctype html>
    <html>
      <head>
        <base id="base">
        <link id="link">
        <meta id="meta">
        <title id="title">Title</title>
      </head>
      <body id="body">
        <form id="form">
          <button id="button"></button>
          <fieldset id="fieldset"><legend id="legend"></legend><input id="field"></fieldset>
          <input id="input" list="choices">
          <datalist id="choices"><option id="data-option"></option></datalist>
          <select id="select"><option id="option">Choice</option></select>
          <textarea id="textarea">Text</textarea>
          <output id="output"></output>
          <object id="object"><param id="param"></object>
          <meter id="meter"></meter>
          <progress id="progress"></progress>
        </form>
        <a id="anchor"></a>
        <area id="area">
        <audio id="audio"></audio>
        <blockquote id="blockquote"></blockquote>
        <data id="data"></data>
        <del id="del"></del>
        <details id="details"></details>
        <dir id="dir"></dir>
        <dl id="dl"></dl>
        <embed id="embed">
        <font id="font"></font>
        <frame id="frame">
        <iframe id="iframe"></iframe>
        <hr id="hr">
        <img id="image">
        <ins id="ins"></ins>
        <label id="label" for="input"></label>
        <li id="li"></li>
        <map id="map"></map>
        <marquee id="marquee"></marquee>
        <menu id="menu"></menu>
        <ol id="ol"></ol>
        <optgroup id="optgroup"></optgroup>
        <q id="q"></q>
        <slot id="slot"></slot>
        <source id="source">
        <table id="table"><tbody id="tbody"><tr id="row"><td id="cell"></td></tr></tbody></table>
        <template id="template"><span></span></template>
        <time id="time"></time>
        <track id="track">
        <ul id="ul"></ul>
        <video id="video"></video>
      </body>
    </html>
  `, "text/html");
  const id = name => doc.getElementById(name);
  const detachedFrame = id("frame") || doc.createElement("frame");

  const cases = [
    [doc.documentElement, ["version"], "html"],
    [id("body"), ["onload", "text", "link", "vLink", "aLink", "background"], "body"],
    [id("anchor"), ["href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "target", "download", "rel", "relList", "name", "text"], "anchor"],
    [id("area"), ["href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "target", "download", "rel", "relList", "alt"], "area"],
    [id("audio"), ["preload", "play", "pause", "load", "canPlayType", "addTextTrack"], "audio"],
    [id("base"), ["target"], "base"],
    [id("blockquote"), ["cite"], "blockquote"],
    [id("button"), ["disabled", "form", "formAction", "formEnctype", "formMethod", "formNoValidate", "formTarget", "labels", "name", "type", "validity", "validationMessage", "value", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "button"],
    [id("data"), ["value"], "data"],
    [id("del"), ["cite", "dateTime"], "del"],
    [id("details"), ["name", "open"], "details"],
    [id("dir"), ["compact"], "dir"],
    [id("dl"), ["compact"], "dl"],
    [id("embed"), ["getSVGDocument", "name"], "embed"],
    [id("fieldset"), ["disabled", "elements", "form", "name", "type", "validity", "validationMessage", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "fieldset"],
    [id("font"), ["color"], "font"],
    [id("form"), ["acceptCharset", "action", "autocomplete", "elements", "encoding", "enctype", "length", "method", "name", "noValidate", "rel", "relList", "target", "requestSubmit", "submit", "reset", "checkValidity", "reportValidity"], "form"],
    [detachedFrame, ["frameBorder", "longDesc", "marginHeight", "marginWidth", "name", "scrolling"], "frame"],
    [id("hr"), ["color", "noShade"], "hr"],
    [id("iframe"), ["contentDocument", "contentWindow", "frameBorder", "getSVGDocument", "longDesc", "marginHeight", "marginWidth", "name", "scrolling", "src", "srcdoc"], "iframe"],
    [id("image"), ["alt", "border", "decode", "decoding", "height", "hspace", "longDesc", "lowsrc", "name", "src", "srcset", "useMap", "vspace", "width"], "image"],
    [id("input"), ["accept", "alt", "autocomplete", "checked", "defaultChecked", "defaultValue", "dirName", "disabled", "files", "form", "formAction", "formEnctype", "formMethod", "formNoValidate", "formTarget", "height", "indeterminate", "labels", "list", "max", "maxLength", "min", "minLength", "multiple", "name", "pattern", "placeholder", "readOnly", "required", "size", "src", "step", "type", "validity", "validationMessage", "value", "valueAsDate", "valueAsNumber", "willValidate", "width", "checkValidity", "reportValidity", "setCustomValidity", "select", "setRangeText", "setSelectionRange", "showPicker", "stepDown", "stepUp"], "input"],
    [id("ins"), ["cite", "dateTime"], "ins"],
    [id("label"), ["control", "form", "htmlFor"], "label"],
    [id("legend"), ["form"], "legend"],
    [id("li"), ["value"], "li"],
    [id("link"), ["integrity", "media", "rel", "relList", "rev", "target", "type"], "link"],
    [id("map"), ["name"], "map"],
    [id("marquee"), ["bgColor", "hspace", "vspace"], "marquee"],
    [id("menu"), ["compact"], "menu"],
    [id("meta"), ["content", "httpEquiv", "media", "name", "scheme"], "meta"],
    [id("meter"), ["high", "labels", "low", "max", "min", "optimum", "value"], "meter"],
    [id("object"), ["archive", "border", "code", "codeBase", "codeType", "contentDocument", "contentWindow", "data", "declare", "form", "getSVGDocument", "hspace", "name", "standby", "type", "useMap", "validity", "validationMessage", "vspace", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "object"],
    [id("ol"), ["compact", "reversed", "start", "type"], "ol"],
    [id("optgroup"), ["disabled", "label"], "optgroup"],
    [id("option"), ["defaultSelected", "disabled", "form", "index", "label", "selected", "text", "value"], "option"],
    [id("output"), ["defaultValue", "form", "labels", "name", "type", "validity", "validationMessage", "value", "willValidate", "checkValidity", "reportValidity", "setCustomValidity"], "output"],
    [id("param"), ["name", "type", "value", "valueType"], "param"],
    [id("progress"), ["labels", "max", "position", "value"], "progress"],
    [id("q"), ["cite"], "q"],
    [id("select"), ["autocomplete", "disabled", "form", "labels", "length", "multiple", "name", "options", "required", "selectedIndex", "selectedOptions", "size", "validity", "validationMessage", "value", "willValidate", "add", "checkValidity", "item", "namedItem", "remove", "reportValidity", "setCustomValidity"], "select"],
    [id("slot"), ["name"], "slot"],
    [id("source"), ["media", "srcset"], "source"],
    [id("table"), ["bgColor", "border", "caption", "rows", "tBodies", "tFoot", "tHead"], "table"],
    [id("tbody"), ["rows"], "tbody"],
    [id("row"), ["bgColor", "cells", "rowIndex", "sectionRowIndex"], "row"],
    [id("cell"), ["bgColor", "cellIndex", "colSpan", "rowSpan"], "cell"],
    [id("template"), ["content"], "template"],
    [id("textarea"), ["autocomplete", "cols", "defaultValue", "dirName", "disabled", "form", "labels", "maxLength", "minLength", "name", "placeholder", "readOnly", "required", "rows", "textLength", "type", "validity", "validationMessage", "value", "willValidate", "wrap", "checkValidity", "reportValidity", "select", "setCustomValidity", "setRangeText", "setSelectionRange"], "textarea"],
    [id("time"), ["dateTime"], "time"],
    [id("track"), ["label"], "track"],
    [id("ul"), ["compact"], "ul"],
    [id("video"), ["preload", "play", "pause", "load", "canPlayType", "addTextTrack"], "video"]
  ];

  for (const [element, names, label] of cases) {
    probeInherited(element, names, label);
  }
  return "ok";
})()
"#,
        )
        .expect("detached specialized owner prototype inventory should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_form_and_target_accessors_live_on_owner_prototypes() {
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
              const accessor = (prototype, name, hasSetter = true) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
                return descriptor;
              };
              const throwsTypeError = callback => {
                try {
                  callback();
                  return false;
                } catch (error) {
                  return error instanceof TypeError;
                }
              };

              const formNames = [
                ["action", true],
                ["acceptCharset", true],
                ["autocomplete", true],
                ["enctype", true],
                ["encoding", true],
                ["elements", false],
                ["length", false],
                ["method", true],
                ["name", true],
                ["noValidate", true],
                ["target", true]
              ];
              for (const [name, hasSetter] of formNames) {
                accessor(HTMLFormElement.prototype, name, hasSetter);
              }

              const form = document.createElement("form");
              form.innerHTML = "<input name='q'><button name='go'></button>";
              document.body.append(form);
              for (const name of [
                "action",
                "acceptCharset",
                "autocomplete",
                "enctype",
                "encoding",
                "elements",
                "length",
                "method",
                "name",
                "noValidate",
                "target"
              ]) {
                assert(!own(form, name), `${name} should not be own on form`);
              }
              form.action = "/submit";
              form.acceptCharset = "utf-8";
              form.autocomplete = "off";
              form.enctype = "multipart/form-data";
              assert(form.enctype === "multipart/form-data", "form enctype behavior");
              form.encoding = "text/plain";
              form.method = "post";
              form.name = "search";
              form.noValidate = true;
              form.target = "_blank";
              assert(form.action === "https://example.com/submit", "form action behavior");
              assert(form.acceptCharset === "utf-8", "form acceptCharset behavior");
              assert(form.encoding === "text/plain", "form encoding behavior");
              assert(form.method === "post", "form method behavior");
              assert(form.name === "search", "form name behavior");
              assert(form.noValidate === true, "form noValidate behavior");
              assert(form.target === "_blank", "form target behavior");
              assert(form.elements.length === 2 && form.length === 2, "form collection behavior");

              const associationHost = document.createElement("form");
              associationHost.id = "owner";
              associationHost.innerHTML = `
                <button></button>
                <fieldset></fieldset>
                <input>
                <object></object>
                <output></output>
                <select></select>
                <textarea></textarea>
              `;
              document.body.append(associationHost);
              const formAssociatedOwners = [
                [HTMLButtonElement.prototype, associationHost.querySelector("button"), "button"],
                [HTMLFieldSetElement.prototype, associationHost.querySelector("fieldset"), "fieldset"],
                [HTMLInputElement.prototype, associationHost.querySelector("input"), "input"],
                [HTMLObjectElement.prototype, associationHost.querySelector("object"), "object"],
                [HTMLOutputElement.prototype, associationHost.querySelector("output"), "output"],
                [HTMLSelectElement.prototype, associationHost.querySelector("select"), "select"],
                [HTMLTextAreaElement.prototype, associationHost.querySelector("textarea"), "textarea"]
              ];
              for (const [prototype, element, label] of formAssociatedOwners) {
                accessor(prototype, "form", false);
                assert(!own(element, "form"), `${label} form should not be own`);
                assert(element.form === associationHost, `${label} form owner`);
                element.form = null;
                assert(!own(element, "form"), `${label} form assignment should not create own`);
                assert(element.form === associationHost, `${label} form after assignment`);
                assert(delete element.form, `${label} form delete`);
                assert(!own(element, "form"), `${label} form after delete`);
              }

              const explicitLabel = document.createElement("label");
              const explicitInput = document.createElement("input");
              const implicitLabel = document.createElement("label");
              const implicitInput = document.createElement("textarea");
              explicitInput.id = "label-target";
              explicitLabel.htmlFor = "label-target";
              implicitLabel.append("implicit", implicitInput);
              associationHost.append(explicitLabel, explicitInput, implicitLabel);
              accessor(HTMLLabelElement.prototype, "htmlFor", true);
              accessor(HTMLLabelElement.prototype, "control", false);
              accessor(HTMLLabelElement.prototype, "form", false);
              for (const label of [explicitLabel, implicitLabel]) {
                assert(!own(label, "htmlFor"), "label htmlFor should not be own");
                assert(!own(label, "control"), "label control should not be own");
                assert(!own(label, "form"), "label form should not be own");
              }
              assert(explicitLabel.htmlFor === "label-target", "label htmlFor behavior");
              assert(explicitLabel.control === explicitInput, "explicit label control");
              assert(implicitLabel.control === implicitInput, "implicit label control");
              assert(explicitLabel.form === associationHost, "explicit label form");
              assert(implicitLabel.form === associationHost, "implicit label form");
              explicitLabel.control = null;
              explicitLabel.form = null;
              assert(!own(explicitLabel, "control"), "label control assignment should not create own");
              assert(!own(explicitLabel, "form"), "label form assignment should not create own");
              assert(explicitLabel.control === explicitInput, "label control after assignment");
              assert(explicitLabel.form === associationHost, "label form after assignment");

              const autocompleteOwners = [
                [HTMLInputElement.prototype, document.createElement("input"), "input"],
                [HTMLSelectElement.prototype, document.createElement("select"), "select"],
                [HTMLTextAreaElement.prototype, document.createElement("textarea"), "textarea"]
              ];
              for (const [prototype, element, label] of autocompleteOwners) {
                accessor(prototype, "autocomplete", true);
                assert(!own(element, "autocomplete"), `${label} autocomplete should not be own`);
                element.autocomplete = " NAME\t";
                assert(element.getAttribute("autocomplete") === " NAME\t", `${label} autocomplete setter reflection`);
                assert(element.autocomplete === "name", `${label} autocomplete canonical getter`);
              }

              const targetOwners = [
                [HTMLAnchorElement.prototype, document.createElement("a"), "anchor"],
                [HTMLAreaElement.prototype, document.createElement("area"), "area"],
                [HTMLBaseElement.prototype, document.createElement("base"), "base"],
                [HTMLLinkElement.prototype, document.createElement("link"), "link"],
                [HTMLFormElement.prototype, form, "form"]
              ];
              const div = document.createElement("div");
              const text = document.createTextNode("x");
              const targetDescriptors = targetOwners.map(([prototype, element, label]) => [
                accessor(prototype, "target", true),
                element,
                label
              ]);
              for (const [descriptor, element, label] of targetDescriptors) {
                assert(!own(element, "target"), `${label} target should not be own`);
                descriptor.set.call(element, `${label}-target`);
                assert(element.target === `${label}-target`, `${label} target behavior`);
                assert(descriptor.get.call(element) === `${label}-target`, `${label} target direct getter`);
                assert(!own(element, "target"), `${label} target should stay inherited`);
                for (const receiver of [{}, text, div]) {
                  assert(throwsTypeError(() => descriptor.get.call(receiver)), `${label} target getter receiver`);
                  assert(throwsTypeError(() => descriptor.set.call(receiver, "bad")), `${label} target setter receiver`);
                }
                for (const [, otherElement, otherLabel] of targetDescriptors) {
                  if (otherElement === element) continue;
                  assert(throwsTypeError(() => descriptor.get.call(otherElement)), `${label} getter rejects ${otherLabel}`);
                  assert(throwsTypeError(() => descriptor.set.call(otherElement, "bad")), `${label} setter rejects ${otherLabel}`);
                }
              }
              assert(Object.getOwnPropertyDescriptor(HTMLElement.prototype, "form") === undefined, "HTMLElement form absent");
              assert(!("form" in div), "plain HTMLElement form absent");
              assert(Object.getOwnPropertyDescriptor(HTMLElement.prototype, "target") === undefined, "HTMLElement target absent");
              assert(!("target" in div), "plain HTMLElement target absent");
              assert(!("autocomplete" in div), "plain HTMLElement autocomplete absent");
              return "ok";
            })()
            "#,
        )
        .expect("form and target accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn html_rel_accessors_live_on_owner_prototypes() {
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
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };
              const cases = [
                [HTMLAnchorElement.prototype, document.createElement("a"), "anchor"],
                [HTMLAreaElement.prototype, document.createElement("area"), "area"],
                [HTMLFormElement.prototype, document.createElement("form"), "form"],
                [HTMLLinkElement.prototype, document.createElement("link"), "link"]
              ];
              for (const [prototype] of cases) {
                accessor(prototype, "rel");
                accessor(prototype, "relList");
              }
              assert(!own(HTMLElement.prototype, "rel"), "rel should not be on HTMLElement.prototype");
              assert(!own(HTMLElement.prototype, "relList"), "relList should not be on HTMLElement.prototype");
              const div = document.createElement("div");
              assert(!("rel" in div), "rel should not be on div");
              assert(!("relList" in div), "relList should not be on div");

              for (const [, element, label] of cases) {
                assert(!own(element, "rel"), `${label}.rel should not be own before set`);
                assert(!own(element, "relList"), `${label}.relList should not be own before set`);
                const list = element.relList;
                assert(Object.prototype.toString.call(list) === "[object DOMTokenList]", `${label}.relList tag`);
                assert(list === element.relList, `${label}.relList should be stable`);
                element.rel = `${label}-one ${label}-two ${label}-one`;
                assert(element.rel === `${label}-one ${label}-two ${label}-one`, `${label}.rel getter`);
                assert(element.getAttribute("rel") === `${label}-one ${label}-two ${label}-one`, `${label}.rel attr`);
                assert(list.length === 2, `${label}.relList length`);
                assert(list.contains(`${label}-one`), `${label}.relList contains`);
                element.relList = `${label}-three`;
                assert(element.rel === `${label}-three`, `${label}.relList setter`);
                assert(list.length === 1 && list.contains(`${label}-three`), `${label}.relList after setter`);
                assert(!own(element, "rel"), `${label}.rel should not be own after set`);
                assert(!own(element, "relList"), `${label}.relList should not be own after set`);
                assert(delete element.rel, `${label}.rel delete`);
                assert(delete element.relList, `${label}.relList delete`);
                assert(!own(element, "rel"), `${label}.rel should stay inherited`);
                assert(!own(element, "relList"), `${label}.relList should stay inherited`);
                assert(element.rel === `${label}-three`, `${label}.rel after delete`);
                assert(element.relList === list, `${label}.relList stable after delete`);
              }
              return "ok";
            })()
            "#,
        )
        .expect("rel owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_boolean_accessors_live_on_owner_prototypes() {
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
                assert(!!descriptor, `${prototype.constructor.name}.${name} descriptor missing`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(typeof descriptor.set === "function", `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const compactOwners = [
                [HTMLDirectoryElement.prototype, document.createElement("dir"), "dir"],
                [HTMLDListElement.prototype, document.createElement("dl"), "dl"],
                [HTMLMenuElement.prototype, document.createElement("menu"), "menu"],
                [HTMLOListElement.prototype, document.createElement("ol"), "ol"],
                [HTMLUListElement.prototype, document.createElement("ul"), "ul"]
              ];
              for (const [prototype] of compactOwners) {
                accessor(prototype, "compact");
              }
              accessor(HTMLHRElement.prototype, "noShade");
              assert(!own(HTMLElement.prototype, "compact"), "compact should not be on HTMLElement.prototype");
              assert(!own(HTMLElement.prototype, "noShade"), "noShade should not be on HTMLElement.prototype");
              const div = document.createElement("div");
              assert(!("compact" in div), "plain HTMLElement compact absent");
              assert(!("noShade" in div), "plain HTMLElement noShade absent");

              for (const [, element, label] of compactOwners) {
                assert(!own(element, "compact"), `${label}.compact should not be own before set`);
                element.compact = true;
                assert(element.compact === true, `${label}.compact true`);
                assert(element.hasAttribute("compact"), `${label}.compact attr`);
                assert(!own(element, "compact"), `${label}.compact should not be own after true`);
                element.compact = false;
                assert(element.compact === false, `${label}.compact false`);
                assert(!element.hasAttribute("compact"), `${label}.compact attr removed`);
                element.compact = true;
                assert(delete element.compact, `${label}.compact delete`);
                assert(!own(element, "compact"), `${label}.compact should stay inherited`);
                assert(element.compact === true, `${label}.compact after delete`);
              }

              const hr = document.createElement("hr");
              assert(!own(hr, "noShade"), "hr.noShade should not be own before set");
              hr.noShade = true;
              assert(hr.noShade === true, "hr.noShade true");
              assert(hr.hasAttribute("noshade"), "hr.noShade attr");
              assert(!own(hr, "noShade"), "hr.noShade should not be own after true");
              hr.noShade = false;
              assert(hr.noShade === false, "hr.noShade false");
              assert(!hr.hasAttribute("noshade"), "hr.noShade attr removed");
              hr.noShade = true;
              assert(delete hr.noShade, "hr.noShade delete");
              assert(!own(hr, "noShade"), "hr.noShade should stay inherited");
              assert(hr.noShade === true, "hr.noShade after delete");
              return "ok";
            })()
            "#,
        )
        .expect("legacy boolean owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn link_and_meta_legacy_metadata_reflect_on_native_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://metadata-reflection.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm.eval(r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  for (const owner of [document, document.implementation.createHTMLDocument('')]) {
    for (const [tag, property, prototype] of [
      ['link', 'rev', HTMLLinkElement.prototype],
      ['link', 'type', HTMLLinkElement.prototype],
      ['meta', 'scheme', HTMLMetaElement.prototype]
    ]) {
      const element = owner.createElement(tag);
      owner.head.appendChild(element);
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      assert(descriptor && descriptor.enumerable && descriptor.configurable, `${tag}.${property} descriptor`);
      assert(typeof descriptor.get === 'function' && typeof descriptor.set === 'function', 'accessors');
      assert(element[property] === '', 'missing value');
      element[property] = 'MiXeD\u00E9\u{1F642}';
      assert(element.getAttribute(property) === 'MiXeD\u00E9\u{1F642}', 'DOMString reflection');
      element.setAttribute(property, 'updated');
      assert(element[property] === 'updated', 'attribute mutation');
      element.removeAttribute(property);
      assert(element[property] === '', 'attribute removal');
      let converted = false;
      try {
        descriptor.set.call({}, { toString() { converted = true; return 'forged'; } });
        throw new Error('accepted forged receiver');
      } catch (error) { assert(error instanceof TypeError, 'receiver brand'); }
      assert(!converted, 'receiver validation precedes conversion');
    }
  }
  return 'ok';
})()
"#).expect("link and meta metadata reflection probe should evaluate");
    assert_eq!(result, "ok");
}
