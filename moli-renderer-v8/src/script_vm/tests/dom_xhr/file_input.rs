use super::*;

#[test]
fn input_files_accepts_genuine_cross_realm_lists_without_copying() {
    let mut vm = new_parsed_test_vm(
        "https://input-files-realms.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const child = frame.contentWindow;
  for (const [owner, source] of [[window, child], [child, window]]) {
    const form = owner.document.createElement('form');
    const input = owner.document.createElement('input');
    input.type = 'file';
    input.name = 'upload';
    form.appendChild(input);
    owner.document.body.appendChild(form);
    const foreignGetter = Object.getOwnPropertyDescriptor(source.HTMLInputElement.prototype, 'files').get;
    assert(Object.getPrototypeOf(foreignGetter.call(input)) === owner.FileList.prototype,
      'new FileList belongs to the input realm even through a borrowed getter');
    const transfer = new source.DataTransfer();
    const file = new source.File(['abc'], 'foreign.txt', {type: 'text/plain', lastModified: 7});
    transfer.items.add(file);
    const files = transfer.files;
    assert(Object.getPrototypeOf(files) === source.FileList.prototype, 'source FileList realm');
    assert(Object.getPrototypeOf(files) !== owner.FileList.prototype, 'distinct FileList realms');
    input.files = files;
    assert(input.files === files, 'assigned FileList must retain identity');
    assert(input.files[0] === file, 'assigned File must retain identity');
    const submitted = new owner.FormData(form).get('upload');
    assert(submitted.name === 'foreign.txt' && submitted.size === 3 &&
      submitted.type === 'text/plain' && submitted.lastModified === 7, 'native selected file payload');
  }
  return 'ok';
})()
"#,
        )
        .expect("file inputs must accept genuine FileLists from other realms");
    assert_eq!(result, "ok");
}

#[test]
fn input_files_uses_internal_file_list_and_file_payloads() {
    let mut vm = new_parsed_test_vm(
        "https://input-files-payload.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const form = document.createElement('form');
  const input = document.createElement('input');
  input.type = 'file';
  input.name = 'upload';
  form.appendChild(input);
  document.body.appendChild(form);
  const transfer = new DataTransfer();
  const file = new File(['abc'], 'original.txt', {type: 'text/plain', lastModified: 7});
  transfer.items.add(file);
  const files = transfer.files;
  const nameGetter = Object.getOwnPropertyDescriptor(File.prototype, 'name').get;
  const lastModifiedGetter = Object.getOwnPropertyDescriptor(File.prototype, 'lastModified').get;
  let reads = 0;
  const trap = {get() { ++reads; throw new Error('public property must not be read'); }};
  for (const name of ['name', 'lastModified']) Object.defineProperty(file, name, trap);
  for (const name of ['length', Symbol.iterator]) Object.defineProperty(files, name, trap);
  for (const prototype of [FileList.prototype, null]) {
    Object.setPrototypeOf(files, prototype);
    input.files = files;
    assert(input.files === files, 'preserve FileList identity despite public property changes');
    Object.defineProperty(input, 'files', {...trap, configurable: true});
    const submitted = new FormData(form).get('upload');
    delete input.files;
    assert(submitted === file, 'FormData must retain the selected File identity');
    assert(nameGetter.call(submitted) === 'original.txt' && submitted.size === 3 &&
      submitted.type === 'text/plain' && lastModifiedGetter.call(submitted) === 7, 'use original native File payload');
    assert(input.value.endsWith('original.txt'), 'native input selection retains the filename');
    assert(reads === 0, 'assignment and form serialization must not invoke public getters');
  }
  return 'ok';
})()
"#,
        )
        .expect("file input assignment must use internal payloads, not script-visible properties");
    assert_eq!(result, "ok");
}

#[test]
fn input_files_validates_interface_values_before_input_type_applicability() {
    let mut vm = new_storage_test_vm("https://input-files-values.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const transfer = new DataTransfer();
  transfer.items.add(new File(['abc'], 'original.txt'));
  const files = transfer.files;
  let reads = 0;
  const fake = Object.create(FileList.prototype, {
    length: {get() { ++reads; return 1; }},
    0: {value: new File(['fake'], 'fake.txt')}
  });
  const proxy = new Proxy(files, {
    getPrototypeOf() { ++reads; throw new Error('prototype trap'); },
    get() { ++reads; throw new Error('get trap'); },
  });
  const values = [false, 1, 'files', Symbol(), [], {}, fake, proxy, Object.create(files)];
  for (const type of ['file', 'text', 'hidden']) {
    const input = document.createElement('input');
    input.type = type;
    input.files = files;
    const before = input.files;
    for (const value of values) {
      let error;
      try { input.files = value; } catch (caught) { error = caught.name; }
      assert(error === 'TypeError', 'reject non-FileList on input type=' + type);
      assert(input.files === before, 'rejected assignment must not change the selection');
    }
    input.files = null;
    input.files = undefined;
    assert(input.files === before, 'nullable values must leave the selection unchanged');
  }
  assert(reads === 0, 'interface conversion must not inspect prototype or public properties');
  return 'ok';
})()
"#,
        )
        .expect("FileList interface conversion must precede the HTML setter steps");
    assert_eq!(result, "ok");
}

#[test]
fn input_files_accessors_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://input-files-receivers.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'files');
  const receivers = [null, undefined, {}, HTMLInputElement.prototype,
    document.createElement('div'), document.createElement('textarea'),
    document.createElementNS('http://www.w3.org/2000/svg', 'input'), document.createTextNode('text')];
  for (const receiver of receivers) {
    for (const operation of [() => descriptor.get.call(receiver), () => descriptor.set.call(receiver, null)]) {
      let error;
      try { operation(); } catch (caught) { error = caught.name; }
      if (error !== 'TypeError') throw new Error('files accessors must validate their HTMLInputElement receiver');
    }
  }
  return 'ok';
})()
"#,
        )
        .expect("files getters and setters must validate their HTMLInputElement receivers");
    assert_eq!(result, "ok");
}
