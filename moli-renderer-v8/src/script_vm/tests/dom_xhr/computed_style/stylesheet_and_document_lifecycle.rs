use super::*;

#[test]
fn text_value_change_invalidates_placeholder_shown_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-placeholder-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(255, 0, 0); }
    #input:placeholder-shown + #inputTarget { color: rgb(0, 128, 0); }
    #textarea:placeholder-shown + #textareaTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const input = document.createElement('input');
  input.id = 'input';
  input.placeholder = 'placeholder';
  const inputTarget = document.createElement('span');
  inputTarget.id = 'inputTarget';
  inputTarget.className = 'target';
  const textarea = document.createElement('textarea');
  textarea.id = 'textarea';
  textarea.placeholder = 'placeholder';
  const textareaTarget = document.createElement('span');
  textareaTarget.id = 'textareaTarget';
  textareaTarget.className = 'target';
  document.body.append(input, inputTarget, textarea, textareaTarget);

  const inputStyle = getComputedStyle(inputTarget);
  const textareaStyle = getComputedStyle(textareaTarget);
  const before = [inputStyle.color, textareaStyle.color].join(',');
  input.value = 'typed';
  textarea.value = 'typed';
  const afterFilled = [inputStyle.color, textareaStyle.color].join(',');
  input.value = '';
  textarea.value = '';
  const afterEmpty = [inputStyle.color, textareaStyle.color].join(',');
  return `${before}|${afterFilled}|${afterEmpty}`;
})()
"#,
        )
        .expect("placeholder-shown value invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(0, 0, 255)|rgb(255, 0, 0),rgb(255, 0, 0)|rgb(0, 128, 0),rgb(0, 0, 255)"
    );
}

#[test]
fn text_value_change_invalidates_validity_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-validity-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(255, 0, 0); }
    #required:invalid + #invalidTarget { color: rgb(0, 128, 0); }
    #required:valid ~ #validTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const required = document.createElement('input');
  required.id = 'required';
  required.required = true;
  const invalidTarget = document.createElement('span');
  invalidTarget.id = 'invalidTarget';
  invalidTarget.className = 'target';
  const validTarget = document.createElement('span');
  validTarget.id = 'validTarget';
  validTarget.className = 'target';

  document.body.append(
    required,
    invalidTarget,
    validTarget
  );

  const invalidStyle = getComputedStyle(invalidTarget);
  const validStyle = getComputedStyle(validTarget);
  const before = [
    invalidStyle.color,
    validStyle.color
  ].join(',');
  required.value = 'filled';
  const after = [
    invalidStyle.color,
    validStyle.color
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("validity state invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(0, 0, 255)"
    );
}

#[test]
fn child_list_change_invalidates_form_and_fieldset_validity_computed_style() {
    let mut vm = new_parsed_test_vm(
        "https://computed-style-validity-child-list-invalidation.test/",
        r#"<!doctype html><html><head><style>
          form, fieldset { background-color: rgb(0, 128, 0); }
          form:invalid, fieldset:invalid { background-color: rgb(0, 255, 0); }
          .target { color: rgb(255, 0, 0); }
          #form:invalid + #form-target { color: rgb(0, 0, 255); }
          #fieldset:invalid + #fieldset-target { color: rgb(1, 2, 3); }
        </style></head><body>
          <form id="form"></form><span id="form-target" class="target"></span>
          <fieldset id="fieldset"></fieldset><span id="fieldset-target" class="target"></span>
        </body></html>"#,
    );

    let result = vm
        .eval(
            r#"
(() => {
  const form = document.getElementById('form');
  const fieldset = document.getElementById('fieldset');
  const formTarget = document.getElementById('form-target');
  const fieldsetTarget = document.getElementById('fieldset-target');
  const invalid = document.createElement('input');
  invalid.type = 'number';
  invalid.min = '8';
  invalid.value = '4';
  const state = () => [
    getComputedStyle(form).backgroundColor,
    getComputedStyle(formTarget).color,
    getComputedStyle(fieldset).backgroundColor,
    getComputedStyle(fieldsetTarget).color
  ].join(',');

  const initial = state();
  form.append(invalid);
  const inForm = state();
  fieldset.append(invalid);
  const inFieldset = state();
  invalid.remove();
  const removed = state();
  return [initial, inForm, inFieldset, removed].join('|');
})()
"#,
        )
        .expect("child-list validity invalidation should evaluate");

    assert_eq!(
        result,
        concat!(
            "rgb(0, 128, 0),rgb(255, 0, 0),rgb(0, 128, 0),rgb(255, 0, 0)|",
            "rgb(0, 255, 0),rgb(0, 0, 255),rgb(0, 128, 0),rgb(255, 0, 0)|",
            "rgb(0, 128, 0),rgb(255, 0, 0),rgb(0, 255, 0),rgb(1, 2, 3)|",
            "rgb(0, 128, 0),rgb(255, 0, 0),rgb(0, 128, 0),rgb(255, 0, 0)"
        )
    );
}

#[test]
fn text_value_change_invalidates_range_computed_style() {
    let mut vm = new_storage_test_vm("https://computed-style-range-invalidation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const style = document.createElement('style');
  style.textContent = `
    .target { color: rgb(255, 0, 0); }
    #range:in-range + #inRangeTarget { color: rgb(0, 128, 0); }
    #range:out-of-range ~ #outRangeTarget { color: rgb(0, 0, 255); }`;
  (document.head || document.documentElement || document).appendChild(style);

  const range = document.createElement('input');
  range.id = 'range';
  range.type = 'number';
  range.min = '0';
  range.max = '10';
  range.value = '5';
  const inRangeTarget = document.createElement('span');
  inRangeTarget.id = 'inRangeTarget';
  inRangeTarget.className = 'target';
  const outRangeTarget = document.createElement('span');
  outRangeTarget.id = 'outRangeTarget';
  outRangeTarget.className = 'target';

  document.body.append(
    range,
    inRangeTarget,
    outRangeTarget
  );

  const inRangeStyle = getComputedStyle(inRangeTarget);
  const outRangeStyle = getComputedStyle(outRangeTarget);
  const before = [
    inRangeStyle.color,
    outRangeStyle.color
  ].join(',');
  range.value = '20';
  const after = [
    inRangeStyle.color,
    outRangeStyle.color
  ].join(',');
  return `${before}|${after}`;
})()
"#,
        )
        .expect("range state invalidation should evaluate");

    assert_eq!(
        result,
        "rgb(0, 128, 0),rgb(255, 0, 0)|rgb(255, 0, 0),rgb(0, 0, 255)"
    );
}

