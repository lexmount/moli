use super::*;

fn start_adapter_reply_file_input(
    ctx: &TestContext,
    id: u64,
    params: Value,
) -> (CdpCommandTaskStep, crate::conn::CommandDispatchContext) {
    let command = crate::conn::ParsedCdpCommand::from_serializable(json!({
        "id":id,"method":"DOM.setFileInputFiles","params":params
    }))
    .unwrap();
    let mut context = crate::conn::CommandDispatchContext::default();
    context.set_terminal_response_delivery_override(
        moli_page_types::RendererInspectorResponseDelivery::AdapterReply,
    );
    let step = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&command, &mut context)
    });
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(pending) if pending.kind_name() == "Workflow"),
        "AdapterReply file admission must leave the protocol turn before filesystem work"
    );
    (step, context)
}

async fn finish_adapter_reply_file_input(
    ctx: &mut TestContext,
    (step, mut context): (CdpCommandTaskStep, crate::conn::CommandDispatchContext),
) -> Vec<Value> {
    ctx.complete_command_task_step_with_context_for_test(step, &mut context)
        .await
        .0
}

#[tokio::test(flavor = "multi_thread")]
async fn directory_upload_crosses_cdp_filelist_clone_and_multipart_boundaries() {
    let scratch = std::env::temp_dir().join(format!(
        "moli-cdp-directory-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let directory = scratch.join("chosen");
    std::fs::create_dir_all(directory.join("nested")).unwrap();
    std::fs::write(directory.join("a.txt"), b"alpha").unwrap();
    std::fs::write(directory.join("nested/b.txt"), b"beta").unwrap();

    for adapter_reply in [false, true] {
        for reference in ["nodeId", "backendNodeId", "objectId"] {
            let mut ctx = TestContext::new();
            load_bc(&mut ctx, "BID-directory");
            navigate_to_data_html_async(
            &mut ctx,
            31000,
            "<!doctype html><form id=form><input id=upload name=files type=file webkitdirectory></form>",
        )
        .await;
            ctx.process_async(json!({"id":31001,"method":"DOM.getDocument"}))
                .await;
            let root = take_response_by_id(&mut ctx, 31001)["result"]["root"]["nodeId"].clone();
            ctx.process_async(json!({"id":31002,"method":"DOM.querySelector","params":{"nodeId":root,"selector":"#upload"}})).await;
            let node = take_response_by_id(&mut ctx, 31002)["result"]["nodeId"].clone();
            let identifier = match reference {
                "nodeId" => node,
                "backendNodeId" => {
                    ctx.process_async(
                        json!({"id":31003,"method":"DOM.describeNode","params":{"nodeId":node}}),
                    )
                    .await;
                    take_response_by_id(&mut ctx, 31003)["result"]["node"]["backendNodeId"].clone()
                }
                "objectId" => {
                    ctx.process_async(json!({"id":31004,"method":"Runtime.evaluate","params":{"expression":"document.getElementById('upload')"}})).await;
                    take_response_by_id(&mut ctx, 31004)["result"]["result"]["objectId"].clone()
                }
                _ => unreachable!(),
            };
            assert!(!identifier.is_null(), "{reference} must resolve");
            let mut params = json!({"files":[directory.to_string_lossy()]});
            params[reference] = identifier;
            if adapter_reply {
                let pending = start_adapter_reply_file_input(&ctx, 31005, params);
                let messages = finish_adapter_reply_file_input(&mut ctx, pending).await;
                ctx.sent.extend(messages);
            } else {
                ctx.process_and_wait_for_response_async(
                    json!({"id":31005,"method":"DOM.setFileInputFiles","params":params}),
                )
                .await;
            }
            assert_eq!(
                take_response_by_id(&mut ctx, 31005)["result"],
                json!({}),
                "{reference}"
            );
            ctx.process_and_wait_for_response_async(json!({
                "id":31006,"method":"Runtime.evaluate","params":{
                    "expression":r#"(async () => {
                    const input = document.getElementById('upload');
                    const list = input.files;
                    const files = [...list];
                    const transfer = new DataTransfer();
                    transfer.items.add(files[0]);
                    const form = new FormData(document.getElementById('form'));
                    form.append('renamed', files[0], 'explicit.txt');
                    const body = await new Response(form).text();
                    return {
                        paths: files.map(file => file.webkitRelativePath),
                        contents: await Promise.all(files.map(file => file.text())),
                        clone: structuredClone(files[0]).webkitRelativePath,
                        transfer: transfer.files[0].webkitRelativePath,
                        cached: input.files === list,
                        defaultFilename: body.includes('filename="chosen/nested/b.txt"'),
                        explicitFilename: body.includes('filename="explicit.txt"')
                    };
                })()"#,
                    "awaitPromise":true,"returnByValue":true
                }
            }))
            .await;
            let reply = take_response_by_id(&mut ctx, 31006);
            assert_eq!(
                reply["result"]["result"]["value"],
                json!({
                    "paths":["chosen/a.txt","chosen/nested/b.txt"],
                    "contents":["alpha","beta"],
                    "clone":"chosen/a.txt", "transfer":"chosen/a.txt", "cached":true,
                    "defaultFilename":true,"explicitFilename":true
                }),
                "{reference}: {reply}"
            );
        }
    }
    std::fs::remove_dir_all(scratch).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn lazy_uploaded_files_keep_metadata_and_reject_changed_or_missing_contents() {
    for missing in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "moli-lazy-upload-{}-{}-{missing}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"alpha").unwrap();
        let mut ctx = TestContext::new();
        load_bc(&mut ctx, "BID-lazy-upload");
        navigate_to_data_html_async(
            &mut ctx,
            31100,
            "<!doctype html><input id=upload type=file><input id=destination type=file>",
        )
        .await;
        ctx.process_async(json!({"id":31101,"method":"Runtime.evaluate","params":{"expression":"document.getElementById('upload')"}})).await;
        let object = take_response_by_id(&mut ctx, 31101)["result"]["result"]["objectId"].clone();
        ctx.process_and_wait_for_response_async(json!({"id":31102,"method":"DOM.setFileInputFiles","params":{"objectId":object,"files":[path.to_string_lossy()]}})).await;
        assert_eq!(take_response_by_id(&mut ctx, 31102)["result"], json!({}));
        if missing {
            std::fs::remove_file(&path).unwrap();
        } else {
            std::fs::write(&path, b"changed").unwrap();
        }
        ctx.process_and_wait_for_response_async(
            json!({"id":31103,"method":"Runtime.evaluate","params":{
                "expression":r#"(async () => {
                const file = document.getElementById('upload').files[0];
                const copy = structuredClone(file);
                const transfer = new DataTransfer(); transfer.items.add(copy);
                document.getElementById('destination').files = transfer.files;
                const selected = document.getElementById('destination').files[0];
                const errors = [];
                for (const blob of [file, copy, selected, file.slice(1, 3)]) {
                    try { await blob.text(); errors.push('unexpected success'); }
                    catch (error) { errors.push(error.name); }
                }
                const reader = await new Promise(resolve => {
                    const reader = new FileReader();
                    reader.onerror = () => resolve(reader.error.name);
                    reader.onload = () => resolve('unexpected success');
                    reader.readAsArrayBuffer(file);
                });
                return {size:file.size, copiedSize:selected.size, errors, reader};
            })()"#,
                "awaitPromise":true,"returnByValue":true
            }}),
        )
        .await;
        let reply = take_response_by_id(&mut ctx, 31103);
        let error = if missing {
            "NotFoundError"
        } else {
            "NotReadableError"
        };
        assert_eq!(
            reply["result"]["result"]["value"],
            json!({"size":5,"copiedSize":5,"errors":[error,error,error,error],"reader":error}),
            "{reply}"
        );
        if !missing {
            std::fs::remove_file(&path).unwrap();
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn directory_upload_symlink_replacements_reject_blob_and_file_reader_reads() {
    use std::os::unix::fs::symlink;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    for replace_parent in [false, true] {
        let scratch = std::env::temp_dir().join(format!(
            "moli-cdp-directory-symlinks-{}-{}-{replace_parent}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let root = scratch.join("chosen");
        let original = root.join("nested/file.txt");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::write(&original, b"public").unwrap();
        let outside = scratch.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("file.txt");
        fs::write(&secret, b"SECRET").unwrap();
        fs::File::open(&secret)
            .unwrap()
            .set_modified(fs::metadata(&original).unwrap().modified().unwrap())
            .unwrap();
        assert_eq!(
            fs::metadata(&original).unwrap().len(),
            fs::metadata(&secret).unwrap().len()
        );
        assert_eq!(
            fs::metadata(&original).unwrap().modified().unwrap(),
            fs::metadata(&secret).unwrap().modified().unwrap()
        );

        let mut ctx = TestContext::new();
        load_bc(&mut ctx, "BID-directory-symlinks");
        navigate_to_data_html_async(
            &mut ctx,
            31200,
            "<!doctype html><input id=upload type=file webkitdirectory>",
        )
        .await;
        ctx.process_async(json!({"id":31201,"method":"Runtime.evaluate","params":{"expression":"document.getElementById('upload')"}})).await;
        let object = take_response_by_id(&mut ctx, 31201)["result"]["result"]["objectId"].clone();
        ctx.process_and_wait_for_response_async(json!({"id":31202,"method":"DOM.setFileInputFiles","params":{"objectId":object,"files":[root.to_string_lossy()]}})).await;
        assert_eq!(take_response_by_id(&mut ctx, 31202)["result"], json!({}));
        // Freeze the exposed File/clone before swapping the filesystem entry.
        ctx.process_async(json!({"id":31203,"method":"Runtime.evaluate","params":{"expression":"globalThis.file = document.getElementById('upload').files[0];globalThis.copy = structuredClone(file);"}})).await;
        take_response_by_id(&mut ctx, 31203);
        if replace_parent {
            fs::rename(root.join("nested"), scratch.join("retained")).unwrap();
            symlink(&outside, root.join("nested")).unwrap();
        } else {
            fs::remove_file(&original).unwrap();
            symlink(&secret, &original).unwrap();
        }
        ctx.process_and_wait_for_response_async(json!({
            "id":31204,"method":"Runtime.evaluate","params":{
                "expression":r#"(async () => {
                    const errors = [];
                    for (const blob of [file, copy, file.slice(1, 3)]) {
                        try { await blob.text(); errors.push('unexpected success'); }
                        catch (error) { errors.push(error.name); }
                    }
                    const reader = await new Promise(resolve => {
                        const reader = new FileReader();
                        reader.onerror = () => resolve(reader.error.name);
                        reader.onload = () => resolve('unexpected success');
                        reader.readAsArrayBuffer(file);
                    });
                    let stream;
                    try { await file.stream().getReader().read(); stream = 'unexpected success'; }
                    catch (error) { stream = error.name; }
                    return { size:file.size, relative:file.webkitRelativePath, errors, reader, stream };
                })()"#,
                "awaitPromise":true,"returnByValue":true
            }
        })).await;
        let reply = take_response_by_id(&mut ctx, 31204);
        assert_eq!(
            reply["result"]["result"]["value"],
            json!({
                "size":6,"relative":"chosen/nested/file.txt",
                "errors":["NotReadableError","NotReadableError","NotReadableError"],
                "reader":"NotReadableError","stream":"NotReadableError"
            }),
            "parent={replace_parent}: {reply}"
        );
        fs::remove_dir_all(scratch).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn adapter_reply_file_input_preparation_keeps_its_original_page() {
    let mut ctx = TestContext::new();
    load_bc(&mut ctx, "BID-adapter_reply-files");
    navigate_to_data_html_async(
        &mut ctx,
        31500,
        "<!doctype html><input type=file id=upload>",
    )
    .await;
    ctx.process_async(json!({"id":31501,"method":"DOM.getDocument"}))
        .await;
    let root = take_response_by_id(&mut ctx, 31501)["result"]["root"]["nodeId"].clone();
    ctx.process_async(json!({"id":31502,"method":"DOM.querySelector","params":{"nodeId":root,"selector":"#upload"}})).await;
    let node = take_response_by_id(&mut ctx, 31502)["result"]["nodeId"].clone();
    let pending = start_adapter_reply_file_input(&ctx, 31503, json!({"nodeId":node,"files":[]}));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><input type=file id=upload>",
        None,
    )
    .await;
    let messages = finish_adapter_reply_file_input(&mut ctx, pending).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == 31503)
        .expect("old command must settle");
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(response["error"]["message"], "NoDocumentLoaded");
    ctx.process_async(json!({"id":31504,"method":"Runtime.evaluate","params":{"expression":"document.getElementById('upload').files.length","returnByValue":true}})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 31504)["result"]["result"]["value"],
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn adapter_reply_file_input_checks_node_before_reporting_bad_paths() {
    for field in ["nodeId", "backendNodeId"] {
        let mut ctx = TestContext::new();
        load_bc(&mut ctx, "BID-adapter_reply-file-preflight");
        navigate_to_data_html_async(&mut ctx, 31600, "<!doctype html><input type=file>").await;
        let mut params = json!({"files":["/moli-nonexistent-directory/upload.txt"]});
        params[field] = json!(1);
        let pending = start_adapter_reply_file_input(&ctx, 31601, params);
        let messages = finish_adapter_reply_file_input(&mut ctx, pending).await;
        let response = messages
            .iter()
            .find(|message| message["id"] == 31601)
            .expect("preflight response");
        assert_eq!(response["error"]["code"], -32000);
        assert_eq!(
            response["error"]["message"],
            "Could not find node with given id"
        );
    }
}
