use super::*;

#[path = "navigation/element_navigation.rs"]
mod element_navigation;
#[path = "navigation/history_api.rs"]
mod history_api;
#[path = "navigation/history_traversal.rs"]
mod history_traversal;
mod main_beforeunload;
#[path = "navigation/navigate_event.rs"]
mod navigate_event;
#[path = "navigation/performance.rs"]
mod performance;
#[path = "navigation/same_document.rs"]
mod same_document;

fn new_unload_lifecycle_test_vm(url: &str) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm(url);
    let owner = vm.current_main_document_task_owner().unwrap();
    let interactive = vm.finish_current_main_document_parsing(owner).unwrap();
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .unwrap();
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert!(
        vm.dispatch_main_document_window_load_lifecycle(owner)
            .unwrap()
            .is_none()
    );
    vm
}
