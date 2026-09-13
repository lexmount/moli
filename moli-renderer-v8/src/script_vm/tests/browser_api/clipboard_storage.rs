use super::*;

const CLIPBOARD_SNAPSHOT_PROBE: &str = r#"
(async () => {
  const clipboard = navigator.clipboard;
  const source = new Blob(['hello 世界 🦊'], {type: 'text/plain'});
  const item = new ClipboardItem({'text/plain': source});
  await clipboard.write([item]);
  const first = (await clipboard.read())[0];
  const second = (await clipboard.read())[0];
  const blob = await first.getType('text/plain');
  if (first === item || first === second || blob === source) return 'reused author objects';
  if (!(first instanceof ClipboardItem) || !(blob instanceof Blob)) return 'wrong realm';
  if (first.types.join(',') !== 'text/plain' || await blob.text() !== 'hello 世界 🦊') return 'bytes changed';
  Object.defineProperty(item, 'getType', {get() {throw Error('author getType');}});
  Object.defineProperty(source, 'text', {get() {throw Error('author text');}});
  if (await clipboard.readText() !== 'hello 世界 🦊') return 'read author properties';
  await clipboard.writeText('replacement');
  const replacement = (await clipboard.read())[0];
  if (replacement === first || await (await replacement.getType('text/plain')).text() !== 'replacement') {
    return 'replacement failed';
  }
  return 'ok';
})()
"#;

fn assert_clipboard_probe(vm: &mut ScriptVm, script: &str) {
    vm.eval(&format!(
        "globalThis.__clipboardStorageResult = 'pending'; ({script}).then(\
         result => globalThis.__clipboardStorageResult = result, \
         error => globalThis.__clipboardStorageResult = String(error));"
    ))
    .expect("clipboard probe should evaluate");
    vm.eval("0").expect("clipboard promises should drain");
    assert_eq!(
        vm.eval("globalThis.__clipboardStorageResult")
            .expect("clipboard result should evaluate"),
        "ok"
    );
}

#[test]
fn clipboard_storage_reads_new_items_and_blobs_from_byte_snapshots() {
    let mut vm = new_storage_test_vm("https://clipboard-storage.test/");
    assert_clipboard_probe(&mut vm, CLIPBOARD_SNAPSHOT_PROBE);
}

#[test]
fn clipboard_storage_waits_for_all_representations_and_preserves_data_after_rejection() {
    let mut vm = new_storage_test_vm("https://clipboard-storage.test/");
    assert_clipboard_probe(
        &mut vm,
        r#"
(async () => {
  const clipboard = navigator.clipboard;
  await clipboard.writeText('before');
  let resolve;
  const pending = new Promise(done => resolve = done);
  const write = clipboard.write([new ClipboardItem({
    'text/plain': Promise.resolve('after'), 'text/html': pending
  })]);
  if (await clipboard.readText() !== 'before') return 'partially committed write';
  resolve(new Blob(['<b>after</b>'], {type: 'text/html'}));
  await write;
  const item = (await clipboard.read())[0];
  if (item.types.join(',') !== 'text/plain,text/html') return 'representation order';
  if (await clipboard.readText() !== 'after' ||
      await (await item.getType('text/html')).text() !== '<b>after</b>') return 'representation bytes';
  try {
    await clipboard.write([new ClipboardItem({
      'text/plain': 'bad replacement',
      'text/html': new Blob(['wrong type'], {type: 'text/plain'})
    })]);
    return 'accepted mismatched representation';
  } catch (error) {
    if (error.name !== 'NotAllowedError') return 'wrong rejection';
  }
  if (await clipboard.readText() !== 'after') return 'failed write replaced clipboard';
  const sentinel = Error('rejected representation');
  let reject;
  const rejection = new Promise((_, fail) => reject = fail);
  const failedWrite = clipboard.write([new ClipboardItem({'text/plain': rejection})]);
  reject(sentinel);
  try {await failedWrite; return 'accepted rejected representation';}
  catch (error) {if (error !== sentinel) return 'replaced rejection reason';}
  if (await clipboard.readText() !== 'after') return 'rejection cleared clipboard';
  return 'ok';
})()
"#,
    );
}

const CLIPBOARD_FRAME_PROBE: &str = r#"
(async () => {
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => frame.onload = resolve);
  document.body.append(frame);
  await loaded;
  const child = frame.contentWindow;
  frame.focus();
  await navigator.clipboard.writeText('parent → child');
  if (await child.navigator.clipboard.readText() !== 'parent → child') return 'parent write invisible';
  const childItem = (await child.navigator.clipboard.read())[0];
  const childBlob = await childItem.getType('text/plain');
  if (!(childItem instanceof child.ClipboardItem) || !(childBlob instanceof child.Blob)) return 'child realm';
  await child.navigator.clipboard.write([new child.ClipboardItem({
    'text/plain': new child.Blob(['child → parent'], {type: 'text/plain'})
  })]);
  if (await navigator.clipboard.readText() !== 'child → parent') return 'child write invisible';
  const parentItem = (await navigator.clipboard.read())[0];
  if (!(parentItem instanceof ClipboardItem) || !((await parentItem.getType('text/plain')) instanceof Blob)) {
    return 'parent realm';
  }
  document.body.tabIndex = 0;
  document.body.focus();
  frame.remove();
  if (await navigator.clipboard.readText() !== 'child → parent') return 'frame removal cleared clipboard';
  return 'ok';
})()
"#;

