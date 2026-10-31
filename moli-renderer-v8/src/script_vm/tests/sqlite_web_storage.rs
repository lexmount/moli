use super::*;

#[test]
fn storage_estimate_does_not_open_local_storage_or_depend_on_its_connection() {
    for closed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unused/localstorage.sqlite");
        let runtime = moli_sqlite::Runtime::new().unwrap();
        let (local, owner) = moli_web_storage::new_lazy_sqlite_web_storage_store(
            path.clone(),
            moli_sqlite::OpenMode::Create,
            runtime.handle(),
        )
        .unwrap();
        let handles =
            crate::RendererWebStorageHandles::new(local, crate::new_shared_web_storage_store());
        let mut vm =
            new_storage_page_task_executor_test_vm("https://quota-without-webstorage.test/");
        vm.set_web_storage_handles(&handles);
        if closed {
            owner.close().unwrap();
        }
        vm.eval("globalThis.estimateResult = 'pending'; navigator.storage.estimate().then(value => {estimateResult = JSON.stringify([value.usage, typeof value.quota]);}, error => {estimateResult = error.name;});").unwrap();
        assert_eq!(
            vm.eval_after_selected_page_tasks("estimateResult").unwrap(),
            "[0,\"number\"]"
        );
        assert!(!path.parent().unwrap().exists());
        drop(vm);
        owner.close().unwrap();
    }
}

#[test]
fn sqlite_storage_wrappers_share_utf16_data_without_sharing_session_storage() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = moli_sqlite::Runtime::new().unwrap();
    let (local, owner) = moli_web_storage::new_sqlite_web_storage_store(
        directory.path().join("localstorage.sqlite"),
        moli_sqlite::OpenMode::Create,
        runtime.handle(),
    )
    .unwrap();
    let first_handles =
        crate::RendererWebStorageHandles::new(local.clone(), crate::new_shared_web_storage_store());
    let second_handles =
        crate::RendererWebStorageHandles::new(local, crate::new_shared_web_storage_store());
    let mut first = new_storage_test_vm("https://sqlite-storage.test/a");
    first.set_web_storage_handles(&first_handles);
    let mut second = new_storage_test_vm("https://sqlite-storage.test/b");
    second.set_web_storage_handles(&second_handles);
    first.eval("localStorage.setItem('key', String.fromCharCode(0xd800, 0, 0xdc00)); sessionStorage.setItem('private','first'); localStorage['property']='shared';").unwrap();
    assert_eq!(second.eval("JSON.stringify([Array.from({length:3},(_,i)=>localStorage.getItem('key').charCodeAt(i)),localStorage.property,localStorage.length,sessionStorage.getItem('private')])").unwrap(),"[[55296,0,56320],\"shared\",2,null]");
    second.eval("delete localStorage.property").unwrap();
    assert_eq!(
        first
            .eval("String(localStorage.getItem('property'))")
            .unwrap(),
        "null"
    );
    drop(first);
    drop(second);
    owner.close().unwrap();
}
#[test]
fn retired_sqlite_storage_reports_unknown_error_through_all_read_surfaces() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = moli_sqlite::Runtime::new().unwrap();
    let (local, owner) = moli_web_storage::new_sqlite_web_storage_store(
        directory.path().join("localstorage.sqlite"),
        moli_sqlite::OpenMode::Create,
        runtime.handle(),
    )
    .unwrap();
    let handles =
        crate::RendererWebStorageHandles::new(local, crate::new_shared_web_storage_store());
    let mut vm = new_storage_test_vm("https://closed-sqlite.test/");
    vm.set_web_storage_handles(&handles);
    vm.eval("localStorage.setItem('entry','value');globalThis.storage=localStorage;")
        .unwrap();
    owner.close().unwrap();
    let result = vm
        .eval(
            r#"JSON.stringify([
        ()=>storage.getItem('entry'),()=>storage.key(0),()=>storage.length,
        ()=>storage.entry,()=>('entry' in storage),()=>Object.keys(storage),
        ()=>storage.setItem('entry','changed'),()=>storage.removeItem('entry'),()=>storage.clear()
    ].map(read=>{try {read();return 'missing error';} catch(error) {return error.name;}}))"#,
        )
        .unwrap();
    assert_eq!(
        result,
        "[\"UnknownError\",\"UnknownError\",\"UnknownError\",\"UnknownError\",\"UnknownError\",\"UnknownError\",\"UnknownError\",\"UnknownError\",\"UnknownError\"]"
    );
    assert_eq!(vm.eval("typeof storage.key").unwrap(), "function");
}
