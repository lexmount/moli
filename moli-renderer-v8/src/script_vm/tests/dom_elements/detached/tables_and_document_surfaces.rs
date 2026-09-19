use super::*;

#[test]
fn table_cell_legacy_accessors_use_owner_prototype() {
    let mut vm = new_storage_test_vm("https://table-cell-legacy-prototype.test/");

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
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const names = ["headers", "abbr", "axis", "scope", "noWrap"];
  for (const name of names) {
    accessor(HTMLTableCellElement.prototype, name);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }

  for (const [tag, missing] of [
    ["div", names],
    ["table", ["headers", "abbr", "axis", "scope", "noWrap"]],
    ["tr", ["headers", "abbr", "axis", "scope", "noWrap"]],
    ["a", ["headers", "abbr", "axis", "scope", "noWrap"]]
  ]) {
    const element = document.createElement(tag);
    for (const name of missing) {
      assert(!(name in element), `${tag} should not expose ${name}`);
    }
  }

  const parsed = new DOMParser().parseFromString(
    "<html><body><table><tr><td></td><th></th></tr></table></body></html>",
    "text/html"
  );
  const cells = [
    [document.createElement("td"), "live td"],
    [document.createElement("th"), "live th"],
    [parsed.querySelector("td"), "detached td"],
    [parsed.querySelector("th"), "detached th"]
  ];

  for (const [cell, label] of cells) {
    for (const name of names) {
      assert(!own(cell, name), `${label}.${name} should not be own before set`);
    }
    cell.headers = `${label}-headers`;
    cell.abbr = `${label}-abbr`;
    cell.axis = `${label}-axis`;
    cell.scope = "ROWGROUP";
    cell.noWrap = true;
    for (const name of names) {
      assert(!own(cell, name), `${label}.${name} should not be own after set`);
    }
    assert(cell.headers === `${label}-headers`, `${label}.headers value`);
    assert(cell.getAttribute("headers") === `${label}-headers`, `${label}.headers attr`);
    assert(cell.abbr === `${label}-abbr`, `${label}.abbr value`);
    assert(cell.getAttribute("abbr") === `${label}-abbr`, `${label}.abbr attr`);
    assert(cell.axis === `${label}-axis`, `${label}.axis value`);
    assert(cell.getAttribute("axis") === `${label}-axis`, `${label}.axis attr`);
    assert(cell.scope === "rowgroup", `${label}.scope canonical`);
    assert(cell.getAttribute("scope") === "ROWGROUP", `${label}.scope attr`);
    cell.scope = "invalid";
    assert(cell.scope === "", `${label}.scope invalid canonical`);
    assert(cell.getAttribute("scope") === "invalid", `${label}.scope invalid attr`);
    assert(cell.noWrap === true && cell.hasAttribute("nowrap"), `${label}.noWrap true`);
    cell.noWrap = false;
    assert(cell.noWrap === false && !cell.hasAttribute("nowrap"), `${label}.noWrap false`);
    for (const name of names) {
      assert(delete cell[name], `${label}.${name} delete`);
      assert(!own(cell, name), `${label}.${name} should stay inherited`);
    }
    assert(cell.headers === `${label}-headers`, `${label}.headers after delete`);
    assert(cell.scope === "", `${label}.scope after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("table cell legacy prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_table_structural_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://detached-table-structural-prototypes.test/");

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
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter shape`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  for (const [prototype, name, hasSetter] of [
    [HTMLTableSectionElement.prototype, "rows", false],
    [HTMLTableRowElement.prototype, "rowIndex", false],
    [HTMLTableRowElement.prototype, "sectionRowIndex", false],
    [HTMLTableRowElement.prototype, "cells", false],
    [HTMLTableCellElement.prototype, "colSpan", true],
    [HTMLTableCellElement.prototype, "rowSpan", true],
    [HTMLTableCellElement.prototype, "cellIndex", false]
  ]) {
    accessor(prototype, name, hasSetter);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }

  const detachedDoc = document.implementation.createHTMLDocument("");
  for (const [doc, label] of [[document, "live"], [detachedDoc, "detached"]]) {
    const table = doc.createElement("table");
    const tbody = doc.createElement("tbody");
    const firstRow = doc.createElement("tr");
    const secondRow = doc.createElement("tr");
    const firstCell = doc.createElement("td");
    const secondCell = doc.createElement("th");
    table.append(tbody);
    tbody.append(firstRow, secondRow);
    firstRow.append(firstCell);
    secondRow.append(secondCell);

    for (const [element, names, elementLabel] of [
      [tbody, ["rows"], "tbody"],
      [firstRow, ["rowIndex", "sectionRowIndex", "cells"], "firstRow"],
      [secondRow, ["rowIndex", "sectionRowIndex", "cells"], "secondRow"],
      [firstCell, ["colSpan", "rowSpan", "cellIndex"], "firstCell"],
      [secondCell, ["colSpan", "rowSpan", "cellIndex"], "secondCell"]
    ]) {
      for (const name of names) {
        assert(!own(element, name), `${label}.${elementLabel}.${name} should not be own before access`);
      }
    }

    assert(tbody.rows.length === 2, `${label}.rows length`);
    assert(firstRow.rowIndex === 0, `${label}.first rowIndex`);
    assert(firstRow.sectionRowIndex === 0, `${label}.first sectionRowIndex`);
    assert(secondRow.rowIndex === 1, `${label}.second rowIndex`);
    assert(secondRow.sectionRowIndex === 1, `${label}.second sectionRowIndex`);
    assert(firstRow.cells.length === 1, `${label}.first cells`);
    assert(secondRow.cells.length === 1, `${label}.second cells`);
    assert(firstCell.cellIndex === 0, `${label}.first cellIndex`);
    assert(secondCell.cellIndex === 0, `${label}.second cellIndex`);

    firstCell.colSpan = 7;
    firstCell.rowSpan = 0;
    secondCell.colSpan = 2000;
    secondCell.rowSpan = -5;
    assert(firstCell.colSpan === 7 && firstCell.getAttribute("colspan") === "7", `${label}.colSpan`);
    assert(firstCell.rowSpan === 0 && firstCell.getAttribute("rowspan") === "0", `${label}.rowSpan zero`);
    assert(secondCell.colSpan === 1000 && secondCell.getAttribute("colspan") === "2000", `${label}.colSpan clamp`);
    assert(secondCell.rowSpan === 1 && secondCell.getAttribute("rowspan") === "1", `${label}.rowSpan clamp`);

    firstCell.setAttribute("colspan", "4294967296");
    firstCell.setAttribute("rowspan", "2147483648");
    assert(firstCell.colSpan === 1000, `${label}.colSpan large content clamp`);
    assert(firstCell.rowSpan === 65534, `${label}.rowSpan large content clamp`);
    firstCell.setAttribute("rowspan", "-0");
    assert(firstCell.rowSpan === 0, `${label}.rowSpan minus-zero content`);
    firstCell.colSpan = "-0";
    assert(firstCell.getAttribute("colspan") === "0" && firstCell.colSpan === 1, `${label}.colSpan zero setter`);
    firstCell.colSpan = 1001;
    assert(firstCell.getAttribute("colspan") === "1001" && firstCell.colSpan === 1000, `${label}.colSpan setter clamp`);
    firstCell.rowSpan = 65535;
    assert(firstCell.getAttribute("rowspan") === "65535" && firstCell.rowSpan === 65534, `${label}.rowSpan setter clamp`);
    firstCell.colSpan = 2147483648;
    firstCell.rowSpan = 4294967295;
    assert(firstCell.getAttribute("colspan") === "1" && firstCell.colSpan === 1, `${label}.colSpan setter default`);
    assert(firstCell.getAttribute("rowspan") === "1" && firstCell.rowSpan === 1, `${label}.rowSpan setter default`);
    firstCell.colSpan = 7;
    firstCell.rowSpan = 0;

    for (const [element, names, elementLabel] of [
      [tbody, ["rows"], "tbody"],
      [firstRow, ["rowIndex", "sectionRowIndex", "cells"], "firstRow"],
      [secondRow, ["rowIndex", "sectionRowIndex", "cells"], "secondRow"],
      [firstCell, ["colSpan", "rowSpan", "cellIndex"], "firstCell"],
      [secondCell, ["colSpan", "rowSpan", "cellIndex"], "secondCell"]
    ]) {
      for (const name of names) {
        assert(!own(element, name), `${label}.${elementLabel}.${name} should not be own after access`);
        assert(delete element[name], `${label}.${elementLabel}.${name} delete`);
        assert(!own(element, name), `${label}.${elementLabel}.${name} should stay inherited`);
      }
    }
    assert(tbody.rows.length === 2, `${label}.rows after delete`);
    assert(firstRow.cells.length === 1, `${label}.cells after delete`);
    assert(firstCell.colSpan === 7, `${label}.colSpan after delete`);
    assert(firstCell.rowSpan === 0, `${label}.rowSpan after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached table structural owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_html_table_structural_members_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://detached-table-receiver-brand.test/base/");

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
  const doc = document.implementation.createHTMLDocument("");
  const table = doc.createElement("table");
  const caption = doc.createElement("caption");
  const thead = doc.createElement("thead");
  const tfoot = doc.createElement("tfoot");
  const tbody = doc.createElement("tbody");
  const row = doc.createElement("tr");
  const td = doc.createElement("td");
  const th = doc.createElement("th");
  const div = doc.createElement("div");
  const text = doc.createTextNode("x");
  table.append(caption, thead, tbody, tfoot);
  tbody.append(row);
  row.append(td, th);

  const tableBad = [{}, text, div, tbody, row, td, th];
  const sectionBad = [{}, text, div, table, row, td, th];
  const rowBad = [{}, text, div, table, tbody, td, th];
  const cellBad = [{}, text, div, table, tbody, row];

  for (const name of ["caption", "tHead", "tFoot", "rows", "tBodies"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name);
    assert(!!descriptor, `${name} descriptor`);
    assert(typeof descriptor.get.call(table) !== "undefined", `${name} valid getter`);
    if (typeof descriptor.set === "function") {
      descriptor.set.call(table, null);
    }
    for (const receiver of tableBad) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      if (typeof descriptor.set === "function") {
        assert(throwsTypeError(() => descriptor.set.call(receiver, null)), `${name} setter receiver`);
      }
    }
  }

  const tableMethods = {
    createCaption: [],
    deleteCaption: [],
    createTHead: [],
    deleteTHead: [],
    createTFoot: [],
    deleteTFoot: [],
    createTBody: [],
    insertRow: [-1],
    deleteRow: [-1]
  };
  const methodTable = doc.createElement("table");
  for (const [name, args] of Object.entries(tableMethods)) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name).value;
    assert(typeof method === "function", `${name} method`);
    method.call(methodTable, ...args);
    for (const receiver of tableBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} method receiver`);
    }
  }

  const sectionRows = Object.getOwnPropertyDescriptor(HTMLTableSectionElement.prototype, "rows");
  assert(sectionRows.get.call(tbody).length === 1, "section rows valid getter");
  for (const receiver of sectionBad) {
    assert(throwsTypeError(() => sectionRows.get.call(receiver)), "section rows receiver");
  }
  for (const [name, args] of [["insertRow", [-1]], ["deleteRow", [-1]]]) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableSectionElement.prototype, name).value;
    const section = doc.createElement("tbody");
    method.call(section, ...args);
    for (const receiver of sectionBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} receiver`);
    }
  }

  for (const name of ["rowIndex", "sectionRowIndex", "cells"]) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableRowElement.prototype, name);
    assert(typeof descriptor.get.call(row) !== "undefined", `${name} valid getter`);
    for (const receiver of rowBad) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} receiver`);
    }
  }
  for (const [name, args] of [["insertCell", [-1]], ["deleteCell", [-1]]]) {
    const method = Object.getOwnPropertyDescriptor(HTMLTableRowElement.prototype, name).value;
    const methodRow = doc.createElement("tr");
    method.call(methodRow, ...args);
    for (const receiver of rowBad) {
      assert(throwsTypeError(() => method.call(receiver, ...args)), `${name} receiver`);
    }
  }

  for (const cell of [td, th]) {
    for (const name of ["colSpan", "rowSpan", "cellIndex"]) {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLTableCellElement.prototype, name);
      assert(typeof descriptor.get.call(cell) !== "undefined", `${name} valid getter`);
      if (typeof descriptor.set === "function") {
        descriptor.set.call(cell, 2);
      }
      for (const receiver of cellBad) {
        assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
        if (typeof descriptor.set === "function") {
          assert(throwsTypeError(() => descriptor.set.call(receiver, 2)), `${name} setter receiver`);
        }
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("detached HTML table structural receiver brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn table_legacy_alignment_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://table-legacy-alignment-prototype.test/");

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
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const tableAlignmentNames = ["ch", "chOff", "vAlign"];
  for (const prototype of [
    HTMLTableSectionElement.prototype,
    HTMLTableRowElement.prototype,
    HTMLTableColElement.prototype,
    HTMLTableCellElement.prototype
  ]) {
    for (const name of tableAlignmentNames) accessor(prototype, name);
  }
  accessor(HTMLTableColElement.prototype, "span");

  for (const name of [...tableAlignmentNames, "span"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
  }
  for (const tag of ["div", "table", "a"]) {
    const element = document.createElement(tag);
    for (const name of tableAlignmentNames) {
      assert(!(name in element), `${tag} should not expose ${name}`);
    }
    assert(!("span" in element), `${tag} should not expose span`);
  }
  for (const tag of ["thead", "tr", "td"]) {
    assert(!("span" in document.createElement(tag)), `${tag} should not expose span`);
  }

  const parsed = new DOMParser().parseFromString(
    "<html><body><table><colgroup><col></colgroup><thead></thead><tbody><tr><td></td></tr></tbody></table></body></html>",
    "text/html"
  );
  const alignmentCases = [
    [document.createElement("thead"), "live section"],
    [document.createElement("tr"), "live row"],
    [document.createElement("colgroup"), "live colgroup"],
    [document.createElement("col"), "live col"],
    [document.createElement("td"), "live cell"],
    [parsed.querySelector("thead"), "detached section"],
    [parsed.querySelector("tr"), "detached row"],
    [parsed.querySelector("colgroup"), "detached colgroup"],
    [parsed.querySelector("col"), "detached col"],
    [parsed.querySelector("td"), "detached cell"]
  ];

  for (const [element, label] of alignmentCases) {
    for (const name of tableAlignmentNames) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }
    element.ch = `${label}-char`;
    element.chOff = `${label}-charoff`;
    element.vAlign = `${label}-valign`;
    for (const name of tableAlignmentNames) {
      assert(!own(element, name), `${label}.${name} should not be own after set`);
    }
    assert(element.ch === `${label}-char`, `${label}.ch value`);
    assert(element.getAttribute("char") === `${label}-char`, `${label}.char attr`);
    assert(element.chOff === `${label}-charoff`, `${label}.chOff value`);
    assert(element.getAttribute("charoff") === `${label}-charoff`, `${label}.charoff attr`);
    assert(element.vAlign === `${label}-valign`, `${label}.vAlign value`);
    assert(element.getAttribute("valign") === `${label}-valign`, `${label}.valign attr`);
    for (const name of tableAlignmentNames) {
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
    assert(element.ch === `${label}-char`, `${label}.ch after delete`);
    assert(element.chOff === `${label}-charoff`, `${label}.chOff after delete`);
    assert(element.vAlign === `${label}-valign`, `${label}.vAlign after delete`);
  }

  const colCases = [
    [document.createElement("colgroup"), "live colgroup"],
    [document.createElement("col"), "live col"],
    [parsed.querySelector("colgroup"), "detached colgroup"],
    [parsed.querySelector("col"), "detached col"]
  ];
  for (const [element, label] of colCases) {
    assert(!own(element, "span"), `${label}.span should not be own before set`);
    assert(element.span === 1, `${label}.span default`);
    element.span = 12;
    assert(!own(element, "span"), `${label}.span should not be own after set`);
    assert(element.span === 12, `${label}.span numeric value`);
    assert(element.getAttribute("span") === "12", `${label}.span attr`);
    element.span = 0;
    assert(element.getAttribute("span") === "0", `${label}.span zero attr`);
    assert(element.span === 1, `${label}.span zero canonical`);
    element.span = 1002;
    assert(element.getAttribute("span") === "1002", `${label}.span large attr`);
    assert(element.span === 1000, `${label}.span large canonical`);
    element.setAttribute("span", "4294967296");
    assert(element.span === 1000, `${label}.span large content clamp`);
    element.span = 2147483648;
    assert(element.getAttribute("span") === "1", `${label}.span setter default attr`);
    assert(element.span === 1, `${label}.span setter default`);
    element.setAttribute("span", "invalid");
    assert(element.span === 1, `${label}.span invalid canonical`);
    assert(delete element.span, `${label}.span delete`);
    assert(!own(element, "span"), `${label}.span should stay inherited`);
  }

  const spanDescriptor = Object.getOwnPropertyDescriptor(HTMLTableColElement.prototype, "span");
  for (const receiver of [document.createElement("div"), document.createElement("td"), {}]) {
    assert(throwsTypeError(() => spanDescriptor.get.call(receiver)), "span getter receiver");
    assert(throwsTypeError(() => spanDescriptor.set.call(receiver, 2)), "span setter receiver");
  }
  return "ok";
})()
"#,
        )
        .expect("table legacy alignment prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_align_accessor_uses_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://legacy-align-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, label) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, "align");
    assert(!!descriptor, `${label}.align descriptor missing`);
    assert(typeof descriptor.get === "function", `${label}.align getter`);
    assert(typeof descriptor.set === "function", `${label}.align setter`);
    assert(descriptor.enumerable === true, `${label}.align enumerable`);
    assert(descriptor.configurable === true, `${label}.align configurable`);
  };

  const ownerPrototypes = [
    [HTMLDivElement.prototype, "HTMLDivElement"],
    [HTMLHeadingElement.prototype, "HTMLHeadingElement"],
    [HTMLParagraphElement.prototype, "HTMLParagraphElement"],
    [HTMLHRElement.prototype, "HTMLHRElement"],
    [HTMLImageElement.prototype, "HTMLImageElement"],
    [HTMLObjectElement.prototype, "HTMLObjectElement"],
    [HTMLIFrameElement.prototype, "HTMLIFrameElement"],
    [HTMLEmbedElement.prototype, "HTMLEmbedElement"],
    [HTMLLegendElement.prototype, "HTMLLegendElement"],
    [HTMLTableCaptionElement.prototype, "HTMLTableCaptionElement"],
    [HTMLTableElement.prototype, "HTMLTableElement"],
    [HTMLTableSectionElement.prototype, "HTMLTableSectionElement"],
    [HTMLTableRowElement.prototype, "HTMLTableRowElement"],
    [HTMLTableColElement.prototype, "HTMLTableColElement"],
    [HTMLTableCellElement.prototype, "HTMLTableCellElement"],
    [HTMLInputElement.prototype, "HTMLInputElement"]
  ];
  for (const [prototype, label] of ownerPrototypes) accessor(prototype, label);

  assert(!own(HTMLElement.prototype, "align"), "align should not be on HTMLElement.prototype");
  for (const tag of ["body", "section", "a", "span", "button"]) {
    const element = document.createElement(tag);
    assert(!("align" in element), `${tag} should not expose align`);
  }

  const parsed = new DOMParser().parseFromString(
    `<!doctype html><html><body>
      <div></div><h1></h1><p></p><hr><img><object></object><iframe></iframe><embed>
      <fieldset><legend></legend></fieldset><input>
      <table><caption></caption><colgroup><col></colgroup><thead></thead><tbody><tr><td></td></tr></tbody></table>
    </body></html>`,
    "text/html"
  );
  const cases = [
    [document.createElement("div"), parsed.querySelector("div"), "div"],
    [document.createElement("h1"), parsed.querySelector("h1"), "h1"],
    [document.createElement("p"), parsed.querySelector("p"), "p"],
    [document.createElement("hr"), parsed.querySelector("hr"), "hr"],
    [document.createElement("img"), parsed.querySelector("img"), "img"],
    [document.createElement("object"), parsed.querySelector("object"), "object"],
    [document.createElement("iframe"), parsed.querySelector("iframe"), "iframe"],
    [document.createElement("embed"), parsed.querySelector("embed"), "embed"],
    [document.createElement("legend"), parsed.querySelector("legend"), "legend"],
    [document.createElement("caption"), parsed.querySelector("caption"), "caption"],
    [document.createElement("table"), parsed.querySelector("table"), "table"],
    [document.createElement("thead"), parsed.querySelector("thead"), "thead"],
    [document.createElement("tr"), parsed.querySelector("tr"), "tr"],
    [document.createElement("colgroup"), parsed.querySelector("colgroup"), "colgroup"],
    [document.createElement("col"), parsed.querySelector("col"), "col"],
    [document.createElement("td"), parsed.querySelector("td"), "td"],
    [document.createElement("input"), parsed.querySelector("input"), "input"]
  ];

  for (const [live, detached, tag] of cases) {
    for (const [element, flavor] of [[live, "live"], [detached, "detached"]]) {
      assert(!own(element, "align"), `${flavor} ${tag}.align should not be own before set`);
      element.align = `${flavor}-${tag}-align`;
      assert(!own(element, "align"), `${flavor} ${tag}.align should not be own after set`);
      assert(element.getAttribute("align") === `${flavor}-${tag}-align`, `${flavor} ${tag}.align attr`);
      assert(element.align === `${flavor}-${tag}-align`, `${flavor} ${tag}.align value`);
      assert(delete element.align, `${flavor} ${tag}.align delete`);
      assert(!own(element, "align"), `${flavor} ${tag}.align should stay inherited`);
      assert(element.align === `${flavor}-${tag}-align`, `${flavor} ${tag}.align after delete`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("legacy align prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_element_names_use_element_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-element-names-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const htmlDoc = document.implementation.createHTMLDocument("");
  const htmlElement = htmlDoc.createElement("div");
  const svgElement = htmlDoc.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  const xmlDoc = document.implementation.createDocument("urn:doc", "root", null);
  const xmlElement = xmlDoc.createElementNS("urn:item", "p:item");
  const parsedDoc = new DOMParser().parseFromString("<html><body><section></section></body></html>", "text/html");
  const parsedElement = parsedDoc.querySelector("section");
  const adoptedElement = xmlDoc.adoptNode(document.createElementNS("urn:live", "q:live"));

  const names = ["tagName", "localName", "namespaceURI", "prefix"];
  const descriptorShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const ownShape = (element) =>
    names.map((name) => `${name}:${Object.prototype.hasOwnProperty.call(element, name)}`).join(",");
  const values = (element) =>
    [element.tagName, element.localName, element.namespaceURI, element.prefix].join(",");

  htmlElement.tagName = "shadow";
  htmlElement.localName = "shadow";
  htmlElement.namespaceURI = "urn:shadow";
  htmlElement.prefix = "shadow";
  const deleteResult = [
    delete htmlElement.tagName,
    delete htmlElement.localName,
    delete htmlElement.namespaceURI,
    delete htmlElement.prefix
  ].join(",");

  return [
    names.map(descriptorShape).join("|"),
    [htmlElement, svgElement, xmlElement, parsedElement, adoptedElement].map(ownShape).join("|"),
    [htmlElement, svgElement, xmlElement, parsedElement, adoptedElement].map(values).join("|"),
    deleteResult,
    ownShape(htmlElement)
  ].join("||");
})()
"#,
        )
        .expect("detached element name prototype accessors should evaluate");

    assert_eq!(
        result,
        "true:function:true:true:true|true:function:true:true:true|true:function:true:true:true|true:function:true:true:true||tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false|tagName:false,localName:false,namespaceURI:false,prefix:false||DIV,div,http://www.w3.org/1999/xhtml,|svg:g,g,http://www.w3.org/2000/svg,svg|p:item,item,urn:item,p|SECTION,section,http://www.w3.org/1999/xhtml,|q:live,live,urn:live,q||true,true,true,true||tagName:false,localName:false,namespaceURI:false,prefix:false"
    );
}