#[test]
fn computed_style_resolves_pseudo_element_argument() {
    let mut vm = new_storage_test_vm("https://computed-pseudo.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #target { width: 100px; color: rgb(255, 0, 0); }
    #target::before { content: "x"; width: 50%; color: rgb(0, 128, 0); }
    #target::after { content: "y"; width: 25px; color: rgb(0, 0, 255); }
    #target::checkmark { width: 17px; }
    #file::file-selector-button { color: rgb(0, 255, 0); }
    #select::picker(select) { background-color: rgb(0, 128, 0); }
    #select::picker-icon { width: 13px; }
    #picker-div::picker(select) { background-color: rgb(0, 0, 255); }
    #item { display: list-item; }
    #item::marker { color: rgb(10, 20, 30); }
    #input::placeholder { color: rgb(30, 20, 10); }
    #target::highlight(name) { color: rgb(0, 0, 255); }
    #target::highlight(other) { color: rgb(255, 0, 255); }
    #plain { font-style: italic; }`;
  (document.head || document.documentElement || document).appendChild(style);
  const target = document.createElement('div');
  target.id = 'target';
  (document.body || document.documentElement || document).appendChild(target);
  const file = document.createElement('input');
  file.id = 'file';
  file.type = 'file';
  (document.body || document.documentElement || document).appendChild(file);
  const select = document.createElement('select');
  select.id = 'select';
  select.style.width = '321px';
  (document.body || document.documentElement || document).appendChild(select);
  const pickerDiv = document.createElement('div');
  pickerDiv.id = 'picker-div';
  (document.body || document.documentElement || document).appendChild(pickerDiv);
  const pickerDivNoRules = document.createElement('div');
  pickerDivNoRules.id = 'picker-div-no-rules';
  (document.body || document.documentElement || document).appendChild(pickerDivNoRules);
  const item = document.createElement('li');
  item.id = 'item';
  (document.body || document.documentElement || document).appendChild(item);
  const input = document.createElement('input');
  input.id = 'input';
  input.placeholder = 'placeholder';
  (document.body || document.documentElement || document).appendChild(input);
  const plain = document.createElement('div');
  plain.id = 'plain';
  (document.body || document.documentElement || document).appendChild(plain);
  const heldBefore = getComputedStyle(target, '::before');
  const heldAfter = getComputedStyle(target, '::after');
  const heldOrigin = getComputedStyle(target);
  return [
    getComputedStyle(target, 'before').width,
    getComputedStyle(target, ':before').width,
    getComputedStyle(target, '::before').width,
    getComputedStyle(target, '::after').width,
    getComputedStyle(target, '::before(test)').length,
    getComputedStyle(target, ':checkmark').width,
    getComputedStyle(target, '::checkmark').width,
    getComputedStyle(target, 'file-selector-button').color,
    getComputedStyle(file, '::file-selector-button').color,
    getComputedStyle(select, 'picker-icon').width,
    getComputedStyle(select, ':picker-icon').width,
    getComputedStyle(select, '::picker-icon').width,
    getComputedStyle(select, '::picker(select)').backgroundColor,
    getComputedStyle(pickerDiv, '::picker(select)').backgroundColor,
    getComputedStyle(pickerDivNoRules, '::picker(select)').backgroundColor,
    getComputedStyle(select, '::picker(div)').length,
    getComputedStyle(item, '::marker').color,
    getComputedStyle(input, '::placeholder').color,
    getComputedStyle(target, '::highlight(n\\61me)').color,
    getComputedStyle(target, '::highlight(other)').color,
    getComputedStyle(plain).getPropertyValue('font-style'),
    getComputedStyle(plain, '::before').getPropertyValue('font-style'),
    heldBefore.width,
    heldAfter.width,
    heldOrigin.width,
    heldBefore.width
  ].join('|');
})()
"#,
        )
        .expect("computed pseudo element style should evaluate");

    assert_eq!(
        result,
        "100px|50px|50px|25px|0||17px|rgb(255, 0, 0)|rgb(0, 255, 0)|321px||13px|rgb(0, 128, 0)|rgb(0, 0, 255)|rgba(0, 0, 0, 0)|0|rgb(10, 20, 30)|rgb(30, 20, 10)|rgb(0, 0, 255)|rgb(255, 0, 255)|italic|italic|50px|25px|100px|50px"
    );
}

#[test]
fn computed_style_resolves_highlight_font_relative_text_decoration_properties() {
    let mut vm = new_storage_test_vm("https://highlight-font-relative-computed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  const style = document.createElement('style');
  style.textContent = `
    :root { font-size: 16px; }
    div { font-size: 20px; }
    ::highlight(highlight1) {
      text-underline-offset: 0.5em;
      text-decoration-line: underline;
      text-decoration-color: green;
      text-decoration-thickness: 0.25rem;
    }
    body > div[data-kind="one"]::highlight(highlight1) {
      text-underline-offset: 0.75em;
      text-decoration-thickness: 0.5rem;
    }
    #h2::highlight(highlight1) {
      text-underline-offset: 1.0em;
      text-decoration-line: underline;
      text-decoration-color: blue;
      text-decoration-thickness: 0.125rem;
    }`;
  (document.head || document.documentElement || document).appendChild(style);
  const h1 = document.createElement('div');
  h1.id = 'h1';
  h1.dataset.kind = 'one';
  h1.textContent = 'one';
  const h2 = document.createElement('div');
  h2.id = 'h2';
  h2.dataset.kind = 'two';
  h2.textContent = 'two';
  const body = document.body || document.createElement('body');
  if (!body.parentNode) {
    document.documentElement.appendChild(body);
  }
  body.appendChild(h1);
  body.appendChild(h2);
  const r1 = document.createRange();
  r1.setStart(h1, 0);
  r1.setEnd(h1, 1);
  const r2 = document.createRange();
  r2.setStart(h2, 0);
  r2.setEnd(h2, 1);
  CSS.highlights.set('highlight1', new Highlight(r1, r2));
  const pseudo = '::highlight(highlight1)';
  const rootStyle = getComputedStyle(document.documentElement, pseudo);
  const h1Style = getComputedStyle(h1, pseudo);
  const h2Style = getComputedStyle(h2, pseudo);
  return [
    rootStyle.textUnderlineOffset,
    rootStyle.textDecorationThickness,
    h1Style.textUnderlineOffset,
    h1Style.textDecorationThickness,
    h2Style.textUnderlineOffset,
    h2Style.textDecorationThickness
  ].join('|');
})()
"#,
        )
        .expect("Highlight font-relative computed properties should evaluate");

    assert_eq!(result, "8px|4px|15px|8px|20px|2px");
}

#[test]
fn file_selector_button_pseudo_only_border_style_is_named_property() {
    let mut vm = new_storage_test_vm("https://file-selector-button-pseudo-style.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `::file-selector-button { border: 3px double black; }`;
  (document.head || document.documentElement || document).appendChild(style);
  const file = document.createElement('input');
  file.type = 'file';
  (document.body || document.documentElement || document).appendChild(file);

  const host = document.createElement('div');
  (document.body || document.documentElement || document).appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = `
    <style>::file-selector-button { border: 5px dotted black; }</style>
    <input type="file" id="shadow-file">
  `;

  const documentStyle = getComputedStyle(file, '::file-selector-button');
  const shadowStyle = getComputedStyle(
    shadow.getElementById('shadow-file'),
    '::file-selector-button'
  );
  return JSON.stringify({
    documentNamed: documentStyle.borderTopStyle,
    documentMethod: documentStyle.getPropertyValue('border-top-style'),
    shadowNamed: shadowStyle.borderTopStyle,
    shadowMethod: shadowStyle.getPropertyValue('border-top-style')
  });
})()
"#,
        )
        .expect("file selector button pseudo-only border style should evaluate");

    assert_eq!(
        result,
        r#"{"documentNamed":"double","documentMethod":"double","shadowNamed":"dotted","shadowMethod":"dotted"}"#
    );
}

