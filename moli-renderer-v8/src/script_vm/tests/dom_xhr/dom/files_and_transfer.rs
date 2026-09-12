use super::*;

#[test]
fn file_constructor_preserves_declared_metadata_slots() {
    let mut vm = new_storage_test_vm("https://file-metadata-declaration.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptorReport = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof descriptor?.set,
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(":");
  };
  const defaultFile = new File(["a"], "default.txt");
  const explicitFile = new File([new Uint8Array([104, 105])], "note.txt", {
    type: "text/plain",
    lastModified: 7
  });
  const weirdFile = new File(["x"], "weird.txt", { lastModified: Infinity });
  const dt = new DataTransfer();
  const item = dt.items.add(explicitFile);
  const roundTrip = item && item.getAsFile && item.getAsFile();
  const nameDescriptor = Object.getOwnPropertyDescriptor(File.prototype, "name");
  const lastModifiedDescriptor = Object.getOwnPropertyDescriptor(File.prototype, "lastModified");
  const lengthDescriptor = Object.getOwnPropertyDescriptor(FileList.prototype, "length");
  const fileInternalNames = Object.getOwnPropertyNames(explicitFile)
    .filter(name => name.startsWith("__lmFile"))
    .sort();
  const fileListInternalNames = Object.getOwnPropertyNames(dt.files)
    .filter(name => name.startsWith("__lmFile"))
    .sort();
  const fakeFile = {
    __lmFileName: "spoofed.txt",
    __lmFileLastModified: 99
  };
  const fakeFileList = { __lmFileListLength: 99 };
  return JSON.stringify({
    fileDescriptors: [
      descriptorReport(File.prototype, "name"),
      descriptorReport(File.prototype, "lastModified")
    ],
    fileListDescriptors: [
      descriptorReport(FileList.prototype, "length")
    ],
    fileInternalNames,
    fileListInternalNames,
    defaultName: defaultFile.name,
    defaultLastModifiedFinite: Number.isFinite(defaultFile.lastModified),
    explicitName: explicitFile.name,
    explicitLastModified: explicitFile.lastModified,
    explicitType: explicitFile.type,
    roundTripName: roundTrip && roundTrip.name,
    roundTripLastModified: roundTrip && roundTrip.lastModified,
    roundTripType: roundTrip && roundTrip.type,
    tag: Object.prototype.toString.call(explicitFile),
    ctor: explicitFile instanceof File,
    weirdLastModifiedFinite: Number.isFinite(weirdFile.lastModified),
    fakeName: nameDescriptor.get.call(fakeFile),
    fakeLastModifiedFinite: Number.isFinite(lastModifiedDescriptor.get.call(fakeFile)),
    fakeLength: lengthDescriptor.get.call(fakeFileList)
  });
})()
"#,
        )
        .expect("File metadata declaration should preserve script-visible slots");

    assert_eq!(
        result,
        r#"{"fileDescriptors":["name:function:get name:0:undefined:true:true","lastModified:function:get lastModified:0:undefined:true:true"],"fileListDescriptors":["length:function:get length:0:undefined:true:true"],"fileInternalNames":[],"fileListInternalNames":[],"defaultName":"default.txt","defaultLastModifiedFinite":true,"explicitName":"note.txt","explicitLastModified":7,"explicitType":"text/plain","roundTripName":"note.txt","roundTripLastModified":7,"roundTripType":"text/plain","tag":"[object File]","ctor":true,"weirdLastModifiedFinite":true,"fakeName":"","fakeLastModifiedFinite":true,"fakeLength":0}"#
    );
}

#[test]
fn data_transfer_and_input_files_support_playwright_style_upload_assignment() {
    let mut vm = new_storage_test_vm("https://input-files-upload.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const input = document.createElement('input');
  input.type = 'file';
  host.appendChild(input);

  const seen = [];
  input.addEventListener('input', () => {
    seen.push(`input:${input.files.length}:${input.files[0].name}:${input.value}`);
  });
  input.addEventListener('change', () => {
    seen.push(`change:${input.files[0].type}:${input.files[0].lastModified}`);
  });

  const dt = new DataTransfer();
  const file = new File([new Uint8Array([104, 105])], 'note.txt', {
    type: 'text/plain',
    lastModified: 7
  });
  dt.items.add(file);
  input.files = dt.files;
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));

  return [
    typeof DataTransfer,
    dt.items.length,
    Object.prototype.toString.call(dt.files),
    input.files.length,
    input.files[0].name,
    input.files[0].type,
    input.files[0].lastModified,
    input.value,
    typeof input.files[Symbol.iterator],
    Array.from(input.files).map(file => file.name).join(','),
    seen.join(',')
  ].join('|');
})()
"#,
        )
        .expect("DataTransfer-backed file assignment should succeed");

    assert_eq!(
        result,
        "function|1|[object FileList]|1|note.txt|text/plain|7|C:\\fakepath\\note.txt|function|note.txt|input:1:note.txt:C:\\fakepath\\note.txt,change:text/plain:7"
    );
}