#[test]
fn detached_element_declared_prototype_members_are_not_public_own_properties() {
    let mut vm = new_storage_test_vm("https://detached-element-own-property-audit.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const declaredPrototypeNames = (object) => {
    const names = new Set();
    for (
      let prototype = Object.getPrototypeOf(object);
      prototype && prototype !== Object.prototype;
      prototype = Object.getPrototypeOf(prototype)
    ) {
      for (const name of Object.getOwnPropertyNames(prototype)) {
        if (name !== "constructor") names.add(name);
      }
    }
    return names;
  };
  const publicOwnNames = (object) =>
    Object.getOwnPropertyNames(object).filter((name) =>
      !name.startsWith("__moli") &&
      !name.startsWith("__lm") &&
      !/^\d+$/.test(name)
    );
  const offendersFor = (object) => {
    const declared = declaredPrototypeNames(object);
    return publicOwnNames(object).filter((name) => declared.has(name)).sort();
  };

  const html = document.implementation.createHTMLDocument("");
  const liveTags = [
    "a", "area", "audio", "base", "br", "button", "canvas", "data",
    "datalist", "details", "dialog", "div", "embed", "fieldset", "font",
    "form", "h1", "hr", "iframe", "img", "input", "label", "legend", "li",
    "link", "map", "marquee", "meta", "meter", "object", "ol", "optgroup",
    "option", "output", "p", "param", "pre", "progress", "q", "script",
    "select", "source", "span", "style", "table", "caption", "colgroup",
    "col", "tbody", "tr", "td", "textarea", "time", "title", "track", "ul",
    "video"
  ];
  const objects = [
    ["detached:html", html.documentElement],
    ["detached:head", html.head],
    ["detached:body", html.body]
  ];
  for (const tag of liveTags) {
    objects.push([`detached:${tag}`, html.createElement(tag)]);
    objects.push([`live:${tag}`, document.createElement(tag)]);
  }

  const svg = html.createElementNS("http://www.w3.org/2000/svg", "svg:g");
  const xml = document.implementation.createDocument("urn:test", "root", null);
  objects.push(["detached:svg:g", svg]);
  objects.push(["detached:xml:p:item", xml.createElementNS("urn:item", "p:item")]);
  objects.push(["detached:xml:plain", xml.createElement("MixedCase")]);

  const offenders = [];
  for (const [label, object] of objects) {
    const names = offendersFor(object);
    if (names.length) offenders.push(`${label}:${names.join(",")}`);
  }

  assert(offenders.length === 0, offenders.join("|"));
  return `ok:${objects.length}`;
})()
"#,
        )
        .expect("detached element own-property audit should evaluate");

    assert_eq!(result, "ok:120");
}