#[test]
fn clipboard_storage_is_shared_with_child_realms() {
    let mut vm = new_parsed_test_vm(
        "https://clipboard-storage.test/",
        "<!doctype html><body></body>",
    );
    // The synchronous VM fixture does not run the frame's later load task.
    let script = CLIPBOARD_FRAME_PROBE.replace("  await loaded;", "  void loaded;");
    assert_clipboard_probe(&mut vm, &script);
}

fn clipboard_permission(
    name: &str,
    setting: &str,
) -> crate::protocol_types::PermissionOverrideRegistration {
    crate::protocol_types::PermissionOverrideRegistration {
        permission: serde_json::json!({"name": name}),
        setting: setting.to_owned(),
        origin: Some("https://clipboard-storage.test".to_owned()),
        embedded_origin: None,
    }
}

#[test]
fn clipboard_storage_respects_independent_read_and_write_permission_denials() {
    let mut vm = new_storage_test_vm("https://clipboard-storage.test/");
    vm.set_permission_overrides(&[clipboard_permission("clipboard-read", "denied")]);
    assert_clipboard_probe(
        &mut vm,
        r#"
(async () => {
  await navigator.clipboard.writeText('write still allowed');
  for (const operation of ['read', 'readText']) {
    try {await navigator.clipboard[operation](); return 'allowed denied ' + operation;}
    catch (error) {if (error.name !== 'NotAllowedError') return 'read error';}
  }
  return 'ok';
})()
"#,
    );
    vm.set_permission_overrides(&[clipboard_permission("clipboard-write", "denied")]);
    assert_clipboard_probe(
        &mut vm,
        r#"
(async () => {
  const writes = [
    () => navigator.clipboard.writeText('denied replacement'),
    () => navigator.clipboard.write([new ClipboardItem({'text/plain': 'denied replacement'})])
  ];
  for (const write of writes) {
    try {await write(); return 'allowed denied write';}
    catch (error) {if (error.name !== 'NotAllowedError') return 'write error';}
  }
  if (await navigator.clipboard.readText() !== 'write still allowed') return 'denied write changed data';
  const item = (await navigator.clipboard.read())[0];
  if (await (await item.getType('text/plain')).text() !== 'write still allowed') return 'read denied by write permission';
  return 'ok';
})()
"#,
    );
}

#[test]
fn clipboard_storage_rechecks_write_permission_after_pending_data_settles() {
    let mut vm = new_storage_test_vm("https://clipboard-storage.test/");
    vm.eval(
        r#"
      navigator.clipboard.writeText('before revocation');
      const pending = new Promise(resolve => globalThis.__clipboardResolve = resolve);
      navigator.clipboard.write([new ClipboardItem({'text/plain': pending})]).then(
        () => globalThis.__clipboardWriteResult = 'accepted',
        error => globalThis.__clipboardWriteResult = error.name
      );
    "#,
    )
    .expect("pending clipboard write should start");
    vm.set_permission_overrides(&[clipboard_permission("clipboard-write", "denied")]);
    vm.eval("__clipboardResolve('after revocation')")
        .expect("clipboard data should settle");
    assert_eq!(
        vm.eval("__clipboardWriteResult").expect("write result"),
        "NotAllowedError"
    );
    assert_clipboard_probe(
        &mut vm,
        r#"
(async () => (await navigator.clipboard.readText()) === 'before revocation' ? 'ok' : 'revoked write committed')()
"#,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn clipboard_storage_survives_page_teardown_and_keeps_browser_contexts_separate() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("clipboard test loader");
    let owner = crate::runtime::RendererBrowserContextRuntime::new();
    let new_page = || {
        crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
            url::Url::parse("https://clipboard-storage.test/").expect("clipboard URL"),
            &loader,
            owner.handle(),
        )
    };
    {
        let mut page = new_page();
        assert_clipboard_probe(
            &mut page,
            r#"
(async () => {
  await navigator.clipboard.write([new ClipboardItem({
    'text/plain': new Blob(['survives teardown'], {type: 'text/plain'}),
    'image/png': new Blob([Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAABkAAAAZCAIAAABLixI0AAAAJklEQVR4nGJg+M9ANTRq1qhZo2aNmjVq1qhZo2ZhIAAAAAD//wMA1XBuro0zsokAAAAASUVORK5CYII='), ch => ch.charCodeAt(0))], {type: 'image/png'})
  })]);
  return 'ok';
})()
"#,
        );
    }
    let mut replacement = new_page();
    assert_clipboard_probe(
        &mut replacement,
        r#"
(async () => {
  if (await navigator.clipboard.readText() !== 'survives teardown') return 'lost page data';
  const item = (await navigator.clipboard.read())[0];
  const blob = await item.getType('image/png');
  if (!(item instanceof ClipboardItem) || !(blob instanceof Blob)) return 'retired realm';
  const expected = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAABkAAAAZCAIAAABLixI0AAAAJklEQVR4nGJg+M9ANTRq1qhZo2aNmjVq1qhZo2ZhIAAAAAD//wMA1XBuro0zsokAAAAASUVORK5CYII='), ch => ch.charCodeAt(0));
  if (Array.from(new Uint8Array(await blob.arrayBuffer())).join(',') !== expected.join(',')) return 'binary data';
  return 'ok';
})()
"#,
    );
    let mut separate = new_storage_test_vm("https://clipboard-storage.test/");
    assert_clipboard_probe(
        &mut separate,
        r#"
(async () => {
  if (await navigator.clipboard.readText() !== '' || (await navigator.clipboard.read()).length !== 0) {
    return 'another browser context clipboard';
  }
  return 'ok';
})()
"#,
    );
}