#[test]
fn input_files_nullable_setter_preserves_current_file_list() {
    let mut vm = new_storage_test_vm("https://input-files-nullable-setter.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const input = document.createElement('input');
  input.type = 'file';
  const transfer = new DataTransfer();
  transfer.items.add(new File(['data'], 'note.txt'));
  input.files = transfer.files;
  const files = input.files;

  input.files = null;
  const afterNull = input.files === files && input.files[0].name;
  input.files = undefined;
  const afterUndefined = input.files === files && input.files[0].name;

  const assignmentError = value => {
    try {
      input.files = value;
      return 'none';
    } catch (error) {
      return error.name;
    }
  };
  return [
    afterNull,
    afterUndefined,
    assignmentError([]),
    assignmentError([new File([], 'other.txt')])
  ].join('|');
})()
"#,
        )
        .expect("nullable input files setter probe should evaluate");

    assert_eq!(result, "note.txt|note.txt|TypeError|TypeError");
}

#[test]
fn input_file_list_cache_refreshes_after_external_file_selection_replacement() {
    let mut vm = new_storage_test_vm("https://input-files-external-replace.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const host = document.body || document.documentElement || document;
  const input = document.createElement('input');
  input.id = 'upload';
  input.type = 'file';
  host.appendChild(input);
  globalThis.__emptyFiles = input.files;
  return [input.files.length, input.files === globalThis.__emptyFiles].join('|');
})()
"#,
        )
        .expect("initial file input cache probe should run");
    assert_eq!(initial, "0|true");

    let upload = vm
        .document_runtime
        .get_element_by_id("upload")
        .expect("upload input should exist");
    assert!(
        vm.set_file_input_files(
            upload,
            vec![crate::dom::native::SelectedFile {
                bytes: b"alpha".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "first.txt".to_owned(),
                last_modified: 1.0,
            }],
            false,
        )
        .expect("first external file selection should run")
    );
    let first = vm
        .eval(
            r#"
(() => {
  const input = document.getElementById('upload');
  globalThis.__firstFiles = input.files;
  return [
    input.files.length,
    input.files[0].name,
    input.files === globalThis.__emptyFiles,
    input.files === globalThis.__firstFiles
  ].join('|');
})()
"#,
        )
        .expect("first file input cache probe should run");
    assert_eq!(first, "1|first.txt|false|true");

    assert!(
        vm.set_file_input_files(
            upload,
            vec![crate::dom::native::SelectedFile {
                bytes: b"bravo".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "second.txt".to_owned(),
                last_modified: 2.0,
            }],
            false,
        )
        .expect("second external file selection should run")
    );
    let second = vm
        .eval(
            r#"
(() => {
  const input = document.getElementById('upload');
  const files = input.files;
  return [
    globalThis.__firstFiles[0].name,
    files.length,
    files[0].name,
    files === globalThis.__firstFiles,
    input.files === files
  ].join('|');
})()
"#,
        )
        .expect("second file input cache probe should run");
    assert_eq!(second, "first.txt|1|second.txt|false|true");
}

#[test]
fn data_transfer_item_list_add_returns_item_with_get_as_file() {
    let mut vm = new_storage_test_vm("https://data-transfer-item-add.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const dt = new DataTransfer();
  const file = new File(['hi'], 'note.txt', { type: 'text/plain', lastModified: 7 });
  const item = dt.items.add(file);
  const roundTrip = item && item.getAsFile && item.getAsFile();
  return [
    item !== null,
    item && item.kind,
    item && item.type,
    roundTrip && roundTrip.name,
    roundTrip && roundTrip.type,
    roundTrip && roundTrip.lastModified
  ].join('|');
})()
"#,
        )
        .expect("DataTransferItemList.add return value should evaluate");

    assert_eq!(result, "true|file|text/plain|note.txt|text/plain|7");
}