#[test]
fn legacy_dimension_accessors_use_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://legacy-dimensions-prototype.test/");

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
    assert(!!descriptor, `${prototype.constructor.name}.${name} missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const ownerChecks = [
    [HTMLHRElement.prototype, "size"],
    [HTMLHRElement.prototype, "width"],
    [HTMLFontElement.prototype, "size"],
    [HTMLMarqueeElement.prototype, "height"],
    [HTMLMarqueeElement.prototype, "width"],
    [HTMLTableElement.prototype, "width"],
    [HTMLTableColElement.prototype, "width"],
    [HTMLTableCellElement.prototype, "height"],
    [HTMLTableCellElement.prototype, "width"],
    [HTMLIFrameElement.prototype, "height"],
    [HTMLIFrameElement.prototype, "width"],
    [HTMLEmbedElement.prototype, "height"],
    [HTMLEmbedElement.prototype, "width"],
    [HTMLObjectElement.prototype, "height"],
    [HTMLObjectElement.prototype, "width"],
    [HTMLPreElement.prototype, "width"],
    [HTMLImageElement.prototype, "sizes"],
    [HTMLSourceElement.prototype, "sizes"],
    [HTMLSourceElement.prototype, "height"],
    [HTMLSourceElement.prototype, "width"]
  ];
  for (const [prototype, name] of ownerChecks) accessor(prototype, name);
  for (const name of ["size", "height", "width", "sizes"]) {
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);
    assert(!own(document.createElement("div"), name), `${name} should not be own on div`);
  }

  const parsed = new DOMParser().parseFromString(
    `<!doctype html><html><body>
      <hr><font></font><marquee></marquee><table><colgroup><col></colgroup><tbody><tr><td></td></tr></tbody></table>
      <iframe></iframe><embed><object></object><pre></pre><img><source>
    </body></html>`,
    "text/html"
  );
  const pairs = [
    [document.createElement("hr"), parsed.querySelector("hr"), [["size", "11", "11"], ["width", "33", "33"]]],
    [document.createElement("font"), parsed.querySelector("font"), [["size", "5", "5"]]],
    [document.createElement("marquee"), parsed.querySelector("marquee"), [["height", "44", "44"], ["width", "55", "55"]]],
    [document.createElement("table"), parsed.querySelector("table"), [["width", "66", "66"]]],
    [document.createElement("col"), parsed.querySelector("col"), [["width", "77", "77"]]],
    [document.createElement("td"), parsed.querySelector("td"), [["height", "88", "88"], ["width", "99", "99"]]],
    [document.createElement("iframe"), parsed.querySelector("iframe"), [["height", "101", "101"], ["width", "102", "102"]]],
    [document.createElement("embed"), parsed.querySelector("embed"), [["height", "103", "103"], ["width", "104", "104"]]],
    [document.createElement("object"), parsed.querySelector("object"), [["height", "105", "105"], ["width", "106", "106"]]],
    [document.createElement("pre"), parsed.querySelector("pre"), [["width", 107, 107]]],
    [document.createElement("img"), parsed.querySelector("img"), [["sizes", "10px", "10px"]]],
    [document.createElement("source"), parsed.querySelector("source"), [["sizes", "20px", "20px"], ["height", 108, 108], ["width", 109, 109]]]
  ];

  for (const [live, detached, checks] of pairs) {
    for (const element of [live, detached]) {
      for (const [name, value, expected] of checks) {
        assert(!own(element, name), `${element.localName}.${name} should not be own before set`);
        element[name] = value;
        assert(!own(element, name), `${element.localName}.${name} should not be own after set`);
        assert(element[name] === expected, `${element.localName}.${name} value`);
        assert(element.getAttribute(name) === String(expected), `${element.localName}.${name} attr`);
        assert(delete element[name], `${element.localName}.${name} delete`);
        assert(!own(element, name), `${element.localName}.${name} should stay inherited`);
        assert(element[name] === expected, `${element.localName}.${name} after delete`);
      }
    }
  }
  return "ok";
})()
"#,
        )
        .expect("legacy dimension owner prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_reflected_element_attributes_use_owner_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-reflected-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const div = doc.createElement("div");
  const section = doc.createElement("section");
  const svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
  doc.body.append(div, section, svg);

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.configurable === true, `${name} configurable`);
    assert(descriptor.writable === true, `${name} writable`);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);

  accessor(Element.prototype, "id", true);
  accessor(Element.prototype, "className", true);
  accessor(Element.prototype, "innerHTML", true);
  accessor(Element.prototype, "outerHTML", true);
  accessor(Element.prototype, "classList", true);
  accessor(Element.prototype, "part", true);
  accessor(Element.prototype, "attributes", false);
  accessor(Element.prototype, "shadowRoot", false);
  accessor(Element.prototype, "ariaLabel", true);
  accessor(Element.prototype, "ariaControlsElements", true);
  accessor(HTMLElement.prototype, "contentEditable", true);
  accessor(HTMLElement.prototype, "isContentEditable", false);
  method(Element.prototype, "getHTML");
  method(Element.prototype, "setHTMLUnsafe");
  assert(!own(HTMLElement.prototype, "id"), "id duplicated on HTMLElement");
  assert(!own(HTMLDivElement.prototype, "id"), "id duplicated on HTMLDivElement");
  assert(!own(Element.prototype, "contentEditable"), "contentEditable duplicated on Element");

  div.id = "alpha";
  div.className = "one two";
  section.id = "proxy";
  section.className = "proxy-class";
  section.classList.add("extra");
  section.part = "badge primary";
  section.ariaLabel = "Proxy label";
  section.contentEditable = "plaintext-only";
  section.innerHTML = "<b>x</b>";
  section.setAttribute("data-x", "1");
  const root = section.attachShadow({ mode: "open" });
  root.innerHTML = "<u>s</u>";
  svg.id = "svg-id";

  const names = [
    "id",
    "className",
    "innerHTML",
    "outerHTML",
    "getHTML",
    "setHTMLUnsafe",
    "classList",
    "part",
    "attributes",
    "shadowRoot",
    "ariaLabel",
    "ariaControlsElements",
    "contentEditable",
    "isContentEditable"
  ];
  for (const element of [div, section, svg]) {
    for (const name of names) {
      assert(!own(element, name), `${name} should not be own before delete`);
    }
  }
  assert(div.id === "alpha" && div.className === "one two", "div reflected values");
  assert(section.id === "proxy", "proxy id");
  assert(section.className === "proxy-class extra", "proxy className");
  assert(section.classList.value === "proxy-class extra", "proxy classList");
  assert(section.part.value === "badge primary", "proxy part");
  assert(section.attributes.length === 6, "proxy attributes length");
  assert(section.attributes.getNamedItem("data-x").value === "1", "proxy attributes item");
  assert(section.shadowRoot === root, "proxy shadowRoot");
  assert(section.shadowRoot.innerHTML === "<u>s</u>", "proxy shadowRoot content");
  assert(section.ariaLabel === "Proxy label", "proxy ariaLabel");
  assert(section.getAttribute("aria-label") === "Proxy label", "proxy aria-label attribute");
  const controls = [div];
  section.ariaControlsElements = controls;
  const reflectedControls = section.ariaControlsElements;
  assert(reflectedControls !== controls, "proxy ariaControlsElements snapshots input");
  assert(reflectedControls.length === 1 && reflectedControls[0] === div, "proxy ariaControlsElements");
  assert(Object.isFrozen(reflectedControls), "proxy ariaControlsElements frozen");
  assert(section.ariaControlsElements === reflectedControls, "proxy ariaControlsElements cached");
  assert(section.getAttribute("aria-controls") === "", "proxy aria-controls attribute");
  assert(!own(section, "ariaControlsElements"), "ariaControlsElements should stay inherited after set");
  assert(section.contentEditable === "plaintext-only", "proxy contentEditable");
  assert(section.isContentEditable === true, "proxy isContentEditable");
  assert(section.innerHTML === "<b>x</b>", "proxy innerHTML");
  assert(section.getHTML() === "<b>x</b>", "proxy getHTML");
  assert(svg.id === "svg-id", "svg id");
  assert(!("contentEditable" in svg), "SVG should not expose contentEditable");

  assert(delete section.id, "delete id");
  assert(delete section.className, "delete className");
  assert(delete section.contentEditable, "delete contentEditable");
  assert(delete section.innerHTML, "delete innerHTML");
  assert(delete section.getHTML, "delete getHTML");
  assert(delete section.setHTMLUnsafe, "delete setHTMLUnsafe");
  assert(delete section.classList, "delete classList");
  assert(delete section.part, "delete part");
  assert(delete section.attributes, "delete attributes");
  assert(delete section.shadowRoot, "delete shadowRoot");
  assert(delete section.ariaLabel, "delete ariaLabel");
  assert(delete section.ariaControlsElements, "delete ariaControlsElements");
  for (const name of names) {
    assert(!own(section, name), `${name} should not be own after delete`);
  }
  assert(section.id === "proxy", "proxy id after delete");
  assert(section.className === "proxy-class extra", "proxy className after delete");
  assert(section.classList.value === "proxy-class extra", "proxy classList after delete");
  assert(section.part.value === "badge primary", "proxy part after delete");
  assert(section.attributes.getNamedItem("data-x").value === "1", "proxy attributes after delete");
  assert(section.shadowRoot === root, "proxy shadowRoot after delete");
  assert(section.ariaLabel === "Proxy label", "proxy ariaLabel after delete");
  assert(section.ariaControlsElements === reflectedControls, "proxy ariaControlsElements after delete");
  assert(section.contentEditable === "plaintext-only", "proxy contentEditable after delete");
  assert(section.isContentEditable === true, "proxy isContentEditable after delete");
  assert(section.getHTML() === "<b>x</b>", "proxy getHTML after delete");
  section.setHTMLUnsafe("<i>y</i>");
  assert(section.innerHTML === "<i>y</i>", "proxy setHTMLUnsafe");
  return "ok";
})()
"#,
        )
        .expect("detached reflected attribute prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn aria_nullable_dom_string_reflection_removes_content_attributes() {
    let mut vm = new_storage_test_vm("https://aria-nullable-reflection.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const root = document.appendChild(document.createElement("html"));
  const connected = root.appendChild(document.createElement("div"));
  const detached = document.implementation.createHTMLDocument("").createElement("div");
  const roleDescriptor = Object.getOwnPropertyDescriptor(Element.prototype, "role");
  assert(typeof roleDescriptor?.get === "function", "role getter");
  assert(typeof roleDescriptor?.set === "function", "role setter");
  assert(roleDescriptor.enumerable && roleDescriptor.configurable, "role descriptor flags");
  const connectedText = connected.appendChild(document.createTextNode("connected"));
  const detachedText = detached.ownerDocument.createTextNode("detached");
  const throwsTypeError = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error instanceof TypeError;
    }
  };
  for (const receiver of [connectedText, detachedText, {}]) {
    assert(throwsTypeError(() => roleDescriptor.get.call(receiver)), "getter checks Element brand");
    assert(
      throwsTypeError(() => roleDescriptor.set.call(receiver, null)),
      "nullable setter checks Element brand"
    );
    assert(
      throwsTypeError(() => roleDescriptor.set.call(receiver, "button")),
      "string setter checks Element brand"
    );
  }

  for (const element of [connected, detached]) {
    assert(element.role === null, "missing role is null");
    assert(element.ariaAtomic === null, "missing ariaAtomic is null");
    element.setAttribute("role", "button");
    element.setAttribute("aria-atomic", "true");
    assert(element.role === "button", "role reads content attribute");
    assert(element.ariaAtomic === "true", "ariaAtomic reads content attribute");

    element.role = { toString() { return "checkbox"; } };
    element.ariaAtomic = 0;
    assert(element.getAttribute("role") === "checkbox", "role uses DOMString conversion");
    assert(element.getAttribute("aria-atomic") === "0", "ariaAtomic uses DOMString conversion");

    element.role = null;
    element.ariaAtomic = undefined;
    assert(element.role === null && !element.hasAttribute("role"), "null removes role");
    assert(
      element.ariaAtomic === null && !element.hasAttribute("aria-atomic"),
      "undefined removes ariaAtomic"
    );
    let symbolError = "";
    try {
      element.role = Symbol("role");
    } catch (error) {
      symbolError = error.name;
    }
    assert(symbolError === "TypeError", "Symbol conversion throws TypeError");
  }
  return "ok";
})()
"#,
        )
        .expect("nullable ARIA DOMString reflection probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_global_html_attributes_use_html_element_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-global-html-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const descriptorFor = (object, name) => {
    for (
      let prototype = Object.getPrototypeOf(object);
      prototype;
      prototype = Object.getPrototypeOf(prototype)
    ) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
      if (descriptor) return descriptor;
    }
  };
  const accessor = (object, name) => {
    const descriptor = descriptorFor(object, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const html = document.implementation.createHTMLDocument("");
  const parent = html.createElement("section");
  const detached = html.createElement("button");
  const live = document.createElement("button");
  parent.translate = false;
  parent.append(detached);

  const names = [
    "title",
    "lang",
    "autocapitalize",
    "autocorrect",
    "translate",
    "dir",
    "hidden",
    "inert",
    "accessKey",
    "draggable",
    "spellcheck",
    "writingSuggestions",
    "enterKeyHint",
    "inputMode",
    "autofocus",
    "tabIndex"
  ];
  const descriptors = Object.fromEntries(names.map((name) => [name, accessor(detached, name)]));
  assert(descriptors.translate.get.call(detached) === false, "detached translate should inherit");

  for (const [label, element] of [["live", live], ["detached", detached]]) {
    for (const name of names) {
      assert(!own(element, name), `${label}.${name} should not be own before set`);
    }

    descriptors.title.set.call(element, "Title");
    descriptors.lang.set.call(element, "en");
    descriptors.autocapitalize.set.call(element, "WORDS");
    descriptors.autocorrect.set.call(element, false);
    descriptors.dir.set.call(element, "rtl");
    descriptors.accessKey.set.call(element, "k");
    descriptors.enterKeyHint.set.call(element, "send");
    descriptors.inputMode.set.call(element, "email");
    descriptors.hidden.set.call(element, true);
    descriptors.inert.set.call(element, true);
    descriptors.autofocus.set.call(element, true);
    descriptors.translate.set.call(element, false);
    descriptors.draggable.set.call(element, true);
    descriptors.spellcheck.set.call(element, false);
    descriptors.writingSuggestions.set.call(element, false);
    descriptors.tabIndex.set.call(element, 7);

    assert(descriptors.title.get.call(element) === "Title", `${label}.title`);
    assert(descriptors.lang.get.call(element) === "en", `${label}.lang`);
    assert(descriptors.autocapitalize.get.call(element) === "words", `${label}.autocapitalize`);
    assert(descriptors.autocorrect.get.call(element) === false, `${label}.autocorrect`);
    assert(descriptors.dir.get.call(element) === "rtl", `${label}.dir`);
    assert(descriptors.accessKey.get.call(element) === "k", `${label}.accessKey`);
    assert(descriptors.enterKeyHint.get.call(element) === "send", `${label}.enterKeyHint`);
    assert(descriptors.inputMode.get.call(element) === "email", `${label}.inputMode`);
    assert(descriptors.hidden.get.call(element) === true, `${label}.hidden`);
    assert(descriptors.inert.get.call(element) === true, `${label}.inert`);
    assert(descriptors.autofocus.get.call(element) === true, `${label}.autofocus`);
    assert(descriptors.translate.get.call(element) === false, `${label}.translate`);
    assert(descriptors.draggable.get.call(element) === true, `${label}.draggable`);
    assert(descriptors.spellcheck.get.call(element) === false, `${label}.spellcheck`);
    assert(descriptors.writingSuggestions.get.call(element) === "false", `${label}.writingSuggestions`);
    assert(descriptors.tabIndex.get.call(element) === 7, `${label}.tabIndex`);
    assert(element.getAttribute("translate") === "no", `${label}.translate attr`);
    assert(element.getAttribute("draggable") === "true", `${label}.draggable attr`);
    assert(element.getAttribute("spellcheck") === "false", `${label}.spellcheck attr`);
    assert(element.getAttribute("writingsuggestions") === "false", `${label}.writingsuggestions attr`);
    assert(element.getAttribute("tabindex") === "7", `${label}.tabindex attr`);

    descriptors.hidden.set.call(element, false);
    descriptors.inert.set.call(element, false);
    descriptors.autofocus.set.call(element, false);
    assert(descriptors.hidden.get.call(element) === false, `${label}.hidden false`);
    assert(descriptors.inert.get.call(element) === false, `${label}.inert false`);
    assert(descriptors.autofocus.get.call(element) === false, `${label}.autofocus false`);
    assert(!element.hasAttribute("hidden"), `${label}.hidden removed`);
    assert(!element.hasAttribute("inert"), `${label}.inert removed`);
    assert(!element.hasAttribute("autofocus"), `${label}.autofocus removed`);

    for (const name of names) {
      assert(delete element[name], `${label}.${name} delete`);
      assert(!own(element, name), `${label}.${name} should stay inherited`);
    }
    assert(descriptors.title.get.call(element) === "Title", `${label}.title after delete`);
    assert(descriptors.tabIndex.get.call(element) === 7, `${label}.tabIndex after delete`);
  }
  return "ok";
})()
"#,
        )
        .expect("detached global HTML attribute prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn specialized_url_surfaces_use_owner_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://specialized-url-prototype.test/");

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
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  for (const prototype of [
    HTMLAnchorElement.prototype,
    HTMLAreaElement.prototype,
    HTMLBaseElement.prototype,
    HTMLLinkElement.prototype
  ]) {
    accessor(prototype, "href", true);
  }
  accessor(HTMLAnchorElement.prototype, "text", true);
  method(HTMLAnchorElement.prototype, "toString");
  for (const prototype of [
    HTMLImageElement.prototype,
    HTMLIFrameElement.prototype,
    HTMLSourceElement.prototype,
    HTMLEmbedElement.prototype
  ]) {
    accessor(prototype, "src", true);
  }
  accessor(HTMLIFrameElement.prototype, "srcdoc", true);
  accessor(HTMLIFrameElement.prototype, "contentDocument", false);
  accessor(HTMLIFrameElement.prototype, "contentWindow", false);
  assert(!own(HTMLElement.prototype, "href"), "href should not be on HTMLElement.prototype");
  assert(!own(HTMLElement.prototype, "src"), "src should not be on HTMLElement.prototype");

  const parsed = new DOMParser().parseFromString(
    '<html><head><base><link></head><body><a>Detached</a><area><iframe></iframe><img><source><embed></body></html>',
    'text/html'
  );
  const pairs = [
    [document.createElement("a"), parsed.querySelector("a"), "HTMLAnchorElement"],
    [document.createElement("area"), parsed.querySelector("area"), "HTMLAreaElement"],
    [document.createElement("base"), parsed.querySelector("base"), "HTMLBaseElement"],
    [document.createElement("link"), parsed.querySelector("link"), "HTMLLinkElement"]
  ];
  for (const [live, detached, name] of pairs) {
    for (const element of [live, detached]) {
      assert(!own(element, "href"), `${name}.href should not be own before set`);
      element.href = `https://example.test/${name}/path?q=1#hash`;
      assert(!own(element, "href"), `${name}.href should not be own after set`);
      assert(element.getAttribute("href") === `https://example.test/${name}/path?q=1#hash`, `${name}.href attribute`);
      assert(element.href === `https://example.test/${name}/path?q=1#hash`, `${name}.href value`);
      assert(delete element.href, `${name}.href delete`);
      assert(!own(element, "href"), `${name}.href should not be own after delete`);
      assert(element.href === `https://example.test/${name}/path?q=1#hash`, `${name}.href after delete`);
    }
  }

  const liveAnchor = pairs[0][0];
  const detachedAnchor = pairs[0][1];
  for (const anchor of [liveAnchor, detachedAnchor]) {
    anchor.text = "Updated";
    assert(!own(anchor, "text"), "anchor.text should not be own");
    assert(!own(anchor, "toString"), "anchor.toString should not be own");
    assert(anchor.text === "Updated", "anchor.text value");
    assert(anchor.toString() === anchor.href, "anchor.toString");
  }

  const srcPairs = [
    [document.createElement("img"), parsed.querySelector("img"), "HTMLImageElement"],
    [document.createElement("iframe"), parsed.querySelector("iframe"), "HTMLIFrameElement"],
    [document.createElement("source"), parsed.querySelector("source"), "HTMLSourceElement"],
    [document.createElement("embed"), parsed.querySelector("embed"), "HTMLEmbedElement"]
  ];
  for (const [live, detached, name] of srcPairs) {
    for (const element of [live, detached]) {
      assert(!own(element, "src"), `${name}.src should not be own before set`);
      element.src = `https://assets.test/${name}/asset.bin`;
      assert(!own(element, "src"), `${name}.src should not be own after set`);
      assert(element.getAttribute("src") === `https://assets.test/${name}/asset.bin`, `${name}.src attribute`);
      assert(element.src === `https://assets.test/${name}/asset.bin`, `${name}.src value`);
      assert(delete element.src, `${name}.src delete`);
      assert(!own(element, "src"), `${name}.src should not be own after delete`);
      assert(element.src === `https://assets.test/${name}/asset.bin`, `${name}.src after delete`);
    }
  }

  const liveFrame = document.createElement("iframe");
  const detachedFrame = parsed.querySelector("iframe");
  for (const frame of [liveFrame, detachedFrame]) {
    assert(!own(frame, "srcdoc"), "iframe.srcdoc should not be own before set");
    assert(!own(frame, "contentDocument"), "iframe.contentDocument should not be own");
    assert(!own(frame, "contentWindow"), "iframe.contentWindow should not be own");
    frame.srcdoc = "<p>child</p>";
    assert(!own(frame, "srcdoc"), "iframe.srcdoc should not be own after set");
    assert(frame.getAttribute("srcdoc") === "<p>child</p>", "iframe.srcdoc attribute");
    assert(frame.srcdoc === "<p>child</p>", "iframe.srcdoc value");
    assert(delete frame.srcdoc, "iframe.srcdoc delete");
    assert(!own(frame, "srcdoc"), "iframe.srcdoc should not be own after delete");
    assert(frame.srcdoc === "<p>child</p>", "iframe.srcdoc after delete");
  }

  return "ok";
})()
"#,
        )
        .expect("specialized URL prototype accessors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_shadow_root_surface_uses_shadow_root_prototype() {
    let mut vm = new_storage_test_vm("https://detached-shadow-root-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("section");
  doc.body.append(host);
  const root = host.attachShadow({
    mode: "open",
    delegatesFocus: true,
    slotAssignment: "manual",
    clonable: true,
    serializable: true,
    referenceTarget: "target-id"
  });

  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const accessor = (prototype, name, hasSetter) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert((typeof descriptor.set === "function") === hasSetter, `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.writable === true, `${name} writable`);
    assert(descriptor.configurable === true, `${name} configurable`);
  };

  const accessors = [
    ["host", false],
    ["mode", false],
    ["delegatesFocus", false],
    ["slotAssignment", false],
    ["clonable", false],
    ["serializable", false],
    ["referenceTarget", true],
    ["activeElement", false],
    ["innerHTML", true],
    ["styleSheets", false],
    ["adoptedStyleSheets", true]
  ];
  for (const [name, hasSetter] of accessors) {
    accessor(ShadowRoot.prototype, name, hasSetter);
  }
  for (const name of ["getHTML", "setHTMLUnsafe", "getSelection"]) {
    method(ShadowRoot.prototype, name);
  }
  method(Node.prototype, "cloneNode");

  const surface = accessors.map(([name]) => name).concat([
    "getHTML",
    "setHTMLUnsafe",
    "getSelection",
    "cloneNode"
  ]);
  for (const name of surface) {
    assert(!own(root, name), `${name} should not be own before use`);
  }

  root.innerHTML = '<style>.x{color:red}</style><button id="target-id">go</button>';
  const button = root.querySelector("button");
  assert(root.host === host, "host identity");
  assert(root.mode === "open", "mode");
  assert(root.delegatesFocus === true, "delegatesFocus");
  assert(root.slotAssignment === "manual", "slotAssignment");
  assert(root.clonable === true, "clonable");
  assert(root.serializable === true, "serializable");
  assert(root.referenceTarget === "target-id", "referenceTarget init");
  root.referenceTarget = null;
  assert(root.referenceTarget === null, "referenceTarget null");
  root.referenceTarget = true;
  assert(root.referenceTarget === "true", "referenceTarget string");
  assert(button && button.id === "target-id", "innerHTML parsed");
  assert(root.getHTML().includes('id="target-id"'), "getHTML behavior");
  assert(root.styleSheets === root.styleSheets, "styleSheets stable wrapper");
  assert(typeof root.styleSheets.length === "number", "styleSheets length shape");
  assert(Array.isArray(root.adoptedStyleSheets), "adoptedStyleSheets array");
  root.adoptedStyleSheets = [];
  assert(Array.isArray(root.adoptedStyleSheets), "adoptedStyleSheets setter");
  assert(root.getSelection() === null, "getSelection detached document");
  const focusButton = doc.createElement("button");
  focusButton.id = "focus-target";
  root.append(focusButton);
  focusButton.focus();
  assert(root.activeElement === focusButton, "detached shadow activeElement");
  root.setHTMLUnsafe("<em>done</em>");
  assert(root.innerHTML === "<em>done</em>", "setHTMLUnsafe behavior");

  const cloneResult = (() => {
    try {
      root.cloneNode(true);
      return "no-throw";
    } catch (error) {
      return `${error.name}:${error.code}:${error instanceof DOMException}`;
    }
  })();
  assert(cloneResult === "NotSupportedError:9:true", "cloneNode behavior");

  for (const name of surface) {
    assert(delete root[name], `${name} delete`);
    assert(!own(root, name), `${name} should not be own after delete`);
  }
  assert(root.host === host, "host after delete");
  assert(root.referenceTarget === "true", "referenceTarget after delete");
  return "ok";
})()
"#,
        )
        .expect("detached ShadowRoot prototype surface should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_slot_brand_checks_accept_standard_prototype_methods() {
    let mut vm = new_storage_test_vm("https://detached-slot-brand-check.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const method = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.value === "function", `${name} method`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor.value;
  };
  const accessor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(descriptor.configurable === true, `${name} configurable`);
    return descriptor;
  };

  const assignedNodes = method(HTMLSlotElement.prototype, "assignedNodes");
  const assignedElements = method(HTMLSlotElement.prototype, "assignedElements");
  const assign = method(HTMLSlotElement.prototype, "assign");
  const elementAssignedSlot = accessor(Element.prototype, "assignedSlot");
  const elementSlot = accessor(Element.prototype, "slot");
  const slotName = accessor(HTMLSlotElement.prototype, "name");
  const textAssignedSlot = accessor(Text.prototype, "assignedSlot");
  assert(typeof elementSlot.set === "function", "Element.prototype.slot setter");
  assert(typeof slotName.set === "function", "HTMLSlotElement.prototype.name setter");

  const doc = document.implementation.createHTMLDocument("");
  const host = doc.createElement("section");
  doc.body.append(host);
  const root = host.attachShadow({ mode: "open", slotAssignment: "manual" });
  const slot = doc.createElement("slot");
  const text = doc.createTextNode("alpha");
  const span = doc.createElement("span");
  elementSlot.set.call(span, "main");
  slotName.set.call(slot, "main");
  host.append(text, span);
  root.append(slot);

  assign.call(slot, text, span);
  const nodes = assignedNodes.call(slot);
  const elements = assignedElements.call(slot);

  assert(nodes.length === 2, "assignedNodes length");
  assert(Array.prototype.includes.call(nodes, text), "assignedNodes text");
  assert(Array.prototype.includes.call(nodes, span), "assignedNodes span");
  assert(elements.length === 1, "assignedElements length");
  assert(elements[0] === span, "assignedElements span");
  assert(elementAssignedSlot.get.call(span) === slot, "element assignedSlot");
  assert(textAssignedSlot.get.call(text) === slot, "text assignedSlot");
  assert(elementSlot.get.call(span) === "main", "Element.prototype.slot getter");
  assert(slotName.get.call(slot) === "main", "HTMLSlotElement.prototype.name getter");
  assert(span.getAttribute("slot") === "main", "slot reflected attribute");
  assert(slot.getAttribute("name") === "main", "name reflected attribute");

  for (const [object, names] of [
    [slot, ["name", "assignedNodes", "assignedElements", "assign"]],
    [span, ["slot", "assignedSlot"]],
    [text, ["assignedSlot"]]
  ]) {
    for (const name of names) {
      assert(!own(object, name), `${name} should not be own`);
    }
  }
  assert(delete slot.name, "delete inherited slot name");
  assert(delete span.slot, "delete inherited element slot");
  assert(!own(slot, "name"), "name should stay inherited after delete");
  assert(!own(span, "slot"), "slot should stay inherited after delete");
  assert(slotName.get.call(slot) === "main", "slot name after delete");
  assert(elementSlot.get.call(span) === "main", "element slot after delete");
  return "ok";
})()
"#,
        )
        .expect("detached slot prototype brand checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_document_view_uses_document_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://detached-document-view-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const docs = [
    document.implementation.createHTMLDocument(""),
    document.implementation.createDocument("urn:test", "root", null),
    new DOMParser().parseFromString("<html><body></body></html>", "text/html")
  ];
  const descriptorShape = (name) => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, name);
    return [
      !!descriptor,
      typeof descriptor.get,
      descriptor.set === undefined,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":");
  };
  const own = (doc, name) => Object.prototype.hasOwnProperty.call(doc, name);
  const before = docs.map((doc) => [
    doc.defaultView === null,
    typeof doc.parentWindow,
    own(doc, "defaultView"),
    "parentWindow" in doc,
    Object.keys(doc).includes("defaultView"),
    Object.keys(doc).includes("parentWindow")
  ].join(",")).join("|");
  const deleteResult = delete docs[0].defaultView;
  docs[0].defaultView = window;
  const parentWindowDeleteResult = delete docs[0].parentWindow;
  return [
    descriptorShape("defaultView"),
    Object.getOwnPropertyDescriptor(Document.prototype, "parentWindow") === undefined,
    before,
    deleteResult,
    parentWindowDeleteResult,
    docs[0].defaultView === null,
    typeof docs[0].parentWindow,
    own(docs[0], "defaultView"),
    own(docs[0], "parentWindow")
  ].join("||");
})()
"#,
        )
        .expect("detached document view prototype accessors should evaluate");

    assert_eq!(
        result,
        "true:function:true:true:true||true||true,undefined,false,false,false,false|true,undefined,false,false,false,false|true,undefined,false,false,false,false||true||true||true||undefined||false||false"
    );
}

#[test]
fn table_legacy_dom_string_reflectors_use_owner_prototype() {
    let mut vm = new_storage_test_vm("https://table-legacy-dom-string-reflectors.test/");

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
  const cases = [
    ["frame", "frame", false],
    ["rules", "rules", false],
    ["summary", "summary", false],
    ["cellPadding", "cellpadding", true],
    ["cellSpacing", "cellspacing", true]
  ];
  const detachedDocument = document.implementation.createHTMLDocument("");

  for (const [name, attribute, nullAsEmpty] of cases) {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTableElement.prototype, name);
    assert(!!descriptor, `${name} descriptor missing`);
    assert(typeof descriptor.get === "function", `${name} getter`);
    assert(typeof descriptor.set === "function", `${name} setter`);
    assert(descriptor.enumerable === true, `${name} enumerable`);
    assert(descriptor.configurable === true, `${name} configurable`);
    assert(!own(HTMLElement.prototype, name), `${name} should not be on HTMLElement.prototype`);

    for (const [doc, label] of [[document, "live"], [detachedDocument, "detached"]]) {
      const table = doc.createElement("table");
      assert(!own(table, name), `${label}.${name} should not be own before set`);
      assert(table[name] === "", `${label}.${name} missing-value default`);
      table[name] = { toString: () => `${name}-value` };
      assert(table[name] === `${name}-value`, `${label}.${name} getter`);
      assert(table.getAttribute(attribute) === `${name}-value`, `${label}.${name} attribute`);
      table[name] = null;
      const expectedNull = nullAsEmpty ? "" : "null";
      assert(table[name] === expectedNull, `${label}.${name} null getter`);
      assert(table.getAttribute(attribute) === expectedNull, `${label}.${name} null attribute`);
      assert(!own(table, name), `${label}.${name} should stay inherited after set`);
      assert(delete table[name], `${label}.${name} delete`);
      assert(!own(table, name), `${label}.${name} should stay inherited after delete`);
      assert(table[name] === expectedNull, `${label}.${name} after delete`);
    }

    for (const receiver of [document.createElement("div"), {}]) {
      assert(throwsTypeError(() => descriptor.get.call(receiver)), `${name} getter receiver`);
      assert(throwsTypeError(() => descriptor.set.call(receiver, "wrong")), `${name} setter receiver`);
    }
  }
  return "ok";
})()
"#,
        )
        .expect("table legacy DOMString reflectors should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn detached_iframe_windows_do_not_create_document_browsing_contexts() {
    let mut vm = new_storage_test_vm("https://detached-iframe-visibility.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errors = [];
  const check = (condition, message) => { if (!condition) errors.push(message); };
  const factories = [
    ["createHTMLDocument", () => document.implementation.createHTMLDocument("")],
    ["DOMParser", () => new DOMParser().parseFromString("<body></body>", "text/html")],
    ["createDocument", () => document.implementation.createDocument("urn:test", "root", null)]
  ];
  for (const [name, create] of factories) {
    for (const first of ["document", "window"]) {
      const outer = create();
      const frame = outer.createElementNS("http://www.w3.org/1999/xhtml", "iframe");
      frame.srcdoc = "<p>hello</p>";
      outer.documentElement.appendChild(frame);
      const label = `${name}/${first}`;
      check(outer.defaultView === null, `${label}: windowless owner`);
      let child;
      if (first === "document") {
        child = frame.contentDocument;
        check(child.hidden && child.visibilityState === "hidden", `${label}: before contentWindow`);
      }
      const view = frame.contentWindow;
      child ||= frame.contentDocument;
      check(view !== null && view.document === child, `${label}: synthetic window`);
      check(child.defaultView === null, `${label}: synthetic window has no browsing context`);
      check(child.hidden && child.visibilityState === "hidden", `${label}: after contentWindow`);
      check(outer.hidden && outer.visibilityState === "hidden", `${label}: owner stays hidden`);
      const nested = child.createElement("iframe");
      nested.srcdoc = "<p>nested</p>";
      child.body.appendChild(nested);
      const nestedChild = nested.contentDocument;
      check(nestedChild.hidden && nestedChild.visibilityState === "hidden", `${label}: nested before contentWindow`);
      check(nested.contentWindow.document === nestedChild, `${label}: nested window`);
      check(nested.contentWindow.parent === view, `${label}: nested compatibility parent`);
      check(nestedChild.defaultView === null, `${label}: nested synthetic window has no browsing context`);
      check(nestedChild.hidden && nestedChild.visibilityState === "hidden", `${label}: nested after contentWindow`);
    }
  }
  return JSON.stringify(errors);
})()
"#,
        )
        .expect("detached iframe visibility probe should evaluate");

    assert_eq!(result, "[]");
}
