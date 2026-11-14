use super::*;

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
        ctx.process_and_wait_for_response_async(
            json!({"id":31005,"method":"DOM.setFileInputFiles","params":params}),
        )
        .await;
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