#[test]
fn constructed_data_transfer_defaults_to_none_effects() {
    let mut vm = new_storage_test_vm("https://data-transfer-constructor-defaults.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  return [transfer.dropEffect, transfer.effectAllowed].join('|');
})()
"#,
        )
        .expect("constructed DataTransfer defaults should evaluate");

    assert_eq!(result, "none|none");
}

#[test]
fn data_transfer_item_removals_disable_existing_wrappers() {
    let mut vm = new_storage_test_vm("https://data-transfer-item-disabled-mode.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const removedTransfer = new DataTransfer();
  const removedItem = removedTransfer.items.add(
    new File(['file'], 'removed.txt', { type: 'text/plain' })
  );
  removedTransfer.items.remove(0);

  const clearedTransfer = new DataTransfer();
  const clearedItem = clearedTransfer.items.add('cleared', 'text/plain');
  clearedTransfer.items.clear();
  let clearedCallbackCalled = false;
  clearedItem.getAsString(() => { clearedCallbackCalled = true; });

  const clearDataTransfer = new DataTransfer();
  const clearDataItem = clearDataTransfer.items.add('clear-data', 'text/plain');
  clearDataTransfer.clearData('text/plain');
  let clearDataCallbackCalled = false;
  clearDataItem.getAsString(() => { clearDataCallbackCalled = true; });

  return JSON.stringify({
    removed: [
      removedItem.kind,
      removedItem.type,
      removedItem.getAsFile() === null,
      removedItem.webkitGetAsEntry() === null,
      removedTransfer.items.length
    ],
    cleared: [
      clearedItem.kind,
      clearedItem.type,
      clearedCallbackCalled,
      clearedTransfer.items.length
    ],
    clearData: [
      clearDataItem.kind,
      clearDataItem.type,
      clearDataCallbackCalled,
      clearDataTransfer.items.length
    ]
  });
})()
"#,
        )
        .expect("removed DataTransferItem wrappers should enter disabled mode");

    assert_eq!(
        result,
        r#"{"removed":["","",true,true,0],"cleared":["","",false,0],"clearData":["","",false,0]}"#
    );
}

#[test]
fn data_transfer_item_and_list_use_stable_interface_wrappers() {
    let mut vm = new_storage_test_vm("https://data-transfer-item-wrapper.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const dt = new DataTransfer();
  const file = new File(['hi'], 'note.txt', { type: 'text/plain', lastModified: 7 });
  const item = dt.items.add(file);
  const indexed = dt.items[0];
  const fromItem = dt.items.item(0);
  return [
    item instanceof DataTransferItem,
    dt.items instanceof DataTransferItemList,
    indexed === item,
    fromItem === item,
    indexed && indexed.getAsFile && indexed.getAsFile().name,
    dt.items.length
  ].join('|');
})()
"#,
        )
        .expect("DataTransferItem wrappers should evaluate");

    assert_eq!(result, "true|true|true|true|note.txt|1");
}

