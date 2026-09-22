use super::*;
use base64::Engine as _;

async fn command(
    ctx: &mut TestContext,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    ctx.process_async(
        json!({"id": 88801, "sessionId": "SID-1", "method": method, "params": params}),
    )
    .await;
    let response = ctx.take_response_by_id(88801);
    assert!(response["error"].is_null(), "{method}: {response}");
    response["result"].clone()
}

async fn screenshot(ctx: &mut TestContext) -> moli_image::RgbaImage {
    let result = command(ctx, "Page.captureScreenshot", json!({"format":"png"})).await;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(result["data"].as_str().unwrap())
        .unwrap();
    moli_image::decode_png(&bytes).unwrap()
}
fn pixel(image: &moli_image::RgbaImage, x: usize, y: usize) -> &[u8] {
    let offset = (y * image.width as usize + x) * 4;
    &image.rgba[offset..offset + 4]
}
async fn page() -> TestContext {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    command(
        &mut ctx,
        "Emulation.setDeviceMetricsOverride",
        json!({"width":100,"height":100,"deviceScaleFactor":1,"mobile":false}),
    )
    .await;
    evaluate(
        &mut ctx,
        "document.body.style='margin:0;background:white';undefined",
    )
    .await;
    ctx
}

#[tokio::test(flavor = "multi_thread")]
async fn os_text_scale_updates_css_environment_and_preserves_media_preferences() {
    let mut ctx = page().await;
    evaluate(&mut ctx,r#"document.head.innerHTML='<style>#scaled {font-size:calc(16px * env(preferred-text-scale,1));width:calc(20px * env(preferred-text-scale,1))}#plain{font-size:16px}</style>';document.body.innerHTML='<div id="scaled">A</div><div id="plain">A</div>';undefined"#).await;
    for (scale, expected) in [
        (json!({"scale":1.5}), json!(["24px", "30px", "16px"])),
        (json!({"scale":2}), json!(["32px", "40px", "16px"])),
        (json!({}), json!(["16px", "20px", "16px"])),
    ] {
        command(&mut ctx, "Emulation.setEmulatedOSTextScale", scale).await;
        command(
            &mut ctx,
            "Emulation.setEmulatedMedia",
            json!({"features":[{"name":"prefers-color-scheme","value":"dark"}]}),
        )
        .await;
        assert_eq!(evaluate(&mut ctx,"[getComputedStyle(scaled).fontSize,getComputedStyle(scaled).width,getComputedStyle(plain).fontSize]").await,expected);
        assert_eq!(
            evaluate(
                &mut ctx,
                "matchMedia('(prefers-color-scheme:dark)').matches"
            )
            .await,
            json!(true)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn vision_deficiency_changes_capture_pixels_without_mutating_dom() {
    let mut ctx = page().await;
    evaluate(&mut ctx, "document.body.style.background='red';undefined").await;
    let original = screenshot(&mut ctx).await;
    assert_eq!(pixel(&original, 50, 50), [255, 0, 0, 255]);
    command(
        &mut ctx,
        "Emulation.setEmulatedVisionDeficiency",
        json!({"type":"achromatopsia"}),
    )
    .await;
    let gray = screenshot(&mut ctx).await;
    let channels = pixel(&gray, 50, 50);
    assert_eq!(channels[0], channels[1]);
    assert_eq!(channels[1], channels[2]);
    assert!((126..=128).contains(&channels[0]));
    assert_eq!(
        evaluate(&mut ctx, "getComputedStyle(document.body).backgroundColor").await,
        json!("rgb(255, 0, 0)")
    );
    command(
        &mut ctx,
        "Emulation.setEmulatedVisionDeficiency",
        json!({"type":"none"}),
    )
    .await;
    assert_eq!(screenshot(&mut ctx).await.rgba, original.rgba);
}

#[tokio::test(flavor = "multi_thread")]
async fn inspector_highlight_rect_is_visible_and_hide_restores_original_pixels() {
    let mut ctx = page().await;
    let original = screenshot(&mut ctx).await;
    command(&mut ctx, "DOM.enable", json!({})).await;
    command(&mut ctx, "Overlay.enable", json!({})).await;
    command(
        &mut ctx,
        "DOM.highlightRect",
        json!({"x":10,"y":10,"width":30,"height":30,"color":{"r":255,"g":0,"b":0,"a":1}}),
    )
    .await;
    command(
        &mut ctx,
        "Emulation.setEmulatedVisionDeficiency",
        json!({"type":"achromatopsia"}),
    )
    .await;
    let highlighted = screenshot(&mut ctx).await;
    assert_eq!(pixel(&highlighted, 20, 20), [255, 0, 0, 255]);
    assert_eq!(pixel(&highlighted, 60, 60), pixel(&original, 60, 60));
    assert_eq!(
        evaluate(&mut ctx, "document.body.children.length").await,
        json!(0)
    );
    command(&mut ctx, "DOM.hideHighlight", json!({})).await;
    assert_eq!(screenshot(&mut ctx).await.rgba, original.rgba);
}

#[tokio::test(flavor = "multi_thread")]
async fn inspector_highlight_tracks_live_node_geometry_and_rejects_stale_ids() {
    let mut ctx = page().await;
    evaluate(&mut ctx,"document.body.innerHTML='<div id=target style=\"position:absolute;left:10px;top:10px;width:20px;height:20px\"></div>';undefined").await;
    let doc = command(&mut ctx, "DOM.getDocument", json!({})).await;
    let node = command(
        &mut ctx,
        "DOM.querySelector",
        json!({"nodeId":doc["root"]["nodeId"],"selector":"#target"}),
    )
    .await;
    command(&mut ctx,"Overlay.highlightNode",json!({"nodeId":node["nodeId"],"highlightConfig":{"contentColor":{"r":0,"g":255,"b":0,"a":1}}})).await;
    assert_eq!(pixel(&screenshot(&mut ctx).await, 20, 20), [0, 255, 0, 255]);
    evaluate(&mut ctx, "target.style.left='60px';undefined").await;
    let moved = screenshot(&mut ctx).await;
    assert_eq!(pixel(&moved, 70, 20), [0, 255, 0, 255]);
    assert_eq!(pixel(&moved, 20, 20), [255, 255, 255, 255]);
    command(&mut ctx, "Overlay.disable", json!({})).await;
    assert_eq!(
        pixel(&screenshot(&mut ctx).await, 70, 20),
        [255, 255, 255, 255]
    );
    ctx.process_async(json!({"id":88802,"sessionId":"SID-1","method":"DOM.highlightNode","params":{"nodeId":2147483647,"highlightConfig":{}}})).await;
    assert!(ctx.take_response_by_id(88802)["error"].is_object());
}

#[tokio::test(flavor = "multi_thread")]
async fn scroll_gesture_uses_hit_target_and_honors_wheel_cancellation() {
    let mut ctx = page().await;
    evaluate(&mut ctx,r#"document.body.innerHTML='<div id="scroller" style="width:80px;height:80px;overflow:scroll"><div style="width:500px;height:500px"></div></div>';window.events=[];scroller.addEventListener('wheel',e=>events.push([e.deltaX,e.deltaY,e.isTrusted]));undefined"#).await;
    command(
        &mut ctx,
        "Input.synthesizeScrollGesture",
        json!({"x":20,"y":20,"xDistance":-30,"yDistance":-60,"gestureSourceType":"mouse"}),
    )
    .await;
    assert_eq!(
        evaluate(
            &mut ctx,
            "[scroller.scrollLeft,scroller.scrollTop,scrollY,events.reduce((sum,e)=>sum+e[0],0),events.reduce((sum,e)=>sum+e[1],0),events.every(e=>e[2])]"
        )
        .await,
        json!([30, 60, 0, 30, 60, true])
    );
    evaluate(
        &mut ctx,
        "scroller.addEventListener('wheel',e=>e.preventDefault(),{passive:false});undefined",
    )
    .await;
    command(
        &mut ctx,
        "Input.synthesizeScrollGesture",
        json!({"x":20,"y":20,"yDistance":-60}),
    )
    .await;
    assert_eq!(evaluate(&mut ctx, "scroller.scrollTop").await, json!(60));
}

#[tokio::test(flavor = "multi_thread")]
async fn blurred_vision_composites_transparent_filter_edges_over_browser_background() {
    let mut ctx = page().await;
    evaluate(&mut ctx, "document.body.style.background='red';undefined").await;
    command(
        &mut ctx,
        "Emulation.setEmulatedVisionDeficiency",
        json!({"type":"blurredVision"}),
    )
    .await;
    let blurred = screenshot(&mut ctx).await;
    assert_eq!(pixel(&blurred, 50, 50), [255, 0, 0, 255]);
    let edge = pixel(&blurred, 0, 50);
    assert_eq!(edge[0], 255);
    assert_eq!(edge[3], 255);
    assert!((90..=115).contains(&edge[1]), "{edge:?}");
    assert_eq!(edge[1], edge[2]);
}
