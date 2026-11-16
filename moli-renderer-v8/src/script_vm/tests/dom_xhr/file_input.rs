use super::*;

mod state_styles;

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
  const other = new File(['other'], 'other.txt');
  const indexDescriptor = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  let arraySetterCalls = 0;
  Object.defineProperty(Array.prototype, '0', {configurable: true, set() {
    ++arraySetterCalls;
    throw new Error('internal FileList storage must not invoke Array.prototype');
  }});
  try {
    transfer.items.add(other);
    input.files = files;
  } finally {
    if (indexDescriptor) Object.defineProperty(Array.prototype, '0', indexDescriptor);
    else delete Array.prototype[0];
  }
  assert(arraySetterCalls === 0 && files.length === 2 && files[0] === file && files[1] === other,
    'syncing native FileList contents must bypass Array.prototype setters');
  const nameGetter = Object.getOwnPropertyDescriptor(File.prototype, 'name').get;
  const lastModifiedGetter = Object.getOwnPropertyDescriptor(File.prototype, 'lastModified').get;
  let reads = 0;
  const trap = {get() { ++reads; throw new Error('public property must not be read'); }};
  for (const name of ['name', 'lastModified']) Object.defineProperty(file, name, trap);
  for (const name of ['0', 'length', Symbol.iterator]) Object.defineProperty(files, name, trap);
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

#[test]
fn directory_selected_files_keep_relative_paths_in_wrappers_clones_and_file_lists() {
    let mut vm = new_parsed_test_vm(
        "https://directory-upload.test/",
        "<!doctype html><input id=upload type=file webkitdirectory>",
    );
    let handle = vm.document_runtime.get_element_by_id("upload").unwrap();
    let files = ["folder/a.txt", "folder/sub/b.txt"]
        .into_iter()
        .map(|relative_path| crate::dom::native::SelectedFile {
            data: relative_path.as_bytes().to_vec().into(),
            mime_type: "text/plain".into(),
            name: relative_path.rsplit('/').next().unwrap().into(),
            relative_path: relative_path.into(),
            last_modified: 7.0,
        })
        .collect();
    assert!(vm.set_file_input_files(handle, files, false).unwrap());
    assert_eq!(vm.eval(r#"(() => {
        const assert = (condition, message) => { if (!condition) throw new Error(message); };
        const input = document.getElementById('upload');
        assert(input.webkitdirectory && !input.multiple && input.files.length === 2, 'directory permits multiple files');
        const expected = ['folder/a.txt', 'folder/sub/b.txt'];
        const files = input.files;
        assert(Array.from(files, f => f.webkitRelativePath).join('|') === expected.join('|'), 'relative paths');
        const copy = structuredClone(files[1]);
        assert(copy.name === 'b.txt' && copy.webkitRelativePath === expected[1], 'structured clone metadata');
        const transfer = new DataTransfer(); transfer.items.add(files[0]);
        assert(transfer.files[0].webkitRelativePath === expected[0], 'DataTransfer metadata');
        input.files = transfer.files;
        assert(input.files[0].webkitRelativePath === expected[0], 'FileList round trip');
        assert(new File(['x'], 'ordinary.txt').webkitRelativePath === '', 'ordinary File default');
        input.webkitdirectory = false; assert(!input.hasAttribute('webkitdirectory'), 'IDL setter removes attribute');
        input.setAttribute('webkitdirectory', 'false'); assert(input.webkitdirectory, 'boolean attribute presence');
        globalThis.directoryMultipart = new FormData();
        directoryMultipart.append('upload', input.files[0]);
        Object.defineProperty(input.files[0], 'webkitRelativePath', {get() { throw new Error('public path must not be read for upload'); }});
        return 'ok';
    })()"#).unwrap(), "ok");
    let context_ptr: *const v8::Global<v8::Context> =
        &vm.current_main_world().unwrap().context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);
            let key = v8::String::new(scope, "directoryMultipart").unwrap();
            let form = global.get(scope, key.into()).unwrap();
            let form = v8::Local::<v8::Object>::try_from(form).unwrap();
            let (body, _) = crate::context_bootstrap::form_data_request_body(scope, form)
                .unwrap()
                .unwrap();
            let body = String::from_utf8(body).unwrap();
            assert!(body.contains("filename=\"folder/a.txt\""), "{body}");
            assert!(body.contains("\r\n\r\nfolder/a.txt\r\n"), "{body}");
            Ok(())
        })
        .unwrap();
}

#[test]
fn equal_file_values_keep_the_supplied_filelist_identity() {
    let mut vm = new_parsed_test_vm(
        "https://file-list.test/",
        "<!doctype html><input id=upload type=file>",
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
        const input = document.getElementById('upload');
        const first = new DataTransfer();
        first.items.add(new File(['same'], 'file.txt', {lastModified: 1}));
        input.files = first.files;
        const second = new DataTransfer();
        second.items.add(new File(['same'], 'file.txt', {lastModified: 1}));
        const list = second.files;
        input.files = list;
        return input.files === list && input.files[0] === list[0];
    })()"#
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn file_list_item_uses_native_contents_and_bounds_without_author_getters() {
    let mut vm = new_parsed_test_vm(
        "https://file-list-native-item.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );
    assert_eq!(vm.eval(r#"
(() => {
  const frame = document.body.appendChild(document.createElement('iframe'));
  const high = '4294967294';
  for (const w of [window, frame.contentWindow]) {
    const transfer = new w.DataTransfer();
    const file = new w.File(['native'], 'native.txt');
    transfer.items.add(file);
    const list = transfer.files, item = w.FileList.prototype.item;
    let reads = 0;
    const trap = () => { reads++; throw Error('author numeric getter'); };
    Object.defineProperty(list, '0', {configurable: true, get: trap});
    Object.defineProperty(list, 'length', {configurable: true, get: trap});
    const prototype = Object.create(w.FileList.prototype);
    Object.defineProperty(prototype, '1', {configurable: true, get: trap});
    Object.setPrototypeOf(list, prototype);
    const previous = Object.getOwnPropertyDescriptor(w.Array.prototype, high);
    Object.defineProperty(w.Array.prototype, high, {configurable: true, get: trap});
    try {
      if (item.call(list, 0) !== file) throw Error('native File identity');
      if (item.call(list, 1) !== null || item.call(list, Number(high)) !== null) throw Error('native item bounds');
      const input = w.document.createElement('input'); input.type = 'file';
      const empty = input.files, emptyPrototype = Object.create(w.FileList.prototype);
      Object.defineProperty(emptyPrototype, '0', {get: trap});
      Object.setPrototypeOf(empty, emptyPrototype);
      if (item.call(empty, 0) !== null) throw Error('empty native list bounds');
      Object.setPrototypeOf(list, null);
      if (item.call(list, 0) !== file) throw Error('native identity after prototype mutation');
      if (reads !== 0) throw Error('item consulted author data');
    } finally {
      if (previous) Object.defineProperty(w.Array.prototype, high, previous);
      else delete w.Array.prototype[high];
    }
  }
  frame.remove();
  return 'ok';
})()
"#).expect("FileList.item native data probe"), "ok");
}
