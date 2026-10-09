use super::*;

#[tokio::test]
async fn worker_offscreen_canvas_blob_encodes_real_pixels_through_worker_tasks() {
    ensure_v8();
    let source = format!(
        r#"const checkExports = {};
            (async () => {{
                const result = await checkExports(globalThis, globalThis, 'worker');
                postMessage({{
                    total: result.checks.length,
                    failures: result.checks.filter(check => !check.passed),
                    snapshot: Array.from(new Uint8Array(await result.snapshot.arrayBuffer())),
                    converted: Array.from(new Uint8Array(await result.converted.arrayBuffer())),
                }});
                close();
            }})().catch(error => {{postMessage({{error: String(error)}}); close();}});"#,
        include_str!("../../../../tests/fixtures/canvas-offscreen-blob-checks.js"),
    );
    let mut worker = spawn_worker(source, "test://offscreen_canvas_blob".into());
    let result: serde_json::Value =
        serde_json::from_str(&recv_post_json(&mut worker).await).unwrap();
    assert_eq!(result["total"], 95, "{result}");
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    for (name, pixel) in [
        ("snapshot", [0, 255, 0, 255]),
        ("converted", [0, 0, 255, 255]),
    ] {
        let bytes = result[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|byte| byte.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        let image = moli_image::decode_png(&bytes).expect("worker export must be a valid PNG");
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.rgba, pixel.repeat(2), "{name}");
    }
}