#[test]
fn lazy_pseudo_computed_style_reuses_and_clears_cache() {
    let mut vm = new_storage_test_vm("https://lazy-pseudo-computed-cache.test/");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || root.appendChild(document.createElement('head'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = '#file::file-selector-button { color: rgb(0, 255, 0); }';
  head.appendChild(style);

  globalThis.__lazyPseudoFile = document.createElement('input');
  globalThis.__lazyPseudoFile.id = 'file';
  globalThis.__lazyPseudoFile.type = 'file';
  body.appendChild(globalThis.__lazyPseudoFile);

  const buttonStyle = getComputedStyle(globalThis.__lazyPseudoFile, '::file-selector-button');
  return [buttonStyle.color, buttonStyle.backgroundColor].join('|');
})()
"#,
        )
        .expect("lazy pseudo cache setup should evaluate");

    assert_eq!(result, "rgb(0, 255, 0)|rgba(0, 0, 0, 0)");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        2
    );

    let removed = vm
        .eval(
            r#"
(() => {
  globalThis.__lazyPseudoFile.remove();
  delete globalThis.__lazyPseudoFile;
  return 'removed';
})()
"#,
        )
        .expect("lazy pseudo cached node removal should evaluate");

    assert_eq!(removed, "removed");
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        0
    );
}

#[test]
fn computed_style_uses_stylesheet_source_base_urls() {
    let mut vm = new_storage_test_vm("https://stylesheet-base.test/page/index.html");
    let stylesheet_url = url::Url::parse("https://stylesheet-base.test/assets/app.css").unwrap();
    let stylesheet_final_url =
        url::Url::parse("https://stylesheet-base.test/final/app.css").unwrap();
    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || document.documentElement || document;
  const base = document.createElement('base');
  base.href = 'https://stylesheet-base.test/base/';
  head.appendChild(base);

  const link = document.createElement('link');
  link.id = 'source-base-link';
  link.rel = 'stylesheet';
  link.href = '../assets/app.css';
  head.appendChild(link);
  if (!document.body) {
    html.appendChild(document.createElement('body'));
  }

})()
"#,
    )
    .expect("linked stylesheet base URL setup should evaluate");
    let link = element_handle_by_id(&vm, "source-base-link");
    install_linked_stylesheet_for_test(
        &mut vm,
        link,
        stylesheet_url.clone(),
        crate::style_engine::StyloStylesheetSource::new(
            "body { background-image: url(img/linked.png); }".to_owned(),
            stylesheet_final_url,
        )
        .with_sheet_url(stylesheet_url),
    );

    let result = vm
        .eval(
            r#"
(() => {
  const linked = getComputedStyle(document.body).backgroundImage;
  const sheet = new CSSStyleSheet({ baseURL: 'https://constructed-base.test/styles/sheet.css' });
  sheet.replaceSync('body { background-image: url(img/adopted.png); }');
  document.adoptedStyleSheets = [sheet];
  const adopted = getComputedStyle(document.body).backgroundImage;

  return `${linked}|${adopted}`;
})()
"#,
        )
        .expect("computed stylesheet base URL probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://stylesheet-base.test/final/img/linked.png")|url("https://constructed-base.test/styles/img/adopted.png")"#
    );
}

#[test]
fn inline_style_sheet_keeps_processing_base_across_unrelated_source_set_rebuild() {
    let mut vm = new_storage_test_vm("https://inline-sheet-base.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-base.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  style.textContent = '.subject { background-image: url(img.png); }';
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  const before = getComputedStyle(subject).backgroundImage;
  base.href = 'https://inline-sheet-base.test/new/';

  const unrelated = document.createElement('style');
  unrelated.textContent = '.unrelated { color: red; }';
  head.appendChild(unrelated);
  const after = getComputedStyle(subject).backgroundImage;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("inline stylesheet frozen parser-base probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-base.test/old/img.png")|url("https://inline-sheet-base.test/old/img.png")"#
    );
}

#[test]
fn inline_style_sheet_content_reprocessing_captures_the_current_base() {
    let mut vm = new_storage_test_vm("https://inline-sheet-reprocess.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-reprocess.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  style.textContent = '.subject { background-image: url(before.png); }';
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  const before = getComputedStyle(subject).backgroundImage;
  base.href = 'https://inline-sheet-reprocess.test/new/';
  style.textContent = '.subject { background-image: url(after.png); }';
  const after = getComputedStyle(subject).backgroundImage;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("inline stylesheet content reprocessing probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-reprocess.test/old/before.png")|url("https://inline-sheet-reprocess.test/new/after.png")"#
    );
}

#[test]
fn reconnected_inline_style_sheet_is_processed_with_the_current_base() {
    let mut vm = new_storage_test_vm("https://inline-sheet-reconnect.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-reconnect.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  style.textContent = '.subject { background-image: url(image.png); }';
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  const before = getComputedStyle(subject).backgroundImage;
  style.remove();
  base.href = 'https://inline-sheet-reconnect.test/new/';
  head.appendChild(style);
  const after = getComputedStyle(subject).backgroundImage;
  return `${before}|${after}`;
})()
"#,
        )
        .expect("inline stylesheet reconnect probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-reconnect.test/old/image.png")|url("https://inline-sheet-reconnect.test/new/image.png")"#
    );
}

#[test]
fn inline_cssom_rule_edit_keeps_the_sheet_parser_context() {
    let mut vm = new_storage_test_vm("https://inline-sheet-cssom.test/page/index.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const base = document.createElement('base');
  base.href = 'https://inline-sheet-cssom.test/old/';
  head.appendChild(base);

  const style = document.createElement('style');
  head.appendChild(style);
  const subject = document.createElement('div');
  subject.className = 'subject';
  body.appendChild(subject);

  base.href = 'https://inline-sheet-cssom.test/new/';
  style.sheet.insertRule('.subject { background-image: url(inserted.png); }', 0);
  return getComputedStyle(subject).backgroundImage;
})()
"#,
        )
        .expect("inline stylesheet CSSOM parser-context probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://inline-sheet-cssom.test/old/inserted.png")"#
    );
}

#[test]
fn inline_cssom_rule_inserted_before_target_connection_applies_later() {
    let mut vm = new_storage_test_vm("https://inline-sheet-before-target.test/");

    vm.eval(
        r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.appendChild(document.createTextNode(''));
  head.appendChild(style);
  style.sheet.insertRule('#target { margin-left: 2px; }');
})()
"#,
    )
    .expect("inline CSSOM rule setup should evaluate");

    vm.sync_live_document_style_sources();

    let result = vm
        .eval(
            r#"
(() => {
  const body = document.body;
  const target = document.createElement('div');
  target.id = 'target';
  body.appendChild(target);
  const beforeDomMutation = getComputedStyle(target).marginLeft;
  document.querySelector('style').textContent = '#target { margin-left: 3px; }';
  const afterDomMutation = getComputedStyle(target).marginLeft;
  return `${beforeDomMutation}|${afterDomMutation}`;
})()
"#,
        )
        .expect("inline CSSOM rule should apply to targets connected later");

    assert_eq!(result, "2px|3px");
}