#[test]
fn data_transfer_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://data-transfer-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    const setter = descriptor && descriptor.set;
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof setter,
      setter ? setter.name : 'none',
      setter ? setter.length : 'none',
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(':');
  };
  const dt = new DataTransfer();
  dt.setData('text/plain', 'alpha');
  const file = new File(['hi'], 'note.txt', { type: 'text/plain', lastModified: 7 });
  const item = dt.items.add(file);
  const entry = item.webkitGetAsEntry();
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__lmDataTransfer') || name.startsWith('__lmFileSystem'))
    .sort();
  const ownNamesBefore = {
    dataTransfer: internalNames(dt),
    itemList: internalNames(dt.items),
    item: internalNames(item),
    entry: internalNames(entry)
  };

  DataTransfer.prototype.__lmDataTransferItems = dt.items;
  DataTransfer.prototype.__lmDataTransferTypes = ['prototype/type'];
  DataTransfer.prototype.__lmDataTransferDropEffect = 'copy';
  DataTransfer.prototype.__lmDataTransferEffectAllowed = 'all';
  DataTransferItemList.prototype.__lmDataTransferItemArray = [item, item];
  DataTransferItem.prototype.__lmDataTransferItemKind = 'string';
  DataTransferItem.prototype.__lmDataTransferItemType = 'prototype/type';
  DataTransferItem.prototype.__lmDataTransferItemFile = new File(['bad'], 'bad.txt');
  FileSystemEntry.prototype.__lmFileSystemEntryName = 'prototype.txt';
  FileSystemEntry.prototype.__lmFileSystemEntryFullPath = '/prototype.txt';
  FileSystemEntry.prototype.__lmFileSystemEntryIsFile = false;
  FileSystemEntry.prototype.__lmFileSystemEntryIsDirectory = true;

  const fakeDataTransfer = Object.create(DataTransfer.prototype);
  const fakeList = Object.create(DataTransferItemList.prototype);
  const fakeItem = Object.create(DataTransferItem.prototype);
  const fakeEntry = Object.create(FileSystemEntry.prototype);
  const entryNameGetter = Object.getOwnPropertyDescriptor(FileSystemEntry.prototype, 'name').get;
  const entryPathGetter = Object.getOwnPropertyDescriptor(FileSystemEntry.prototype, 'fullPath').get;
  const descriptors = {
    dataTransfer: [
      accessorDescriptor(DataTransfer.prototype, 'files'),
      accessorDescriptor(DataTransfer.prototype, 'items'),
      accessorDescriptor(DataTransfer.prototype, 'types'),
      accessorDescriptor(DataTransfer.prototype, 'dropEffect'),
      accessorDescriptor(DataTransfer.prototype, 'effectAllowed')
    ],
    itemList: [
      accessorDescriptor(DataTransferItemList.prototype, 'length')
    ],
    item: [
      accessorDescriptor(DataTransferItem.prototype, 'kind'),
      accessorDescriptor(DataTransferItem.prototype, 'type')
    ],
    entry: [
      accessorDescriptor(FileSystemEntry.prototype, 'filesystem'),
      accessorDescriptor(FileSystemEntry.prototype, 'fullPath'),
      accessorDescriptor(FileSystemEntry.prototype, 'isDirectory'),
      accessorDescriptor(FileSystemEntry.prototype, 'isFile'),
      accessorDescriptor(FileSystemEntry.prototype, 'name')
    ]
  };

  dt.__lmDataTransferItems = fakeList;
  dt.__lmDataTransferTypes = ['own/type'];
  dt.__lmDataTransferDropEffect = 'move';
  dt.__lmDataTransferEffectAllowed = 'copyMove';
  dt.items.__lmDataTransferItemArray = [item, item, item];
  dt.items.__lmDataTransferItemListIndexedLength = 9;
  item.__lmDataTransferItemKind = 'string';
  item.__lmDataTransferItemType = 'own/type';
  item.__lmDataTransferItemFile = new File(['bad'], 'bad.txt');
  entry.__lmFileSystemEntryName = 'own.txt';
  entry.__lmFileSystemEntryFullPath = '/own.txt';
  entry.__lmFileSystemEntryIsFile = false;
  entry.__lmFileSystemEntryIsDirectory = true;

  return JSON.stringify({
    ownNamesBefore,
    descriptors,
    real: [
      dt.getData('text/plain'),
      dt.types.join(','),
      dt.dropEffect,
      dt.effectAllowed,
      dt.items.length,
      item.kind,
      item.type,
      item.getAsFile().name,
      entry.name,
      entry.fullPath,
      entry.isFile,
      entry.isDirectory
    ].join('|'),
    fake: [
      DataTransfer.prototype.getData.call(fakeDataTransfer, 'text/plain'),
      fakeDataTransfer.types === undefined ? 'undefined' : fakeDataTransfer.types.join(','),
      fakeDataTransfer.dropEffect,
      fakeDataTransfer.effectAllowed,
      fakeList.length,
      fakeItem.kind,
      fakeItem.type,
      fakeItem.getAsFile(),
      entryNameGetter.call(fakeEntry),
      entryPathGetter.call(fakeEntry)
    ].map(value => value === null ? 'null' : String(value)).join('|')
  });
})()
"#,
        )
        .expect("DataTransfer declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"ownNamesBefore":{"dataTransfer":[],"itemList":[],"item":[],"entry":[]},"descriptors":{"dataTransfer":["files:function:get files:0:undefined:none:none:true:true","items:function:get items:0:undefined:none:none:true:true","types:function:get types:0:undefined:none:none:true:true","dropEffect:function:get dropEffect:0:function:set dropEffect:1:true:true","effectAllowed:function:get effectAllowed:0:function:set effectAllowed:1:true:true"],"itemList":["length:function:get length:0:undefined:none:none:true:true"],"item":["kind:function:get kind:0:undefined:none:none:true:true","type:function:get type:0:undefined:none:none:true:true"],"entry":["filesystem:function:get filesystem:0:undefined:none:none:true:true","fullPath:function:get fullPath:0:undefined:none:none:true:true","isDirectory:function:get isDirectory:0:undefined:none:none:true:true","isFile:function:get isFile:0:undefined:none:none:true:true","name:function:get name:0:undefined:none:none:true:true"]},"real":"alpha|text/plain,Files|none|none|2|file|text/plain|note.txt|note.txt|/note.txt|true|false","fake":"|undefined|none|uninitialized|0|||null||"}"#
    );
}

