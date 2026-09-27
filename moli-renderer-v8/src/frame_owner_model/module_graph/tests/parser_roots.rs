use crate::frame_owner_model::FrameDocumentParserRootTerminalResult;

use super::*;
use crate::frame_owner_model::FrameDocumentModuleScriptTerminalTask;

fn into_parser_root_terminal_works(
    task: FrameDocumentModuleScriptTerminalBatchTask,
) -> Vec<FrameDocumentParserRootTerminalWork> {
    let module_script_terminal_tasks = task.into_payload();
    module_script_terminal_tasks
        .into_iter()
        .map(|task| match task {
            FrameDocumentModuleScriptTerminalTask::ParserRoot(work) => *work,
            FrameDocumentModuleScriptTerminalTask::SingleModule(_) => {
                panic!("expected parser-root terminal work")
            }
            FrameDocumentModuleScriptTerminalTask::Dependency(_) => {
                panic!("expected parser-root terminal work")
            }
        })
        .collect()
}

#[test]
fn child_parser_root_clients_fan_out_from_module_map_entry() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let other_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(4), DocumentId(5));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(6);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let key = ModuleMapKey::java_script(root_url.clone());
    let first_client = parser_root_client(10, &root_url);
    let joined_client = parser_root_client(11, &root_url);

    let reservation =
        store.reserve_parser_root_module_client(owner, realm_id, key.clone(), first_client);
    assert!(matches!(
        reservation.fetch_disposition(),
        FrameDocumentModuleFetchDisposition::StartedFetch(_)
    ));

    let joined_reservation =
        store.reserve_parser_root_module_client(owner, realm_id, key.clone(), joined_client);
    assert!(matches!(
        joined_reservation.fetch_disposition(),
        FrameDocumentModuleFetchDisposition::JoinedFetching(_)
    ));
    assert_eq!(
        store
            .current_document_modulator_entry(document_owner, realm_id)
            .expect("child document modulator entry should exist")
            .document_modulator
            .parser_root_module_client_count_for_testing(),
        2,
        "started and joined parser roots should be stored as ModuleMapEntry clients"
    );
    assert!(
        store
            .finish_parser_root_module_fetch(
                other_owner,
                realm_id,
                key.clone(),
                Ok(ModuleGraphFetchedSource::new(
                    root_url.clone(),
                    false,
                    ModuleSource::text("export const wrong = true;".to_owned()),
                )),
            )
            .is_empty(),
        "parser root fetch clients must be scoped to their child document owner"
    );
    assert!(
        store
            .finish_parser_root_module_fetch(
                owner,
                FrameRealmId(99),
                key.clone(),
                Ok(ModuleGraphFetchedSource::new(
                    root_url.clone(),
                    false,
                    ModuleSource::text("export const stale = true;".to_owned()),
                )),
            )
            .is_empty(),
        "parser root fetch completions must be scoped to the current child FrameRealm"
    );
    let tasks = store.finish_parser_root_module_fetch(
        owner,
        realm_id,
        key.clone(),
        Ok(ModuleGraphFetchedSource::new(
            root_url.clone(),
            false,
            ModuleSource::text("export const value = 1;".to_owned()),
        )),
    );
    assert_eq!(
        tasks.len(),
        1,
        "joined parser root clients should stay in one module-owner terminal event"
    );
    let task = tasks.into_iter().next().expect("parser root terminal task");
    assert_eq!(task.owner(), owner);
    assert_eq!(task.realm_id(), realm_id);
    let works = into_parser_root_terminal_works(task);
    assert_eq!(works.len(), 2);
    assert_eq!(
        works[0].parser_root_payload().script_handle(),
        DomHandle::new(10)
    );
    assert_eq!(
        works[1].parser_root_payload().script_handle(),
        DomHandle::new(11)
    );
    assert!(matches!(
        works[0].result(),
        FrameDocumentParserRootTerminalResult::Fetched(_)
    ));
    assert!(matches!(
        works[1].result(),
        FrameDocumentParserRootTerminalResult::Fetched(_)
    ));
}

