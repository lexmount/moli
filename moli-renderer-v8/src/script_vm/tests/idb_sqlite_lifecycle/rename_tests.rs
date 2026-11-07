use super::*;

#[tokio::test]
async fn idb_sqlite_schema_rename_freezes_finished_handles() {
    let (mut vm, loader, backend) = fixture();
    vm.eval("globalThis.oldStore = db.transaction('records').objectStore('records'); globalThis.oldIndex = oldStore.index('unique'); 'queued'")
        .unwrap();
    backend.drain(&mut vm, &loader).await;
    backend.upgrade(&mut vm);
    vm.eval(
        "globalThis.liveStore = upgrade.objectStore('records'); globalThis.liveIndex = liveStore.index('unique'); liveIndex.name = 'renamed-index'; liveStore.name = 'renamed'; 'queued'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([oldStore.name,oldIndex.name,Array.from(oldStore.indexNames),liveStore.name,liveIndex.name,Array.from(liveStore.indexNames)])")
            .unwrap(),
        r#"["records","unique",["unique"],"renamed","renamed-index",["renamed-index"]]"#,
    );
    backend.drain(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("[oldStore.name,oldIndex.name,liveStore.name,liveIndex.name].join('|')")
            .unwrap(),
        "records|unique|renamed|renamed-index",
    );
}