#[test]
fn data_transfer_directory_entries_use_private_slots_for_reflection_and_spoofing() {
    use crate::runtime::{RendererDragData, RendererDraggedDirectory, RendererDraggedFile};

    let mut vm = new_rendered_test_vm(
        "https://data-transfer-directory-slots.test/",
        r#"<html><body><div id="drop" style="width: 100px; height: 100px">drop</div></body></html>"#,
    );

    vm.eval(
        r#"
(() => {
  const target = document.getElementById('drop');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__lmDataTransfer') || name.startsWith('__lmFileSystem'))
    .sort();
  window.__directoryDropReport = 'missing';
  target.addEventListener('drop', event => {
    const item = event.dataTransfer.items[0];
    const types = event.dataTransfer.types;
    const entry = item.webkitGetAsEntry();
    const reader = entry.createReader();

    const ownNamesBefore = {
      dataTransfer: internalNames(event.dataTransfer),
      itemList: internalNames(event.dataTransfer.items),
      item: internalNames(item),
      entry: internalNames(entry),
      reader: internalNames(reader)
    };

    event.dataTransfer.__lmDataTransferItems = null;
    event.dataTransfer.__lmDataTransferTypes = ['own/type'];
    event.dataTransfer.items.__lmDataTransferItemArray = [];
    item.__lmDataTransferItemKind = 'string';
    item.__lmDataTransferItemType = 'own/type';
    entry.__lmFileSystemEntryName = 'own';
    entry.__lmFileSystemEntryFullPath = '/own';
    entry.__lmFileSystemEntryIsDirectory = false;
    entry.__lmFileSystemEntryIsFile = true;
    entry.__lmFileSystemDirectoryEntryEntries = [];
    reader.__lmFileSystemDirectoryReaderEntries = [];
    reader.__lmFileSystemDirectoryReaderOffset = 0;

    const reader2 = entry.createReader();

    window.__directoryDropReport = JSON.stringify({
      ownNamesBefore,
      transfer: [
        types.join(','),
        types === event.dataTransfer.types,
        Object.isFrozen(types),
        event.dataTransfer.items.length,
        item.kind,
        item.type,
        entry.name,
        entry.fullPath,
        entry.isDirectory,
        entry.isFile
      ].join('|'),
      freshReaderOwnNames: internalNames(reader2)
    });
  });
})()
"#,
    )
    .expect("directory drop listener setup should evaluate");

    let drag_data = RendererDragData {
        items: Vec::new(),
        files: Vec::new(),
        directories: vec![RendererDraggedDirectory {
            name: "docs".to_owned(),
            files: vec![RendererDraggedFile {
                bytes: b"hello".to_vec(),
                mime_type: "text/plain".to_owned(),
                name: "child.txt".to_owned(),
                last_modified: 7.0,
            }],
            directories: vec![RendererDraggedDirectory {
                name: "nested".to_owned(),
                files: Vec::new(),
                directories: Vec::new(),
            }],
        }],
        drag_operations_mask: 1,
    };
    vm.dispatch_drag_event_at_point(10.0, 10.0, "drop", drag_data, 0)
        .expect("directory drop should dispatch");

    let result = vm
        .eval("window.__directoryDropReport")
        .expect("directory drop report should be readable");

    assert_eq!(
        result,
        r#"{"ownNamesBefore":{"dataTransfer":[],"itemList":[],"item":[],"entry":[],"reader":[]},"transfer":"Files|true|true|1|file||docs|/docs|true|false","freshReaderOwnNames":[]}"#
    );
}

