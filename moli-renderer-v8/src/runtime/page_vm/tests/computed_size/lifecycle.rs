//! Moli's demand-driven boundary, not Chromium's synchronous layout behavior.
//! Mutation updates DOM/Stylo; CSSOM consumes an existing sample or falls back
//! to style-only values. Only explicit visual demands may publish new geometry.

use super::*;

fn set_viewport(page: &mut PageVm, width: u32, height: u32) -> anyhow::Result<()> {
    page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
        inner_width: width,
        inner_height: height,
        outer_width: width,
        outer_height: height,
        device_pixel_ratio: 1.0,
        screen_width: width,
        screen_height: height,
        screen_avail_width: width,
        screen_avail_height: height,
    }))?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_display_transitions_follow_the_sampled_box() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<div style="width:160px"><div id=target><div style="width:75px;height:25px"></div></div></div>"#,
        )?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["160px","25px"]}));
        for (style, before_refresh, after_refresh) in [
            ("display:inline;width:200px;height:80px", ["160px","25px"], ["200px","80px"]),
            ("display:block", ["auto","auto"], ["160px","25px"]),
            ("display:none;width:200px;height:80px", ["160px","25px"], ["200px","80px"]),
            ("display:contents;width:40%;height:20%", ["40%","20%"], ["40%","20%"]),
            ("display:block", ["auto","auto"], ["160px","25px"]),
        ] {
            page.vm_mut().eval(&format!(
                "document.getElementById('target').style.cssText={};'changed'",
                serde_json::to_string(style)?,
            ))?;
            assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":before_refresh}),
                "before refresh: {style}");
            publish_size_layout(&mut page)?;
            assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":after_refresh}),
                "after refresh: {style}");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("box applicability must be sampled along with its geometry");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_viewport_change_updates_style_but_not_sampled_geometry() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<style>#target{width:20vw;height:10vh;color:red}