#[test]
fn constructed_css_stylesheet_insert_rule_uses_constructor_document_base_url() {
    let mut vm = new_storage_test_vm("https://constructable-base.test/css/cssom/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const sheet = new CSSStyleSheet();
  sheet.insertRule(':root { background-image: url("../../images/green.png"); }');
  const base = document.createElement('base');
  base.href = 'https://constructable-base.test/changed/';
  head.appendChild(base);
  document.adoptedStyleSheets = [sheet];
  return getComputedStyle(html).backgroundImage;
})()
"#,
        )
        .expect("constructed insertRule base URL probe should evaluate");

    assert_eq!(
        result,
        r#"url("https://constructable-base.test/images/green.png")"#
    );
}

#[test]
fn computed_style_exposes_reading_flow_and_order() {
    let mut vm = new_storage_test_vm("https://reading-flow-computed.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #container { reading-flow: grid-order; }
    #child { reading-order: -2; }`;
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  (document.head || html).appendChild(style);
  const container = document.createElement('div');
  container.id = 'container';
  const child = document.createElement('button');
  child.id = 'child';
  container.appendChild(child);
  (document.body || html.appendChild(document.createElement('body'))).appendChild(container);
  const containerStyle = getComputedStyle(container);
  const childStyle = getComputedStyle(child);
  const unset = getComputedStyle(document.body);
  return [
    containerStyle.getPropertyValue('reading-flow'),
    childStyle.getPropertyValue('reading-order'),
    unset.getPropertyValue('reading-flow'),
    unset.getPropertyValue('reading-order')
  ].join('|');
})()
"#,
        )
        .expect("computed reading-flow probe should evaluate");

    assert_eq!(result, "grid-order|-2|normal|0");
}

#[test]
fn computed_width_resolves_percent_against_parent_and_child_frame_viewport() {
    let mut vm = new_storage_test_vm("https://computed-width.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #outside { width: 200px; }
    #inside { width: 50%; }`;
  (document.documentElement || document.body || document).appendChild(style);
  const outside = document.createElement('div');
  outside.id = 'outside';
  const inside = document.createElement('div');
  inside.id = 'inside';
  outside.appendChild(inside);
  const appendTarget = document.body || document.documentElement || document;
  appendTarget.appendChild(outside);

  const frame = document.createElement('iframe');
  frame.setAttribute('width', '100');
  appendTarget.appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<body style="margin:0"><div style="width:100%"></div>');
  childDocument.close();

  return [
    getComputedStyle(inside).width,
    frame.contentWindow.getComputedStyle(childDocument.querySelector('div')).width
  ].join('|');
})()
"#,
        )
        .expect("computed width should resolve percent values");

    assert_eq!(result, "100px|100px");
}

#[test]
fn transformed_oversized_inline_iframe_uses_its_containing_block_percentage_basis() {
    let mut vm = new_storage_test_vm("https://inline-iframe-percentage-size.test/");

    let result = eval_with_layout_publications(&mut vm,
            r#"
(function* () {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const container = document.createElement('div');
  container.style.cssText = 'width:auto;max-width:200px;height:100px;overflow:hidden';
  const frame = document.createElement('iframe');
  frame.style.cssText = 'width:calc(100% / 0.8);height:50px;border:0;transform:scale(0.8);transform-origin:0 0';
  container.appendChild(frame);
  body.appendChild(container);
  yield; // Publish this scene before reading its geometry.
  const rect = frame.getBoundingClientRect();
  return [
    frame.offsetWidth,
    frame.clientWidth,
    rect.width
  ].join('|');
})()
"#,
        )
        .expect("transformed inline iframe geometry should evaluate");

    assert_eq!(result, "250|250|200");
}

#[test]
fn computed_horizontal_margin_reads_preserve_retained_style_viewport_context() {
    let mut vm = new_storage_test_vm("https://computed-margin-retained-context.test/");
    vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: 800,
        inner_height: 600,
        outer_width: 800,
        outer_height: 600,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,

        ..Default::default()
    }))
    .expect("viewport surface should update");
    let document = vm.document_handle_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = `
    #margin-parent { width: 400px; }
    #margin-child {
      display: block;
      width: 200px;
      margin-left: auto;
      margin-right: auto;
    }`;
  head.appendChild(style);

  const parent = document.createElement('div');
  parent.id = 'margin-parent';
  const child = document.createElement('div');
  child.id = 'margin-child';
  parent.appendChild(child);
  body.appendChild(parent);

  const computed = getComputedStyle(child);
  let values = '';
  for (let i = 0; i < 16; i += 1) {
    values = [
      computed.marginLeft,
      computed.width,
      computed.container,
      computed.containerType
    ].join('|');
  }
  return values;
})()
"#,
        )
        .expect("horizontal margin reads should preserve the full viewport context");

    assert_eq!(result, "100px|200px|none|normal");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(document),
        1,
        "nested width resolution must not replace the 800x600 viewport with a width-only key",
    );
}