#[test]
fn data_transfer_string_surface_and_drag_event_constructor_work() {
    let mut vm = new_storage_test_vm("https://drag-event-data-transfer.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const dt = new DataTransfer();
  dt.setData('text', 'alpha');
  const htmlItem = dt.items.add('<b>beta</b>', 'text/html');
  let stringPayload = '';
  htmlItem.getAsString(value => {
    stringPayload = value;
  });
  const event = new DragEvent('drop', {
    dataTransfer: dt,
    clientX: 12,
    clientY: 34
  });
  const typesBeforeClear = dt.types.join(',');
  dt.clearData('text/html');
  return [
    typeof DragEvent,
    event instanceof DragEvent,
    event instanceof MouseEvent,
    event.dataTransfer === dt,
    dt.getData('text/plain'),
    typesBeforeClear,
    dt.types.join(','),
    htmlItem.kind,
    htmlItem.type,
    stringPayload,
    dt.dropEffect,
    dt.effectAllowed,
    event.clientX,
    event.clientY,
    dt.items.length
  ].join('|');
})()
"#,
        )
        .expect("DragEvent/DataTransfer string surface should evaluate");

    assert_eq!(
        result,
        "function|true|true|true|alpha|text/plain,text/html|text/plain||||none|none|12|34|1"
    );
}

#[test]
fn input_and_drag_event_data_transfer_init_enforce_nullable_interface_conversion() {
    let mut vm = new_storage_test_vm("https://drag-event-data-transfer-conversion.test/");

    let result = vm
        .eval(
            r#"
JSON.stringify([DragEvent, InputEvent].map(EventConstructor => {
  const outcome = dataTransfer => {
    try {
      new EventConstructor('drop', { dataTransfer });
      return 'accepted';
    } catch (error) {
      return error.name;
    }
  };
  const transfer = new DataTransfer();
  const fakeTransfer = Object.create(DataTransfer.prototype);
  let getterError = 'missing';
  try {
    new EventConstructor('drop', {
      get dataTransfer() {
        throw new RangeError('sentinel');
      }
    });
  } catch (error) {
    getterError = `${error.name}:${error.message}`;
  }
  return [
    new EventConstructor('drop').dataTransfer === null,
    new EventConstructor('drop', { dataTransfer: null }).dataTransfer === null,
    new EventConstructor('drop', { dataTransfer: undefined }).dataTransfer === null,
    new EventConstructor('drop', { dataTransfer: transfer }).dataTransfer === transfer,
    outcome({}),
    outcome(fakeTransfer),
    outcome(1),
    getterError
  ].join('|');
}))
"#,
        )
        .expect("DragEvent dataTransfer conversion should evaluate");

    assert_eq!(
        result,
        r#"["true|true|true|true|TypeError|TypeError|TypeError|RangeError:sentinel","true|true|true|true|TypeError|TypeError|TypeError|RangeError:sentinel"]"#
    );
}

#[test]
fn readonly_input_data_transfer_retains_native_items_after_source_changes() {
    let mut vm = new_storage_test_vm("https://input-transfer-snapshot.test/");
    vm.eval(
        r#"
        globalThis.sourceTransfer = new DataTransfer();
        sourceTransfer.setData('text/plain', 'alpha');
        globalThis.sourceItems = sourceTransfer.items;
        globalThis.sourceFile = new File(['payload'], 'a.txt', {type: 'text/plain'});
        sourceItems.add(sourceFile);
        for (const key of ['getData', 'items', 'types', 'files', 'effectAllowed']) {
            Object.defineProperty(sourceTransfer, key, {get() { throw new Error('page getter'); }});
        }
    "#,
    )
    .expect("source DataTransfer fixture");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "sourceTransfer").unwrap();
        let source = global.get(scope, key.into()).unwrap();
        let source = v8::Local::<v8::Object>::try_from(source).unwrap();
        let snapshot = crate::context_bootstrap::readonly_data_transfer_for_input(scope, source)
            .expect("copy native DataTransfer state without page getters");
        let key = v8::String::new(scope, "snapshotTransfer").unwrap();
        assert_eq!(global.set(scope, key.into(), snapshot.into()), Some(true));
        Ok(())
    })
    .expect("input transfer snapshot");
    assert_eq!(
        vm.eval(
            r#"(() => {
        sourceItems.clear();
        sourceTransfer.setData('text/plain', 'beta');
        const retainedFile = snapshotTransfer.items[1].getAsFile();
        snapshotTransfer.items.clear();
        snapshotTransfer.setData('text/plain', 'changed');
        return [snapshotTransfer instanceof DataTransfer,
            snapshotTransfer.getData('text/plain'), snapshotTransfer.types.join(','),
            snapshotTransfer.items.length, snapshotTransfer.files.length,
            retainedFile === sourceFile, snapshotTransfer.files[0] === sourceFile,
            retainedFile.size, retainedFile.name].join('|');
    })()"#
        )
        .expect("read-only retained payload"),
        "true|alpha|text/plain,Files|2|1|true|true|7|a.txt"
    );
}

