//! Reduced cases from Chromium's third_party/blink/web_tests.
//!
//! Source paths below are relative to that directory. We port the width/height
//! assertions, not Chromium's implicit style/layout flush: each used-value
//! comparison follows an explicit Moli layout demand, and every CSSOM read
//! checks that it did not request or publish layout.

use super::*;

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_resolved_value_display_and_unit_matrix() {
    // fast/css/getComputedStyle/getComputedStyle-resolved-values.html and its
    // checked-in expected output. Omit text so platform fonts cannot affect
    // the fixture; keep the 500x300 containing block and 24px em basis.
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<div style="width:500px;height:300px"><div id=target></div></div>"#,
        )?;
        for display in ["block", "inline", "inline-block", "none"] {
            for (dimensions, computed, used) in [
                (
                    "width:150px;height:100px",
                    ["150px", "100px"],
                    ["150px", "100px"],
                ),
                ("width:50%;height:25%", ["50%", "25%"], ["250px", "75px"]),
                (
                    "width:10em;height:5em",
                    ["240px", "120px"],
                    ["240px", "120px"],
                ),
            ] {
                let style = format!(
                    "display:{display};font-size:24px;border:24px solid;padding:20px;{dimensions}"
                );
                page.vm_mut().eval(&format!(
                    "document.getElementById('target').style.cssText={};'changed'",
                    serde_json::to_string(&style)?,
                ))?;
                publish_size_layout(&mut page)?;
                let expected = if matches!(display, "inline" | "none") {
                    computed
                } else {
                    used
                };
                assert_eq!(
                    read_sizes(&mut page, &["target"])?,
                    json!({"target":expected}),
                    "{display}: {dimensions}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Chromium resolved-value display/unit cases");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_no_box_min_max_matrix() {
    // external/wpt/css/cssom/getComputedStyle-resolved-min-max-clamping.html.
    // The same held declarations must remain live both before and after a
    // snapshot, but inapplicable boxes must never acquire used-value clamping.
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<span id=inline></span><div id=none style="display:none"></div>
<div id=contents style="display:contents"></div>"#,
        )?;
        for sampled in [false, true] {
            if sampled {
                publish_size_layout(&mut page)?;
            }
            for id in ["inline", "none", "contents"] {
                page.vm_mut().eval(&format!(
                    "globalThis.target=document.getElementById('{id}');globalThis.held=getComputedStyle(target);\
                     target.style.minWidth=target.style.minHeight='10px';\
                     target.style.maxWidth=target.style.maxHeight='50px';'held'"
                ))?;
                for value in ["10%", "15px", "1px", "60px", "auto"] {
                    page.vm_mut().eval(&format!(
                        "target.style.width=target.style.height='{value}';'changed'"
                    ))?;
                    assert_eq!(
                        read_without_layout(&mut page, "[held.width,held.getPropertyValue('height')]")?,
                        json!([value, value]), "{id}, sampled={sampled}, value={value}"
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("non-replaced inline/none/contents sizes must remain unclamped");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_box_sizing_and_padding_floor() {
    // fast/css/getComputedStyle/getComputedStyle-border-box.html.
    // Extend its content/border 200px pair with zero-size floors, percentages,
    // and conflicting min/max values. All numbers are CSS sizing-box sizes,
    // not offsetWidth/Height (which include border/padding and integer rounding).
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<div style="width:200px;height:120px"><div id=target></div></div>"#,
        )?;
        for (dimensions, expected) in [
            ("width:200px;height:200px", ["200px", "200px"]),
            (
                "box-sizing:border-box;width:200px;height:200px",
                ["200px", "200px"],
            ),
            ("width:0;height:0", ["0px", "0px"]),
            ("box-sizing:border-box;width:0;height:0", ["30px", "30px"]),
            ("width:50%;height:50%", ["100px", "60px"]),
            (
                "box-sizing:border-box;width:50%;height:50%",
                ["100px", "60px"],
            ),
            (
                "width:0;height:0;min-width:100px;max-width:50px;min-height:60px;max-height:20px",
                ["100px", "60px"],
            ),
            (
                "visibility:hidden;width:122px;height:33px",
                ["122px", "33px"],
            ),
        ] {
            page.vm_mut().eval(&format!(
                "document.getElementById('target').style.cssText={};'changed'",
                serde_json::to_string(&format!("padding:10px;border:5px solid;{dimensions}"))?,
            ))?;
            publish_size_layout(&mut page)?;
            assert_eq!(
                read_sizes(&mut page, &["target"])?,
                json!({"target":expected}),
                "{dimensions}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM must select the sampled sizing box, including its minimum border size");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_table_cell_height_excludes_authored_padding() {
    // fast/css/getComputedStyle/getComputedStyle-height.html (WebKit bug 33593).
    // The original 200px content height must not become the 260px border box.
    // Also cover border-box cells and their table-generated anonymous boxes.
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<style>table{border-spacing:0}td{padding:20px;border:10px solid}</style>
<table><tr><td id=content style="width:100px;height:200px"></td></tr></table>
<table><tr><td id=border style="box-sizing:border-box;width:160px;height:260px"></td></tr></table>"#,
        )?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["content", "border"])?,
            json!({"content":["100px","200px"],"border":["160px","260px"]}));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table cells should expose the correct CSS sizing box");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_ancestor_zoom_matrix() {
    // fast/css/getComputedStyle/script-tests/computed-style-with-zoom.js,
    // restricted to width/height. Explicitly resample after every ancestor
    // zoom mutation; a stale snapshot alone would make the equality vacuous.
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<div id=content style="position:absolute;width:20px;height:20px;border:20px solid"></div>
<div id=border style="box-sizing:border-box;width:100.25px;height:80.5px;border:20px solid"></div>"#,
        )?;
        for zoom in ["1", "2", "0.5", "1.25"] {
            page.vm_mut().eval(&format!("document.body.style.zoom='{zoom}';'changed'"))?;
            publish_size_layout(&mut page)?;
            assert_eq!(read_sizes(&mut page, &["content", "border"])?,
                json!({"content":["20px","20px"],"border":["100.25px","80.5px"]}), "zoom={zoom}");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM used sizes should remove the sampled absolute zoom");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_scrollbar_roundtrip_requires_resampling() {
    // external/wpt/css/cssom/getComputedStyle-width-scroll.tentative.html and
    // its Chromium expected.txt: the tentative round-trip assertion FAILS in
    // Chromium (100 -> 85 -> 70 with classic scrollbars). Do not label stale
    // snapshot equality as WPT conformance, or change the geometry to satisfy it.
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<div id=target style="width:100px;height:100px;overflow:scroll;scrollbar-width:auto"></div>"#,
        )?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["85px","85px"]}));
        page.vm_mut().eval(
            "const target=document.getElementById('target');const held=getComputedStyle(target);\
             target.style.width=held.width;target.style.height=held.height;'copied'"
        )?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["85px","85px"]}));
        publish_size_layout(&mut page)?;
        assert_eq!(read_sizes(&mut page, &["target"])?, json!({"target":["70px","70px"]}));
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("a stale snapshot must not disguise Chromium's scrollbar sizing behavior");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_chromium_replacement_in_inline_block_split() {
    // external/wpt/css/cssom/getComputedStyle-layout-dependent-replaced-into-ib-split.html.
    // Deliberate Moli difference: replacement is not a layout demand. Its
    // percentage width can use the existing style-only fallback, but auto
    // height must stay auto until an explicit refresh creates its own box.
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(
            r#"<style>#wrapper div{width:100%}</style>
<section id=wrapper style="width:160px"><span><div id=target></div></span></section>"#,
        )?;
        page.vm_mut().eval(
            "globalThis.oldStyle=getComputedStyle(document.getElementById('target'));'held'",
        )?;
        publish_size_layout(&mut page)?;
        assert_eq!(
            read_sizes(&mut page, &["target"])?,
            json!({"target":["160px","0px"]})
        );
        page.vm_mut().eval(
            "const replacement=document.createElement('div');replacement.id='target';\
             document.getElementById('target').replaceWith(replacement);'replaced'",
        )?;
        assert_eq!(
            read_without_layout(&mut page, "[oldStyle.width,oldStyle.height]")?,
            json!(["", ""])
        );
        assert_eq!(
            read_sizes(&mut page, &["target"])?,
            json!({"target":["160px","auto"]})
        );
        publish_size_layout(&mut page)?;
        assert_eq!(
            read_sizes(&mut page, &["target"])?,
            json!({"target":["160px","0px"]})
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("a replacement in an inline/block split needs its own sampled box");
}