#[test]
fn child_computed_horizontal_margin_reads_preserve_retained_style_viewport_context() {
    let mut vm = new_storage_test_vm("https://child-computed-margin-retained-context.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const body = document.body || document.documentElement || document;
  const frame = document.createElement('iframe');
  frame.id = 'margin-context-frame';
  frame.style.width = '400px';
  frame.style.height = '250px';
  body.appendChild(frame);

  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      #margin-parent { width: 300px; }
      #margin-child {
        display: block;
        width: 100px;
        margin-left: auto;
        margin-right: auto;
      }
    </style>
    <body>
      <div id="margin-parent">
        <div id="margin-child"></div>
      </div>
    </body>`);
  childDocument.close();

  const computed = childWindow.getComputedStyle(
    childDocument.getElementById('margin-child')
  );
  let values = '';
  for (let i = 0; i < 16; i += 1) {
    values = [
      computed.marginLeft,
      computed.width,
      computed.container,
      computed.containerType
    ].join('|');
  }
  return values;
})()
"#,
        )
        .expect("child horizontal margin reads should preserve the iframe viewport context");

    assert_eq!(result, "100px|100px|none|normal");
    let child_document = child_document_handle_for_frame_id(&vm, "margin-context-frame");
    assert_eq!(
        vm.retained_style_system_rebuild_count_for_document_for_test(child_document),
        1,
        "nested child width resolution must preserve the iframe viewport height and screen",
    );
}

#[test]
fn computed_used_size_clamps_infinite_negative_math_to_zero() {
    let mut vm = new_storage_test_vm("https://computed-used-size-infinity.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.style.display = 'block';
  body.appendChild(target);

  const first = 'calc(infinity * 1px - infinity * 1%)';
  const second = 'calc(infinity * 1px - max(infinity * 1%, 0%))';
  const fromAttribute = document.createElement('div');
  fromAttribute.setAttribute('style', `display:block;width:${first};height:${first}`);
  body.appendChild(fromAttribute);

  target.style.setProperty('width', first);
  const firstSpecified = target.style.getPropertyValue('width');
  const firstComputed = getComputedStyle(target).width;

  target.style.setProperty('width', second);
  const secondSpecified = target.style.getPropertyValue('width');
  const secondComputed = getComputedStyle(target).width;

  return [
    CSS.supports('width', first),
    firstSpecified !== '',
    firstComputed,
    CSS.supports('width', second),
    secondSpecified !== '',
    secondComputed,
    getComputedStyle(fromAttribute).width,
    getComputedStyle(fromAttribute).height
  ].join('|');
})()
"#,
        )
        .expect("computed used sizes should clamp negative infinite math");

    assert_eq!(result, "true|true|0px|true|true|0px|0px|0px");
}

#[test]
fn computed_style_child_document_media_queries_use_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://child-document-media-query.test/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const frame = document.createElement('iframe');
  frame.style.width = '100px';
  frame.style.height = '100px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write('<style>body { color: red } @media all and (min-width: 101px) { body { color: green } }</style><body>text</body>');
  childDocument.close();
  document.body.offsetTop;
  const before = getComputedStyle(childDocument.body).color;
  frame.style.width = '200px';
  yield; // Publish the new viewport before evaluating the child's media query.
  const after = getComputedStyle(childDocument.body).color;
  return `${before}|${after}`;
})()
"#,
    )
    .expect("child document media queries should use iframe viewport");

    assert_eq!(result, "rgb(255, 0, 0)|rgb(0, 128, 0)");
}

#[test]
fn computed_style_child_document_media_queries_use_iframe_viewport_height_for_calc() {
    let mut vm = new_storage_test_vm("https://child-document-media-query-calc.test/");

    let result = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const frame = document.createElement('iframe');
  frame.setAttribute('width', '100');
  frame.setAttribute('height', '10');
  (document.body || document.documentElement || document).appendChild(frame);
  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      body { background-color: rgb(0, 0, 255); }
      @media (width: calc(200vh + 5em)) {
        body { background-color: rgb(255, 165, 0); }
      }
    </style>
    <body>text</body>`);
  childDocument.close();
  return frame.contentWindow.getComputedStyle(childDocument.body).backgroundColor;
})()
"#,
        )
        .expect("child document calc media query should use iframe viewport height");

    assert_eq!(result, "rgb(255, 165, 0)");
}

#[test]
fn computed_style_child_document_viewport_units_use_css_iframe_size() {
    let mut vm = new_storage_test_vm("https://child-document-viewport-units.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const head = document.head || html.appendChild(document.createElement('head'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const style = document.createElement('style');
  style.textContent = 'iframe.viewport-probe { width: 200px; height: 100px; }';
  head.appendChild(style);

  const frame = document.createElement('iframe');
  frame.className = 'viewport-probe';
  body.appendChild(frame);

  const childDocument = frame.contentWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      html, body { margin: 0; width: 100%; height: 100%; }
      #vh { height: 100vh; }
      #vmin { width: 100vmin; height: 1px; }
      #vmax { width: 100vmax; height: 1px; }
    </style>
    <body>
      <div id="vh"></div>
      <div id="vmin"></div>
      <div id="vmax"></div>
    </body>`);
  childDocument.close();

  const childWindow = frame.contentWindow;
  return [
    childWindow.getComputedStyle(childDocument.getElementById('vh')).height,
    childWindow.getComputedStyle(childDocument.getElementById('vmin')).width,
    childWindow.getComputedStyle(childDocument.getElementById('vmax')).width
  ].join('|');
})()
"#,
        )
        .expect("child document viewport units should use css iframe size");

    assert_eq!(result, "100px|100px|200px");
}

#[test]
fn held_main_document_computed_styles_follow_repeated_viewport_surface_changes() {
    let mut vm = new_storage_test_vm("https://held-main-viewport-units.test/");
    let surface = |inner_width, inner_height| crate::protocol_types::ViewportSurface {
        inner_width,
        inner_height,
        outer_width: inner_width,
        outer_height: inner_height,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,

        ..Default::default()
    };
    vm.set_viewport_surface(Some(surface(1000, 800)))
        .expect("initial viewport surface should update");

    let before = vm
        .eval(
            r#"
(() => {
  const style = document.createElement('style');
  style.textContent = `
    #held-main-viewport-target {
      position: absolute;
      width: 10vw;
      height: 10vh;
      --pseudo-width: 5vw;
    }
    #held-main-viewport-target::before {
      content: "viewport";
      width: var(--pseudo-width);
      height: 2vmin;
    }
    @media (min-width: 1px) {
      #held-main-viewport-target { left: 50vw; }
    }`;
  (document.head || document.documentElement || document).appendChild(style);
  const target = document.createElement('div');
  target.id = 'held-main-viewport-target';
  target.style.paddingLeft = '12vw';
  (document.body || document.documentElement || document).appendChild(target);
  const held = getComputedStyle(target);
  const heldBefore = getComputedStyle(target, '::before');
  globalThis.__heldMainViewportStyles = { target, held, heldBefore };
  return [
    innerWidth,
    innerHeight,
    held.width,
    held.height,
    held.paddingLeft,
    held.left,
    heldBefore.width,
    heldBefore.height
  ].join('|');
})()
"#,
        )
        .expect("main-document viewport-unit styles should be cached");
    assert_eq!(before, "1000|800|100px|80px|120px|500px|50px|16px");
    let document = vm.document_handle_for_test();
    let updates = vm.retained_style_system_update_count_for_document_for_test(document);
    let stylist = vm.retained_stylist_identity_for_document_for_test(document);

    vm.set_viewport_surface(Some(surface(500, 400)))
        .expect("smaller viewport surface should update");
    let smaller = vm
        .eval(
            r#"
(() => {
  const { target, held, heldBefore } = __heldMainViewportStyles;
  const fresh = getComputedStyle(target);
  const freshBefore = getComputedStyle(target, '::before');
  return [
    innerWidth,
    innerHeight,
    held.width,
    held.height,
    held.paddingLeft,
    held.left,
    heldBefore.width,
    heldBefore.height,
    fresh.width,
    freshBefore.width
  ].join('|');
})()
"#,
        )
        .expect("held styles should observe the smaller viewport");
    assert_eq!(smaller, "500|400|50px|40px|60px|250px|25px|8px|50px|25px");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates + 1,
        "one viewport transition should update the retained document world once",
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist,
        "viewport changes must retain the Document Stylist",
    );

    vm.set_viewport_surface(Some(surface(1000, 800)))
        .expect("restored viewport surface should update");
    let restored = vm
        .eval(
            r#"
(() => {
  const { held, heldBefore } = __heldMainViewportStyles;
  return [
    innerWidth,
    innerHeight,
    held.width,
    held.height,
    held.paddingLeft,
    held.left,
    heldBefore.width,
    heldBefore.height
  ].join('|');
})()
"#,
        )
        .expect("held styles should observe the restored viewport");
    assert_eq!(restored, "1000|800|100px|80px|120px|500px|50px|16px");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(document),
        updates + 2,
        "each distinct viewport transition should produce one retained update",
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(document),
        stylist,
    );
}

#[test]
fn cached_child_viewport_units_update_after_the_iframe_viewport_changes() {
    let mut vm = new_storage_test_vm("https://cached-child-viewport-units.test/");

    let before = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const frame = document.createElement('iframe');
  frame.id = 'cached-viewport-unit-frame';
  frame.style.cssText = 'width:200px;height:160px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      #target {
        position: absolute;
        width: 10vw;
        height: 10vh;
        padding-left: calc(5vw + 8px);
      }
      @media (min-width: 1px) {
        #target { left: 50vw; }
      }
    </style>
    <body><div id="target"></div></body>`);
  childDocument.close();
  const target = childDocument.getElementById('target');
  const held = childWindow.getComputedStyle(target);