@media(min-width:500px){#target{color:green}}</style><div id=target></div>"#,
        )?;
        set_viewport(&mut page, 320, 240)?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["64px","24px"]}));
        let passes = page.vm().layout_pass_observability_for_test().1;
        set_viewport(&mut page, 640, 480)?;
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(read_without_layout(&mut page,
            "(() => {const s=getComputedStyle(document.getElementById('target'));return [s.width,s.height,s.color]})()")?,
            json!(["64px","24px","rgb(0, 128, 0)"]),
            "media style is live while ordinary geometry still belongs to the old viewport");
        publish_size_layout_at(&mut page, 640, 480)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["128px","48px"]}));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("viewport changes must not turn CSSOM into a visual demand");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_document_replacement_discards_the_old_snapshot() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"<div id=target style="width:100px;height:40px"></div>"#)?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["100px","40px"]}));
        let passes = page.vm().layout_pass_observability_for_test().1;
        let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
        page.vm_mut().eval(r#"
document.open();
document.write('<!doctype html><html><head><style>html,body{margin:0}</style></head><body><div id=target></div></body></html>');
document.close();
'replaced'
"#)?;
        let cache = page.vm().layout_snapshot_cache_observability_for_test();
        assert!(cache.3.is_none(), "replacement must retire the old Document's geometry");
        assert_eq!(cache.2, publishes);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["auto","auto"]}),
            "reusing an HTML id must not reuse the previous Document's dimensions");
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["320px","0px"]}));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("a replacement Document must begin without a sampled box");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_shadow_and_slotted_nodes_share_only_their_document_sample() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<div id=host style="width:160px"><div id=slotted style="width:50%;height:20px"></div></div>"#,
        )?;
        page.vm_mut().eval(r#"
const root=document.getElementById('host').attachShadow({mode:'open'});
root.innerHTML='<div id=wrapper><slot></slot><div id=target></div></div>';
globalThis.sheet=new CSSStyleSheet();
sheet.replaceSync('#wrapper{width:120px}#target{width:50%;height:30px;color:red}');
root.adoptedStyleSheets=[sheet];
globalThis.shadowStyle=getComputedStyle(root.getElementById('target'));
globalThis.slottedStyle=getComputedStyle(document.getElementById('slotted'));
'installed'
"#)?;
        publish_size_layout(&mut page)?;
        let read = "[shadowStyle.width,shadowStyle.height,slottedStyle.width,slottedStyle.height,shadowStyle.color]";
        assert_eq!(read_without_layout(&mut page, read)?, json!(["60px","30px","60px","20px","rgb(255, 0, 0)"]));
        page.vm_mut().eval(
            "sheet.replaceSync('#wrapper{width:200px}#target{width:75%;height:45px;color:blue}');'changed'"
        )?;
        assert_eq!(read_without_layout(&mut page, read)?, json!(["60px","30px","60px","20px","rgb(0, 0, 255)"]));
        publish_size_layout(&mut page)?;
        assert_eq!(read_without_layout(&mut page, read)?, json!(["150px","45px","100px","20px","rgb(0, 0, 255)"]));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flat-tree layout and live shadow style must remain separate");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_reads_do_not_invalidate_screencast_tokens() {
    use crate::runtime::{
        RendererCaptureScreencastFrameReply as Reply, RendererCaptureScreencastFrameRequest,
        RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest, RendererScreenshotFormat,
    };
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"<div id=target style="height:10px;background:red"></div>"#)?;
        set_viewport(&mut page, 40, 30)?;
        let request = |known_visual_state| RendererCaptureScreencastFrameRequest {
            format: RendererScreenshotFormat::Png,
            quality: 100,
            optimize_for_speed: true,
            max_width: None,
            max_height: None,
            known_visual_state,
        };
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["auto","10px"]}));
        let before = page.vm().layout_pass_observability_for_test().1;
        let Reply::Captured(first) = page.capture_screencast_frame(request(None))? else {
            panic!("first screencast must capture");
        };
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 1);
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["40px","10px"]}));
        assert_eq!(page.capture_screencast_frame(request(Some(first.visual_state.clone())))?, Reply::Unchanged);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 1);
        page.vm_mut().eval("document.getElementById('target').style.cssText='width:20px;height:10px;background:blue';'changed'")?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["40px","10px"]}));
        let Reply::Captured(second) = page.capture_screencast_frame(request(Some(first.visual_state)))? else {
            panic!("style mutation must capture a new frame");
        };
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 2);
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["20px","10px"]}));
        let raster = moli_image::decode_png(&second.image.bytes)?;
        assert_eq!(&raster.rgba[0..4], &[0,0,255,255], "fresh capture must paint fresh style");
        assert_eq!(page.capture_screencast_frame(request(Some(second.visual_state)))?, Reply::Unchanged);
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 2);
        assert!(matches!(page.capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())?,
            RendererCaptureScreenshotReply::Captured(_)));
        assert_eq!(page.vm().layout_pass_observability_for_test().1, before + 3,
            "screenshot is a fresh demand even when a screencast would be unchanged");
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["20px","10px"]}));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM reads must neither request frames nor dirty clean screencast tokens");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_page_snapshots_are_independent() {
    run_page_vm_async_test(async move {
        let mut first =
            page_with_size_fixture(r#"<div id=target style="width:100px;height:40px"></div>"#)?;
        let mut second =
            page_with_size_fixture(r#"<div id=target style="width:200px;height:80px"></div>"#)?;
        publish_size_layout(&mut first)?;
        publish_size_layout(&mut second)?;
        first.vm_mut().eval(
            "document.getElementById('target').style.cssText='width:300px;height:120px';'changed'",
        )?;
        assert_eq!(
            read_sizes(&mut first, &["target"])?,
            json!({"target":["100px","40px"]})
        );
        assert_eq!(
            read_sizes(&mut second, &["target"])?,
            json!({"target":["200px","80px"]})
        );
        publish_size_layout(&mut second)?;
        assert_eq!(
            read_sizes(&mut first, &["target"])?,
            json!({"target":["100px","40px"]})
        );
        let second_cache = second.vm().layout_snapshot_cache_observability_for_test();
        publish_size_layout(&mut first)?;
        assert_eq!(
            read_sizes(&mut first, &["target"])?,
            json!({"target":["300px","120px"]})
        );
        assert_eq!(
            read_sizes(&mut second, &["target"])?,
            json!({"target":["200px","80px"]})
        );
        assert_eq!(
            second.vm().layout_snapshot_cache_observability_for_test(),
            second_cache
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("publishing one Page must not replace another Page's latest sample");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_pseudo_declarations_never_borrow_the_originating_box() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<style>#target::before{content:'';display:block;width:12px;height:6px}
#target::after{content:'';display:block;width:20px;height:8px}</style>
<div id=target style="width:120px;height:40px"></div>"#,
        )?;
        page.vm_mut().eval(
            "globalThis.target=document.getElementById('target');\
             globalThis.before=getComputedStyle(target,'::before');\
             globalThis.after=getComputedStyle(target,'::after');'held'",
        )?;
        let read = "[before.width,before.height,after.width,after.height]";
        assert_eq!(
            read_without_layout(&mut page, read)?,
            json!(["12px", "6px", "20px", "8px"])
        );
        publish_size_layout(&mut page)?;
        assert_eq!(
            read_sizes(&mut page, &["target"])?,
            json!({"target":["120px","40px"]})
        );
        assert_eq!(
            read_without_layout(&mut page, read)?,
            json!(["12px", "6px", "20px", "8px"]),
            "the element's principal box is not the pseudo-element's sizing box"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("pseudo declarations must not accidentally consume their origin's geometry");
}