#[test]
fn data_transfer_types_is_a_cached_frozen_array_per_item_list_mutation() {
    let mut vm = new_storage_test_vm("https://data-transfer-types-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  const otherTransfer = new DataTransfer();
  const initial = transfer.types;
  const otherInitial = otherTransfer.types;
  transfer.types = ['replacement'];
  const afterReadonlyAssignment = transfer.types;

  transfer.clearData();
  transfer.items.clear();
  transfer.items.remove(0);
  const afterEmptyNoOps = transfer.types;

  transfer.setData('text/plain', 'alpha');
  const afterAdd = transfer.types;
  transfer.clearData('text/html');
  const afterMissingClear = transfer.types;
  transfer.setData('text/plain', 'beta');
  const afterReplace = transfer.types;
  transfer.items.clear();
  const afterClear = transfer.types;
  transfer.items.clear();

  return JSON.stringify({
    arrays: [Array.isArray(initial), Object.isFrozen(initial)],
    initialIdentity: [transfer !== otherTransfer, initial !== otherInitial],
    readonlyIdentity: initial === afterReadonlyAssignment,
    emptyNoOpsPreserveIdentity: initial === afterEmptyNoOps,
    addReplacesIdentity: initial !== afterAdd,
    missingClearPreservesIdentity: afterAdd === afterMissingClear,
    replacementReplacesIdentity: afterMissingClear !== afterReplace,
    clearReplacesIdentity: afterReplace !== afterClear,
    finalNoOpPreservesIdentity: afterClear === transfer.types,
    snapshots: [initial.join(','), afterAdd.join(','), afterReplace.join(','), afterClear.join(',')],
    frozenAfterMutations: [afterAdd, afterReplace, afterClear].every(Object.isFrozen)
  });
})()
"#,
        )
        .expect("DataTransfer types FrozenArray semantics should evaluate");

    assert_eq!(
        result,
        r#"{"arrays":[true,true],"initialIdentity":[true,true],"readonlyIdentity":true,"emptyNoOpsPreserveIdentity":true,"addReplacesIdentity":true,"missingClearPreservesIdentity":true,"replacementReplacesIdentity":true,"clearReplacesIdentity":true,"finalNoOpPreservesIdentity":true,"snapshots":["","text/plain","text/plain",""],"frozenAfterMutations":true}"#
    );
}

#[test]
fn data_transfer_types_lazily_publishes_first_snapshot_after_mutation() {
    let mut vm = new_storage_test_vm("https://data-transfer-types-lazy-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  transfer.setData('text/plain', 'alpha');
  const first = transfer.types;
  return [first.join(','), Object.isFrozen(first), first === transfer.types].join('|');
})()
"#,
        )
        .expect("DataTransfer types should publish its first snapshot lazily");

    assert_eq!(result, "text/plain|true|true");
}

