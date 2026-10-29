use super::*;
use moli_page_types::LayoutPolicy;

const INVALID_COLORS: [&str; 4] = ["rgb(1, 2, 3)"; 4];
const VALID_COLORS: [&str; 4] = ["rgb(4, 5, 6)"; 4];

fn eval_file_state(vm: &mut StandaloneScriptVmHarness, script: &str) -> serde_json::Value {
    serde_json::from_str(&vm.eval(script).unwrap()).unwrap()
}

fn file_input_validity_style_vm(policy: LayoutPolicy) -> StandaloneScriptVmHarness {
    let mut vm = new_parsed_test_vm(
        "https://file-input-state-style.test/",
        r#"<!doctype html><style>
          #upload, #uploadForm, #uploadFieldset, #fileTarget { color:rgb(1,2,3) }
          #upload:valid, #uploadForm:valid, #uploadFieldset:valid,
          body:has(#upload:valid) #fileTarget { color:rgb(4,5,6) }
        </style><form id=uploadForm><fieldset id=uploadFieldset>
          <input id=upload type=file required>
        </fieldset></form><div id=fileTarget></div>"#,
    );
    vm.set_layout_policy(policy);
    assert_eq!(
        eval_file_state(
            &mut vm,
            r#"
            // Initialize dirty value state before warming the declarations
            // or checking identical-file no-ops.
            upload.value='';
            window.fileStyles=[upload,uploadForm,uploadFieldset,fileTarget]
              .map(element=>getComputedStyle(element));
            window.fileColors=()=>fileStyles.map(style=>style.color);
            window.fileEvents=[];
            const transfer=new DataTransfer();
            transfer.items.add(new File(['abc'],'chosen.txt',{type:'text/plain',lastModified:7}));
            window.chosenFiles=transfer.files;
            window.emptyFiles=new DataTransfer().files;
            for (const type of ['input','change','cancel']) {
              upload.addEventListener(type,event=>{
                // These held style reads precede the live validity queries.
                const colors=fileColors();
                fileEvents.push([event.type,colors,upload.files.length,upload.validity.valueMissing]);
              });
            }
            JSON.stringify(fileColors())
            "#,
        ),
        serde_json::json!(INVALID_COLORS)
    );
    assert_eq!(
        eval_file_state(
            &mut vm,
            "upload.required=false;JSON.stringify(fileColors())"
        ),
        serde_json::json!(VALID_COLORS),
        "the attribute route must refresh the same held declarations: {policy:?}"
    );
    assert_eq!(
        eval_file_state(&mut vm, "upload.required=true;JSON.stringify(fileColors())"),
        serde_json::json!(INVALID_COLORS)
    );
    assert_eq!(
        eval_file_state(
            &mut vm,
            "JSON.stringify([upload.files.length,upload.value,upload.validity.valueMissing,upload.matches('#upload:valid')])",
        ),
        serde_json::json!([0, "", true, false])
    );
    vm
}

#[test]
fn input_files_web_assignment_refreshes_held_validity_style_in_the_same_call() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = file_input_validity_style_vm(policy);
        let builds = vm.layout_pass_observability_for_test().1;
        assert_eq!(
            eval_file_state(
                &mut vm,
                "upload.files=chosenFiles;JSON.stringify([fileColors(),upload.files.length,upload.validity.valueMissing,upload.files===chosenFiles,fileEvents.length])",
            ),
            serde_json::json!([VALID_COLORS, 1, false, true, 0]),
            "files= must publish styles before the same-call read: {policy:?}"
        );
        let version = vm._context_host.borrow().dom_host().dom_version();
        assert_eq!(
            eval_file_state(
                &mut vm,
                "upload.files=chosenFiles;JSON.stringify([fileColors(),fileEvents.length])",
            ),
            serde_json::json!([VALID_COLORS, 0])
        );
        assert_eq!(vm._context_host.borrow().dom_host().dom_version(), version);
        // The existing value wrapper is an independent clearing control.
        assert_eq!(
            eval_file_state(
                &mut vm,
                "upload.value='';JSON.stringify([fileColors(),upload.files.length,upload.validity.valueMissing])",
            ),
            serde_json::json!([INVALID_COLORS, 0, true])
        );
        assert_eq!(
            eval_file_state(
                &mut vm,
                "upload.files=chosenFiles;JSON.stringify(fileColors())"
            ),
            serde_json::json!(VALID_COLORS)
        );
        assert_eq!(
            eval_file_state(
                &mut vm,
                "upload.files=emptyFiles;JSON.stringify([fileColors(),upload.files.length,upload.value,upload.validity.valueMissing,upload.files===emptyFiles,fileEvents.length])",
            ),
            serde_json::json!([INVALID_COLORS, 0, "", true, true, 0]),
            "FileList clearing must also refresh before returning: {policy:?}"
        );
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "file validity colors must not require layout: {policy:?}"
        );
    }
}

#[test]
fn input_files_native_commit_refreshes_style_before_input_and_change_callbacks() {
    for policy in [LayoutPolicy::Mock, LayoutPolicy::OnDemand] {
        let mut vm = file_input_validity_style_vm(policy);
        let upload = vm.document_runtime.get_element_by_id("upload").unwrap();
        let chosen = crate::dom::native::SelectedFile {
            bytes: b"abc".to_vec(),
            mime_type: "text/plain".to_owned(),
            name: "chosen.txt".to_owned(),
            last_modified: 7.0,
        };
        let builds = vm.layout_pass_observability_for_test().1;
        // Exercise the existing shared command body. An evaluation after it
        // cannot repair primitive observations saved in input/change handlers.
        assert!(
            vm.set_file_input_files(upload, vec![chosen.clone()], false)
                .unwrap()
        );
        assert_eq!(
            eval_file_state(&mut vm, "JSON.stringify(fileEvents)"),
            serde_json::json!([
                ["input", VALID_COLORS, 1, false],
                ["change", VALID_COLORS, 1, false]
            ]),
            "the input callback is the first public style read: {policy:?}"
        );
        let version = vm._context_host.borrow().dom_host().dom_version();
        assert!(
            vm.set_file_input_files(upload, vec![chosen], false)
                .unwrap()
        );
        assert_eq!(vm._context_host.borrow().dom_host().dom_version(), version);
        assert!(vm.set_file_input_files(upload, Vec::new(), false).unwrap());
        assert_eq!(
            eval_file_state(&mut vm, "JSON.stringify(fileEvents)"),
            serde_json::json!([
                ["input", VALID_COLORS, 1, false],
                ["change", VALID_COLORS, 1, false],
                ["cancel", VALID_COLORS, 1, false],
                ["input", INVALID_COLORS, 0, true],
                ["change", INVALID_COLORS, 0, true]
            ]),
            "identical selection cancels; clearing emits current-state events: {policy:?}"
        );
        assert_eq!(
            vm.layout_pass_observability_for_test().1,
            builds,
            "native file validity callbacks must not require layout: {policy:?}"
        );
    }
}