yield; // Publish this scene before reading its geometry.
  frame.getBoundingClientRect();
  globalThis.__cachedViewportUnitFixture = { frame, childWindow, target, held };
  return [
    childWindow.innerWidth,
    childWindow.innerHeight,
    held.width,
    held.height,
    held.paddingLeft,
    held.left
  ].join('|');
})()
"#,
    )
    .expect("the child viewport-unit style should be cached before resize");
    assert_eq!(before, "200|160|20px|16px|18px|100px");
    let child_document = child_document_handle_for_frame_id(&vm, "cached-viewport-unit-frame");
    let updates = vm.retained_style_system_update_count_for_document_for_test(child_document);

    let after = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const { frame, childWindow, target, held } = __cachedViewportUnitFixture;
  frame.style.width = '100px';
  frame.style.height = '80px';
yield; // Publish this scene before reading its geometry.
  const frameWidth = frame.getBoundingClientRect().width;
  const heldValues = [held.width, held.height, held.paddingLeft, held.left];
  const fresh = childWindow.getComputedStyle(target);
  return [
    frameWidth,
    childWindow.innerWidth,
    childWindow.innerHeight,
    ...heldValues,
    fresh.width,
    fresh.height
  ].join('|');
})()
"#,
    )
    .expect("the same cached child element should observe the resized iframe viewport");
    assert_eq!(after, "100|100|80|10px|8px|13px|50px|10px|8px");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        updates + 1,
        "one iframe viewport change should update its retained style world exactly once",
    );
}

#[test]
fn nested_iframe_cached_viewport_units_follow_recursive_frame_resizes() {
    let mut vm = new_storage_test_vm("https://nested-cached-viewport-units.test/");

    let before = vm
        .eval(
            r#"
(() => {
  const outerFrame = document.createElement('iframe');
  outerFrame.id = 'nested-viewport-outer-frame';
  outerFrame.style.cssText = 'width:200px;height:160px;border:0';
  (document.body || document.documentElement || document).appendChild(outerFrame);

  const outerWindow = outerFrame.contentWindow;
  const outerDocument = outerWindow.document;
  outerDocument.open();
  outerDocument.write(`
    <style>
      html, body { margin: 0; width: 100%; height: 100%; }
      #nested-viewport-inner-frame {
        display: block;
        width: 50%;
        height: 50%;
        border: 0;
      }
    </style>
    <body><iframe id="nested-viewport-inner-frame"></iframe></body>`);
  outerDocument.close();

  const innerFrame = outerDocument.getElementById('nested-viewport-inner-frame');
  const innerWindow = innerFrame.contentWindow;
  const innerDocument = innerWindow.document;
  innerDocument.open();
  innerDocument.write(`
    <style>
      #nested-viewport-target {
        width: 10vw;
        height: 10vh;
        padding-left: 5vmin;
      }
    </style>
    <body><div id="nested-viewport-target"></div></body>`);
  innerDocument.close();

  outerFrame.getBoundingClientRect();
  innerFrame.getBoundingClientRect();
  const target = innerDocument.getElementById('nested-viewport-target');
  const held = innerWindow.getComputedStyle(target);
  globalThis.__nestedViewportUnitFixture = {
    outerFrame,
    innerFrame,
    outerWindow,
    innerWindow,
    target,
    held
  };
  return [
    outerWindow.innerWidth,
    outerWindow.innerHeight,
    innerWindow.innerWidth,
    innerWindow.innerHeight,
    held.width,
    held.height,
    held.paddingLeft
  ].join('|');
})()
"#,
        )
        .expect("nested viewport-unit styles should be cached");
    assert_eq!(before, "200|160|100|80|10px|8px|4px");
    let inner_document = child_document_handle_for_frame_id(&vm, "nested-viewport-inner-frame");
    let updates = vm.retained_style_system_update_count_for_document_for_test(inner_document);
    let stylist = vm.retained_stylist_identity_for_document_for_test(inner_document);

    let query = r#"
(() => {
  const {
    outerFrame,
    innerFrame,
    outerWindow,
    innerWindow,
    target,
    held
  } = __nestedViewportUnitFixture;
  outerFrame.style.width = '100px';
  outerFrame.style.height = '80px';
  outerFrame.getBoundingClientRect();
  innerFrame.getBoundingClientRect();
  const heldValues = [held.width, held.height, held.paddingLeft];
  const fresh = innerWindow.getComputedStyle(target);
  return [
    outerWindow.innerWidth,
    outerWindow.innerHeight,
    innerWindow.innerWidth,
    innerWindow.innerHeight,
    ...heldValues,
    fresh.width,
    fresh.height
  ].join('|');
})()
"#;
    let after = vm
        .eval(query)
        .expect("warm queries retain the published frame viewports");
    assert_eq!(after, "200|160|100|80|10px|8px|4px|10px|8px");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(inner_document),
        updates
    );
    publish_layout_for_test(&mut vm);
    let refreshed = vm
        .eval(query)
        .expect("explicit refresh updates recursive frame viewports");
    assert_eq!(refreshed, "100|80|50|40|5px|4px|2px|5px|4px");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(inner_document),
        updates + 1,
        "the innermost Document should consume one recursive viewport transition",
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(inner_document),
        stylist,
        "recursive iframe resizing must retain the innermost Stylist",
    );
}