#[test]
fn data_transfer_types_snapshot_tracks_file_and_failed_item_mutations() {
    let mut vm = new_storage_test_vm("https://data-transfer-file-types-cache.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  const files = transfer.files;
  const initial = transfer.types;
  const firstFile = new File(['first'], 'first.txt');
  const secondFile = new File(['second'], 'second.txt');

  transfer.items.add(firstFile);
  const afterFirstFile = transfer.types;
  transfer.setData('text/plain', 'alpha');
  const afterText = transfer.types;

  let duplicateError = '';
  try {
    transfer.items.add('duplicate', 'text/plain');
  } catch (error) {
    duplicateError = error.name;
  }
  const afterDuplicate = transfer.types;
  const invalidAdd = transfer.items.add({});
  const afterInvalidAdd = transfer.types;
  transfer.items.remove(99);
  const afterInvalidRemoval = transfer.types;

  transfer.items.add(secondFile);
  const afterSecondFile = transfer.types;
  transfer.items.remove(0);
  const afterFirstRemoval = transfer.types;
  transfer.items.remove(1);
  const afterLastFileRemoval = transfer.types;

  return JSON.stringify({
    values: [
      initial.join(','),
      afterFirstFile.join(','),
      afterText.join(','),
      afterSecondFile.join(','),
      afterFirstRemoval.join(','),
      afterLastFileRemoval.join(',')
    ],
    identities: [
      initial !== afterFirstFile,
      afterFirstFile !== afterText,
      afterText === afterDuplicate,
      afterDuplicate === afterInvalidAdd,
      afterInvalidAdd === afterInvalidRemoval,
      afterInvalidRemoval !== afterSecondFile,
      afterSecondFile !== afterFirstRemoval,
      afterFirstRemoval !== afterLastFileRemoval
    ],
    frozen: [
      initial,
      afterFirstFile,
      afterText,
      afterSecondFile,
      afterFirstRemoval,
      afterLastFileRemoval
    ].every(Object.isFrozen),
    failures: [duplicateError, invalidAdd === null],
    liveFiles: [transfer.files === files, files.length]
  });
})()
"#,
        )
        .expect("DataTransfer file types FrozenArray semantics should evaluate");

    assert_eq!(
        result,
        r#"{"values":["","Files","text/plain,Files","text/plain,Files","text/plain,Files","text/plain"],"identities":[true,true,true,true,true,true,true,true],"frozen":true,"failures":["NotSupportedError",true],"liveFiles":[true,0]}"#
    );
}

#[test]
fn data_transfer_file_list_reference_tracks_item_mutations() {
    let mut vm = new_storage_test_vm("https://data-transfer-live-files.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const transfer = new DataTransfer();
  const files = transfer.files;
  const first = new File(["first"], "first.txt");
  const second = new File(["second"], "second.txt");
  transfer.items.add(first);
  transfer.items.add(second);
  const afterAdd = [
    transfer.files === files,
    files.length,
    files[0] === first,
    files.item(1) === second
  ].join(":");
  transfer.items.remove(0);
  const afterRemove = [
    transfer.files === files,
    files.length,
    files[0] === second,
    files[1] === undefined,
    files.item(1) === null
  ].join(":");
  return `${afterAdd}|${afterRemove}`;
})()
"#,
        )
        .expect("live DataTransfer FileList probe should evaluate");

    assert_eq!(result, "true:2:true:true|true:1:true:true:true");
}

#[test]
fn mouse_dragstart_bubbles_to_window_once() {
    let mut vm = new_rendered_test_vm(
        "https://dragstart-bubbles-once.test/",
        r#"<html><body><div id="drag" draggable="true">drag</div></body></html>"#,
    );
    vm.eval(
        r#"
(() => {
  window.__dragStarts = 0;
  window.__dragStartLog = [];
  window.addEventListener('dragstart', event => {
    window.__dragStarts += 1;
    window.__dragStartLog.push([
      event.type,
      event.currentTarget === window,
      !!event.dataTransfer
    ].join(':'));
  });
})()
"#,
    )
    .expect("dragstart listener setup should evaluate");

    vm.publish_layout_for_test()
        .expect("publish geometry before coordinate input");

    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousedown", 0, None, 0.0, 0.0)
        .expect("mousedown should dispatch");
    vm.dispatch_mouse_event_at_point(20.0, 20.0, "mousemove", 0, Some(1), 0.0, 0.0)
        .expect("mousemove should start drag");

    let result = vm
        .eval(
            r#"
(() => [window.__dragStarts, window.__dragStartLog.join('|')].join('|'))()
"#,
        )
        .expect("dragstart log should evaluate");
    assert_eq!(result, "1|dragstart:true:true");
}

#[test]
fn file_list_uses_illegal_constructor_and_native_file_sources() {
    let mut vm = new_storage_test_vm("https://file-list-construction.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        let reads = 0;
        const input = {get length() { reads++; return 0; }};
        let rejected = false;
        try { new FileList(input); } catch (error) { rejected = error instanceof TypeError; }
        const transfer = new DataTransfer();
        const file = new File(['payload'], 'example.txt');
        transfer.items.add(file);
        const native = transfer.files;
        return JSON.stringify([rejected, reads, native instanceof FileList,
          native.length, native.item(0) === file, native.item(1) === null]);
      })()
    "#,
        )
        .unwrap();
    assert_eq!(result, "[true,0,true,1,true,true]");
}