#[test]
fn child_parser_root_already_terminal_entries_emit_owner_tasks() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(6);
    let fetched_url =
        Url::parse("https://child-module-graph.test/already-fetched.js").expect("root url");
    let fetched_key = ModuleMapKey::java_script(fetched_url.clone());

    let first_reservation = store.reserve_parser_root_module_client(
        owner,
        realm_id,
        fetched_key.clone(),
        parser_root_client(10, &fetched_url),
    );
    assert!(matches!(
        first_reservation.fetch_disposition(),
        FrameDocumentModuleFetchDisposition::StartedFetch(_)
    ));
    let first_tasks = store.finish_parser_root_module_fetch(
        owner,
        realm_id,
        fetched_key.clone(),
        Ok(ModuleGraphFetchedSource::new(
            fetched_url.clone(),
            false,
            ModuleSource::text("export const value = 1;".to_owned()),
        )),
    );
    let first_task = first_tasks
        .into_iter()
        .next()
        .expect("initial terminal task");
    let mut first_parser_root_works = into_parser_root_terminal_works(first_task);
    assert_eq!(first_parser_root_works.len(), 1);
    let first_parser_root_work = first_parser_root_works
        .pop()
        .expect("initial parser root terminal work");
    assert_eq!(
        first_parser_root_work.parser_root_payload().script_handle(),
        DomHandle::new(10)
    );
    assert!(matches!(
        first_parser_root_work.result(),
        FrameDocumentParserRootTerminalResult::Fetched(_)
    ));

    let already_fetched_reservation = store.reserve_parser_root_module_client(
        owner,
        realm_id,
        fetched_key.clone(),
        parser_root_client(11, &fetched_url),
    );
    assert!(matches!(
        already_fetched_reservation.fetch_disposition(),
        FrameDocumentModuleFetchDisposition::AlreadyFetched(_)
    ));
    let tasks = store.take_ready_document_modulator_terminal_batches(owner, realm_id);
    assert_eq!(tasks.len(), 1);
    let task = tasks.into_iter().next().expect("synthetic terminal task");
    let mut works = into_parser_root_terminal_works(task);
    assert_eq!(works.len(), 1);
    let work = works.pop().expect("synthetic parser root terminal work");
    assert!(matches!(
        work.result(),
        FrameDocumentParserRootTerminalResult::Fetched(_)
    ));
    assert_eq!(
        work.parser_root_payload().script_handle(),
        DomHandle::new(11)
    );

    let failed_url =
        Url::parse("https://child-module-graph.test/already-failed.js").expect("root url");
    let failed_key = ModuleMapKey::java_script(failed_url.clone());
    let failed_reservation = store.reserve_parser_root_module_client(
        owner,
        realm_id,
        failed_key.clone(),
        parser_root_client(12, &failed_url),
    );
    assert!(matches!(
        failed_reservation.fetch_disposition(),
        FrameDocumentModuleFetchDisposition::StartedFetch(_)
    ));
    let first_failed_tasks = store.finish_parser_root_module_fetch(
        owner,
        realm_id,
        failed_key.clone(),
        Err("network failed".to_owned()),
    );
    let first_failed_task = first_failed_tasks
        .into_iter()
        .next()
        .expect("initial failed terminal task");
    let mut first_failed_works = into_parser_root_terminal_works(first_failed_task);
    assert_eq!(first_failed_works.len(), 1);
    let first_failed_work = first_failed_works
        .pop()
        .expect("initial failed parser root terminal work");
    assert_eq!(
        first_failed_work.parser_root_payload().script_handle(),
        DomHandle::new(12)
    );
    assert!(matches!(
        first_failed_work.result(),
        FrameDocumentParserRootTerminalResult::Failed(_)
    ));

    let already_failed_reservation = store.reserve_parser_root_module_client(
        owner,
        realm_id,
        failed_key.clone(),
        parser_root_client(13, &failed_url),
    );
    assert!(matches!(
        already_failed_reservation.fetch_disposition(),
        FrameDocumentModuleFetchDisposition::AlreadyFailed(_)
    ));
    let tasks = store.take_ready_document_modulator_terminal_batches(owner, realm_id);
    assert_eq!(tasks.len(), 1);
    let task = tasks.into_iter().next().expect("synthetic failed task");
    let mut works = into_parser_root_terminal_works(task);
    assert_eq!(works.len(), 1);
    let work = works
        .pop()
        .expect("synthetic failed parser root terminal work");
    assert!(matches!(
        work.result(),
        FrameDocumentParserRootTerminalResult::Failed(_)
    ));
    assert_eq!(
        work.parser_root_payload().script_handle(),
        DomHandle::new(13)
    );
}