#[test]
fn child_document_mixed_style_observations_do_not_churn_the_retained_world() {
    let mut vm = new_storage_test_vm("https://child-style-observation-context.test/");
    let warmup = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'mixed-observation-frame';
  frame.style.cssText = 'width: 240px; height: 160px';
  (document.body || document.documentElement || document).appendChild(frame);

  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>#mixed-observation-target { color: rgb(0, 128, 0); }</style>
    <style media="(max-width: 300px)">
      .narrow-child-rule { display: block; }
    </style>
    <style media="(min-width: 700px)">
      .wide-child-rule { display: block; }
    </style>
    <body><div id="mixed-observation-target"
      style="font-size: 16px; transition-duration: calc(1s); text-size-adjust: calc(100%)"
    >child text</div></body>`);
  childDocument.close();

  const shadowHost = childDocument.createElement('section');
  childDocument.body.appendChild(shadowHost);
  shadowHost.attachShadow({ mode: 'open' }).innerHTML = `
    <style media="(max-width: 300px)">:host { display: block; }</style>
    <span>shadow text</span>`;

  const target = childDocument.getElementById('mixed-observation-target');
  globalThis.__mixedStyleObservation = { childWindow, target };
  return [
    target.checkVisibility(),
    target.innerText,
    childWindow.getComputedStyle(target).color
  ].join('|');
})()
"#,
        )
        .expect("mixed child-document style observation fixture should initialize");
    assert_eq!(warmup, "true|child text|rgb(0, 128, 0)");

    let child_document = child_document_handle_for_frame_id(&vm, "mixed-observation-frame");
    let child_updates = vm.retained_style_system_update_count_for_document_for_test(child_document);
    let child_stylist = vm.retained_stylist_identity_for_document_for_test(child_document);
    #[cfg(debug_assertions)]
    let invariant_checks = vm
        ._context_host
        .borrow()
        .completed_style_observation_stability_check_count_for_document_for_test(child_document);
    let materializations = vm
        ._context_host
        .borrow()
        .style_world_update_materializations_for_test();
    let dom_version = vm._context_host.borrow().dom_host().dom_version();
    let (_, viewport_cache_misses, viewport_cache_entries) = vm
        ._context_host
        .borrow()
        .inferred_frame_style_viewport_cache_observability();

    let visibility_result = vm
        .eval(
            r#"
(() => {
  const { target } = __mixedStyleObservation;
  let visible = 0;
  for (let index = 0; index < 12; index += 1) {
    visible += target.checkVisibility() ? 1 : 0;
  }
  return String(visible);
})()
"#,
        )
        .expect("repeated child checkVisibility observations should remain stable");
    assert_eq!(visibility_result, "12");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        child_updates,
        "child checkVisibility must use the child style environment",
    );

    let inner_text_result = vm
        .eval(
            r#"
(() => {
  const { target } = __mixedStyleObservation;
  let length = 0;
  for (let index = 0; index < 12; index += 1) {
    length += target.innerText.length;
  }
  return String(length);
})()
"#,
        )
        .expect("repeated child innerText observations should remain stable");
    assert_eq!(inner_text_result, "120");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        child_updates,
        "child innerText must use the child style environment",
    );

    let computed_result = vm
        .eval(
            r#"
(() => {
  const { childWindow, target } = __mixedStyleObservation;
  let length = 0;
  for (let index = 0; index < 12; index += 1) {
    length += childWindow.getComputedStyle(target).color.length;
  }
  return String(length);
})()
"#,
        )
        .expect("repeated child computed-style observations should remain stable");
    assert_eq!(computed_result, "168");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        child_updates,
        "child getComputedStyle must retain the child style environment",
    );

    let mixed_result = vm
        .eval(
            r#"
(() => {
  const { childWindow, target } = __mixedStyleObservation;
  let checksum = 0;
  for (let index = 0; index < 12; index += 1) {
    checksum += target.checkVisibility() ? 1 : 0;
    checksum += target.innerText.length;
    checksum += childWindow.getComputedStyle(target).color.length;
  }
  return String(checksum);
})()
"#,
        )
        .expect("mixed child-document style observations should remain stable");
    assert_eq!(mixed_result, "300");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        child_updates,
        "alternating child DOM APIs must not change the retained child style environment",
    );
    assert_eq!(
        vm.retained_stylist_identity_for_document_for_test(child_document),
        child_stylist,
        "alternating child DOM APIs must retain the same child Stylist",
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .style_world_update_materializations_for_test(),
        materializations,
        "alternating child DOM APIs must not materialize clean style-world updates",
    );
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version,
        "installing an existing child document wrapper must not report a DOM mutation",
    );
    let (_, final_viewport_cache_misses, final_viewport_cache_entries) = vm
        ._context_host
        .borrow()
        .inferred_frame_style_viewport_cache_observability();
    assert_eq!(
        final_viewport_cache_misses, viewport_cache_misses,
        "clean child style observations must reuse the inferred iframe viewport",
    );
    assert_eq!(final_viewport_cache_entries, viewport_cache_entries);
    #[cfg(debug_assertions)]
    assert!(
        vm._context_host
            .borrow()
            .completed_style_observation_stability_check_count_for_document_for_test(
                child_document,
            )
            > invariant_checks,
        "independent child DOM API operations must be compared by the persistent style-world invariant",
    );
}

#[test]
fn layout_publishes_authoritative_iframe_viewport_without_style_world_ping_pong() {
    let mut vm = new_storage_test_vm("https://authoritative-child-style-viewport.test/");
    let result = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const frame = document.createElement('iframe');
  frame.id = 'authoritative-style-viewport-frame';
  frame.style.cssText = [
    'box-sizing:border-box',
    'width:200px',
    'height:160px',
    'padding:20px',
    'border:10px solid black'
  ].join(';');
  (document.body || document.documentElement || document).appendChild(frame);

  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write(`
    <style>
      body { color: rgb(0, 0, 0); }
      @media (width: 200px) { body { color: rgb(200, 0, 0); } }
      @media (width: 140px) { body { color: rgb(0, 140, 0); } }
    </style>
    <body>authoritative viewport</body>`);
  childDocument.close();

  const beforeLayout = childWindow.getComputedStyle(childDocument.body).color;
yield; // Publish this scene before reading its geometry.
  const frameWidth = frame.getBoundingClientRect().width;
  const afterLayout = childWindow.getComputedStyle(childDocument.body).color;
  globalThis.__authoritativeStyleViewport = { frame, childWindow, childDocument };
  return [beforeLayout, frameWidth, childWindow.innerWidth, afterLayout].join('|');
})()
"#,
    )
    .expect("layout should publish an exact iframe content viewport");

    assert_eq!(result, "rgb(200, 0, 0)|200|140|rgb(0, 140, 0)");
    let child_document =
        child_document_handle_for_frame_id(&vm, "authoritative-style-viewport-frame");
    let updates = vm.retained_style_system_update_count_for_document_for_test(child_document);

    let stable = vm
        .eval(
            r#"
(() => {
  const { frame, childWindow, childDocument } = __authoritativeStyleViewport;
  let stable = true;
  for (let index = 0; index < 8; index += 1) {
    stable &&= childWindow.getComputedStyle(childDocument.body).color === 'rgb(0, 140, 0)';
    stable &&= childDocument.body.innerText === 'authoritative viewport';
    stable &&= frame.getBoundingClientRect().width === 200;
    stable &&= childWindow.innerWidth === 140;
  }
  return String(stable);
})()
"#,
        )
        .expect("published iframe viewport should remain stable across DOM APIs");
    assert_eq!(stable, "true");
    assert_eq!(
        vm.retained_style_system_update_count_for_document_for_test(child_document),
        updates,
        "automatic and layout observations must converge on the published iframe content viewport",
    );
}

#[test]
fn nested_child_mixed_style_observations_keep_each_document_viewport() {
    let mut vm = new_storage_test_vm("https://nested-style-observation-context.test/");
    let warmup = vm
        .eval(
            r#"
(() => {
  const outerFrame = document.createElement('iframe');
  outerFrame.id = 'mixed-observation-outer-frame';
  outerFrame.style.cssText = 'width: 500px; height: 300px';
  (document.body || document.documentElement || document).appendChild(outerFrame);

  const outerWindow = outerFrame.contentWindow;
  const outerDocument = outerWindow.document;
  outerDocument.open();
  outerDocument.write(`
    <style media="(width: 500px)">body { color: rgb(1, 2, 3); }</style>
    <body><iframe id="mixed-observation-inner-frame"
      style="width: 180px; height: 90px"></iframe></body>`);
  outerDocument.close();

  const innerFrame = outerDocument.getElementById('mixed-observation-inner-frame');
  const innerWindow = innerFrame.contentWindow;
  const innerDocument = innerWindow.document;
  innerDocument.open();
  innerDocument.write(`
    <style>#mixed-observation-nested-target { color: rgb(0, 128, 0); }</style>
    <style media="(max-width: 200px)">
      .narrow-nested-rule { display: block; }
    </style>
    <style media="(min-width: 700px)">
      .wide-nested-rule { display: block; }
    </style>
    <body><div id="mixed-observation-nested-target">nested text</div></body>`);
  innerDocument.close();

  const target = innerDocument.getElementById('mixed-observation-nested-target');
  target.checkVisibility();
  outerWindow.getComputedStyle(innerFrame).display;
  const color = innerWindow.getComputedStyle(target).color;
  globalThis.__nestedMixedStyleObservation = { innerWindow, target };
  return color;
})()
"#,
        )
        .expect("nested mixed style observation fixture should initialize");
    assert_eq!(warmup, "rgb(0, 128, 0)");

    let documents = [
        vm.document_handle_for_test(),
        child_document_handle_for_frame_id(&vm, "mixed-observation-outer-frame"),
        child_document_handle_for_frame_id(&vm, "mixed-observation-inner-frame"),
    ];
    let updates = documents
        .map(|document| vm.retained_style_system_update_count_for_document_for_test(document));
    let stylists =
        documents.map(|document| vm.retained_stylist_identity_for_document_for_test(document));
    let dom_version = vm._context_host.borrow().dom_host().dom_version();
    let (_, viewport_cache_misses, viewport_cache_entries) = vm
        ._context_host
        .borrow()
        .inferred_frame_style_viewport_cache_observability();

    let result = vm
        .eval(
            r#"
(() => {
  const { innerWindow, target } = __nestedMixedStyleObservation;
  let checksum = 0;
  for (let index = 0; index < 12; index += 1) {
    checksum += target.checkVisibility() ? 1 : 0;
    checksum += target.innerText.length;
    checksum += innerWindow.getComputedStyle(target).color.length;
  }
  return String(checksum);
})()
"#,
        )
        .expect("nested mixed style observations should remain stable");
    assert_eq!(result, "312");

    for (index, document) in documents.into_iter().enumerate() {
        assert_eq!(
            vm.retained_style_system_update_count_for_document_for_test(document),
            updates[index],
            "mixed nested DOM APIs must not update retained style document {index}",
        );
        assert_eq!(
            vm.retained_stylist_identity_for_document_for_test(document),
            stylists[index],
            "mixed nested DOM APIs must retain Stylist identity for document {index}",
        );
    }
    assert_eq!(
        vm._context_host.borrow().dom_host().dom_version(),
        dom_version,
        "nested child document wrappers must remain connected without repeated mutations",
    );
    let (_, final_viewport_cache_misses, final_viewport_cache_entries) = vm
        ._context_host
        .borrow()
        .inferred_frame_style_viewport_cache_observability();
    assert_eq!(
        final_viewport_cache_misses, viewport_cache_misses,
        "clean nested style observations must reuse every inferred iframe viewport",
    );
    assert_eq!(final_viewport_cache_entries, viewport_cache_entries);
}

#[test]
fn child_window_and_mock_root_geometry_use_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://child-window-viewport.test/");

    let before = eval_with_layout_publications(
        &mut vm,
        r#"
(function* () {
  const frame = document.createElement('iframe');
  frame.id = 'viewport-frame';
  frame.style.width = '300px';
  frame.style.height = '65px';
  (document.body || document.documentElement || document).appendChild(frame);
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.open();
  childDocument.write('<style>html,body{margin:0;width:100%;height:100%}</style><body></body>');
  childDocument.close();

  const snapshot = () => {
    const body = childDocument.body.getBoundingClientRect();
    const root = childDocument.documentElement.getBoundingClientRect();
    return [
      childWindow.innerWidth,
      childWindow.innerHeight,
      childWindow.matchMedia('(width: 300px)').matches,
      body.width,
      body.height,
      root.width,
      root.height
    ].join('|');
  };
yield; // Publish this scene before reading its geometry.
  return snapshot();
})()
"#,
    )
    .expect("initial child Window viewport and root geometry should evaluate");
    assert_eq!(before, "300|65|true|300|65|300|65");

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById('viewport-frame');
  frame.style.width = '320px';
  frame.style.height = '80px';
  return 'resized';
})()
"#,
    )
    .expect("child frame resize should evaluate");
    publish_layout_for_test(&mut vm);
    let after = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById('viewport-frame');
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  return [
    childWindow.innerWidth,
    childWindow.innerHeight,
    childWindow.matchMedia('(width: 320px)').matches,
    childDocument.body.getBoundingClientRect().width,
    childDocument.body.getBoundingClientRect().height
  ].join('|');
})()
"#,
        )
        .expect("resized child Window viewport and root geometry should evaluate");

    assert_eq!(after, "320|80|true|320|80");
}

#[test]
fn direct_child_realm_window_surface_uses_iframe_viewport() {
    let mut vm = new_storage_test_vm("https://direct-child-window-viewport.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.style.width = '300px';
  frame.style.height = '65px';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("direct child Window viewport setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "direct child Window viewport realm",
    );

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => [
  innerWidth,
  innerHeight,
  matchMedia('(width: 300px)').matches,
  matchMedia('(prefers-reduced-motion: no-preference)').matches
].join('|'))()
"#,
        )
        .expect("direct child Window viewport should evaluate in its own realm");

    assert_eq!(result, "300|65|true|true");

    let top_result = vm
        .eval("[innerWidth, innerHeight].join('|')")
        .expect("top Window viewport should remain top-level after direct child evaluation");
    assert_eq!(top_result, "1920|1080");
}

#[test]
fn child_parser_quirks_mode_does_not_mutate_the_top_document() {
    let mut vm = new_storage_test_vm("https://child-quirks-owner.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<body>quirks child</body>';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child quirks-mode owner setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    let result = vm
        .eval(
            "[document.compatMode, document.querySelector('iframe').contentDocument.compatMode].join('|')",
        )
        .expect("top and child document compatMode should evaluate");

    assert_eq!(result, "CSS1Compat|BackCompat");
}
