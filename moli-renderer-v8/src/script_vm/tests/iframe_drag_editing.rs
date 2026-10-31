use super::*;
use crate::runtime::{
    RendererDragData, RendererDragDataItem, RendererDraggedDirectory, RendererDraggedFile,
};

#[tokio::test]
async fn iframe_editing_uses_its_selection_and_drop_file_entries_use_its_realm() {
    let mut vm = new_rendered_test_vm(
        "https://iframe-editing.test/",
        "<!doctype html><body style='margin:0'><p id=parent>parent</p>",
    );
    vm.eval(r#"
getSelection().selectAllChildren(document.getElementById('parent'));
const frame=document.createElement('iframe');frame.id='frame';
frame.style='position:absolute;left:100px;top:100px;width:300px;height:300px;border:0';
frame.srcdoc=`<!doctype html><body style='margin:0'>
<div id=editor contenteditable style='height:80px'>😀abc</div>
<div id=target contenteditable style='height:80px'></div>
<div id=files style='height:80px'></div><script>
const range=document.createRange();range.setStart(editor.firstChild,2);range.setEnd(editor.firstChild,3);
getSelection().removeAllRanges();getSelection().addRange(range);
window.__editing=[];window.__entries=[];
for(const type of ['beforeinput','input'])document.addEventListener(type,e=>{
  __editing.push([e.target.id,e.inputType,e instanceof InputEvent,
    e.dataTransfer===null || e.dataTransfer instanceof DataTransfer]);
});
files.ondragover=e=>e.preventDefault();
files.ondrop=e=>{
  e.preventDefault();
  for(const item of e.dataTransfer.items){
    const entry=item.webkitGetAsEntry();
    __entries.push([entry instanceof FileSystemEntry,
      entry instanceof (entry.isFile?FileSystemFileEntry:FileSystemDirectoryEntry),
      entry.filesystem instanceof FileSystem,entry.fullPath,
      entry.isFile ? item.getAsFile() instanceof File : entry.createReader() instanceof FileSystemDirectoryReader]);
  }
};
</`+`script>`;document.body.append(frame);
"#).unwrap();
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "commit editing frame",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "install editing frame script",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(&mut vm, "editing frame ready").await;
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let script = crate::util::v8str(scope, "frame.contentDocument.getElementById('editor')");
        let script = v8::Script::compile(scope, script, None).unwrap();
        let object = crate::script_execution::execute_compiled_script(scope, script).unwrap();
        let object = v8::Local::<v8::Object>::try_from(object).unwrap();
        let handle =
            crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)?.1;
        assert!(
            crate::native_bridge::element::replace_contenteditable_selection(
                scope,
                host_ptr,
                handle,
                "X",
                crate::native_bridge::element::TextEditInputType::InsertText
            )
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("JSON.stringify([frame.contentDocument.getElementById('editor').textContent,getSelection().toString()])").unwrap(),r#"["😀Xbc","parent"]"#);
    vm.publish_layout_for_test().unwrap();
    let mut data = RendererDragData {
        items: vec![RendererDragDataItem {
            mime_type: "text/plain".to_owned(),
            data: "drop".to_owned(),
            title: None,
            base_url: None,
        }],
        files: vec![],
        directories: vec![],
        drag_operations_mask: 1,
    };
    for event in ["dragenter", "dragover", "drop"] {
        vm.dispatch_drag_event_at_point(120.0, 200.0, event, data.clone(), 0)
            .unwrap();
    }
    assert_eq!(
        vm.eval("frame.contentDocument.getElementById('target').textContent")
            .unwrap(),
        "drop"
    );
    assert_eq!(
        vm.eval("JSON.stringify(frame.contentWindow.__editing)")
            .unwrap(),
        r#"[["editor","insertText",true,true],["editor","insertText",true,true],["target","insertFromDrop",true,true],["target","insertFromDrop",true,true]]"#
    );
    data.items.clear();
    data.files.push(RendererDraggedFile {
        name: "one.txt".to_owned(),
        mime_type: "text/plain".to_owned(),
        bytes: b"one".to_vec(),
        last_modified: 1.0,
    });
    data.directories.push(RendererDraggedDirectory {
        name: "folder".to_owned(),
        files: vec![],
        directories: vec![],
    });
    for event in ["dragenter", "dragover", "drop"] {
        vm.dispatch_drag_event_at_point(120.0, 280.0, event, data.clone(), 0)
            .unwrap();
    }
    assert_eq!(
        vm.eval("JSON.stringify(frame.contentWindow.__entries)")
            .unwrap(),
        r#"[[true,true,true,"/one.txt",true],[true,true,true,"/folder",true]]"#
    );
}
