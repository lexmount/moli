use super::{clean_output, subprocess::run_command_with_process_timeout};
use anyhow::{Context, Result, ensure};
use axum::{Router, http::StatusCode, response::Html, routing::get};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{process::Command, time::Duration};
use tokio::net::TcpListener;

const VISUAL_EXPORT_STYLE: &str = "<!doctype html><style>body{margin:0;background:white} \
    .tile{display:block;width:12px;height:12px}</style>";

fn run_visual_export(url: &str, dump: &str) -> Result<Vec<u8>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
    command.args([
        "fetch",
        "--http-no-proxy",
        "*",
        "--layout",
        "--image",
        "--wait",
        "load",
        "--timeout",
        "60000",
        "--http-timeout",
        "60000",
        "--dump",
        dump,
        url,
    ]);
    let output = run_command_with_process_timeout(command, Duration::from_secs(15))
        .with_context(|| format!("{dump} failed to complete for {url}"))?;
    ensure!(
        output.status.success(),
        "{dump} failed: {}",
        clean_output(&output.stderr),
    );
    Ok(output.stdout)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_visual_exports_complete_with_pending_css_images() -> Result<()> {
    let app = Router::new()
        .route(
            "/",
            get(|| async {
                Html(format!(
                    r#"{VISUAL_EXPORT_STYLE}<div class="tile" style="background:lime url('/pending.svg')"></div><img class="tile" src="/ready.svg" alt="">"#
                ))
            }),
        )
        .route(
            "/control",
            get(|| async {
                Html(format!(
                    r#"{VISUAL_EXPORT_STYLE}<div class="tile" style="background:lime"></div><div class="tile" style="background:red"></div>"#
                ))
            }),
        )
        .route(
            "/ready.svg",
            get(|| async {
                (
                    [("content-type", "image/svg+xml")],
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12"><rect width="12" height="12" fill="red"/></svg>"#,
                )
            }),
        )
        // Hold the CSS image response open throughout the exports.
        .route("/pending.svg", get(std::future::pending::<StatusCode>));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    for dump in ["screenshot", "screenshot_full"] {
        let actual = run_visual_export(&format!("{base_url}/"), dump)?;
        let control = run_visual_export(&format!("{base_url}/control"), dump)?;
        assert!(
            actual == control,
            "{dump} should paint the loaded <img> and the pending CSS image's fallback color"
        );
    }
    assert_eq!(
        lopdf::Document::load_mem(&run_visual_export(&format!("{base_url}/"), "pdf")?)?
            .get_pages()
            .len(),
        1
    );
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_visual_exports_paint_shared_ready_css_images_on_first_capture() -> Result<()> {
    let red_png = STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAwAAAAMCAIAAADZF8uwAAAAFUlEQVR4nGP4z8BAEBFWMapoABQBAPuQj3FCf3VbAAAAAElFTkSuQmCC",
    )?;
    let app = Router::new()
        .route(
            "/",
            get(|| async {
                Html(format!(
                    r#"{VISUAL_EXPORT_STYLE}<div class="tile" style="background:lime url('/ready.png')"></div><img class="tile" src="/ready.png" alt="">"#
                ))
            }),
        )
        .route(
            "/control",
            get(|| async {
                Html(format!(
                    r#"{VISUAL_EXPORT_STYLE}<div class="tile" style="background:red"></div><div class="tile" style="background:red"></div>"#
                ))
            }),
        )
        .route(
            "/ready.png",
            get(move || async move { ([("content-type", "image/png")], red_png) }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base_url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    for dump in ["screenshot", "screenshot_full"] {
        let actual = run_visual_export(&format!("{base_url}/"), dump)?;
        let control = run_visual_export(&format!("{base_url}/control"), dump)?;
        assert!(
            actual == control,
            "{dump} should paint the same decoded PNG as a CSS background and an <img>"
        );
    }
    let pdf = lopdf::Document::load_mem(&run_visual_export(&format!("{base_url}/"), "pdf")?)?;
    let page = pdf.get_pages()[&1];
    let resources = pdf.get_dict_in_dict(pdf.get_dictionary(page)?, b"Resources")?;
    let xobjects = pdf.get_dict_in_dict(resources, b"XObject")?;
    let content = lopdf::content::Content::decode(&pdf.get_page_content(page))?;
    let mut image_draws = 0;
    for operation in content.operations.iter().filter(|op| op.operator == "Do") {
        let name = operation.operands[0].as_name()?;
        let stream = pdf.dereference(xobjects.get(name)?)?.1.as_stream()?;
        assert_eq!(stream.dict.get(b"Subtype")?.as_name()?, b"Image");
        let pixels = stream.decompressed_content()?;
        assert!(
            !pixels.is_empty() && pixels.chunks(3).all(|pixel| pixel == [255, 0, 0]),
            "PDF image should contain red pixels"
        );
        image_draws += 1;
    }
    assert_eq!(
        image_draws, 2,
        "PDF should draw both the background and <img>"
    );
    server.abort();
    Ok(())
}

fn run_layout(html: &str, expression: &str, fonts: bool) -> Result<Value> {
    let url = format!("data:text/html;base64,{}", STANDARD.encode(html));
    let mut command = Command::new(env!("CARGO_BIN_EXE_moli"));
    command.args([
        "fetch",
        "--log-level",
        "error",
        "--http-no-proxy",
        "*",
        "--layout",
        "--timeout",
        "2000",
        "--eval",
        expression,
    ]);
    if fonts {
        command.arg("--font");
    }
    command.arg(url);
    let output = run_command_with_process_timeout(command, Duration::from_secs(10))
        .context("layout probe process failed")?;
    ensure!(
        output.status.success(),
        "layout probe exited with {}: stdout={} stderr={}",
        output.status,
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).context("layout probe must return JSON")
}

#[test]
fn extreme_svg_arc_layout_terminates_without_a_screenshot() -> Result<()> {
    // Publishing layout used to expand this one arc into millions of cubics,
    // then drain them with quadratic buffer shifts. The external process limit
    // also bounds the test if the CLI's readiness timeout cannot interrupt it.
    for doctype in ["<!doctype html>", ""] {
        let control = run_layout(
            &format!("{doctype}<svg width=300 height=300></svg>"),
            "document.body.offsetHeight",
            false,
        )?;
        let value = run_layout(
            &format!(
                "{doctype}<svg width=300 height=300><path d=\"M0 0 A 743 -417 0 1 1 7 1e38\"/></svg>"
            ),
            "({bodyHeight:document.body.offsetHeight,svgHeight:document.querySelector('svg').getBoundingClientRect().height})",
            false,
        )?;
        assert_eq!(value["bodyHeight"], control, "{doctype}");
        assert_eq!(value["svgHeight"], 300);
    }
    Ok(())
}

#[test]
fn fractional_grid_track_limits_terminate_with_a_finite_item_width() -> Result<()> {
    // This formerly loops in track growth even after the CLI's own timeout.
    // The subprocess deadline also bounds this regression if the loop returns.
    let html = r#"<!doctype html><style>
        body { margin: 0 }
        .grid { display: grid; width: 1000px; height: 100px;
            grid-template-columns: minmax(6.879453%, 20.244017%) }
        </style><div class="grid"><div id="item">test</div></div>"#;
    let value = run_layout(
        html,
        "document.querySelector('#item').getBoundingClientRect().width",
        false,
    )?;
    let width = value.as_f64().context("item width must be a number")?;
    assert!((width - 202.44017).abs() < 0.05, "{width}");
    Ok(())
}

#[test]
fn overflowing_svg_filter_region_keeps_layout_process_alive() -> Result<()> {
    // MOL-599: finite filter attributes overflowed during object-bbox mapping,
    // aborting the process inside tiny-skia even without a screenshot request.
    let expression = "JSON.stringify([document.body.offsetHeight, document.querySelector('svg').getBoundingClientRect().width, document.querySelector('svg').getBoundingClientRect().height])";
    let control = run_layout(
        r#"<!doctype html><svg width="300" height="300"></svg>"#,
        expression,
        false,
    )?;
    let value = run_layout(
        r##"<!doctype html><svg width="300" height="300"><defs><filter id="f" x="-736" width="784"><feGaussianBlur stdDeviation="1"/></filter></defs><rect width="1e38" height="100" filter="url(#f)"/></svg>"##,
        expression,
        false,
    )?;
    assert_eq!(
        value, control,
        "filter overflow must preserve the SVG viewport"
    );
    assert_eq!(value[1], 300);
    assert_eq!(value[2], 300);
    Ok(())
}

#[test]
fn layout_probe_drains_large_evaluation_results_before_waiting_for_exit() -> Result<()> {
    const PAYLOAD_SIZE: usize = 256 * 1024;
    let expression = format!(
        "JSON.stringify({{width:document.querySelector('#item').getBoundingClientRect().width,payload:'x'.repeat({PAYLOAD_SIZE})}})"
    );
    let value = run_layout(
        "<!doctype html><div id='item' style='width:123px'></div>",
        &expression,
        false,
    )?;
    assert_eq!(value["width"], 123);
    assert_eq!(value["payload"], "x".repeat(PAYLOAD_SIZE));
    Ok(())
}

#[test]
fn layout_probe_drains_large_script_errors_before_reporting_exit_failure() {
    let error = run_layout(
        "<!doctype html><p>probe</p>",
        "throw new Error('layout-stderr-marker:' + 'x'.repeat(256 * 1024))",
        false,
    )
    .expect_err("the script error must fail the probe");
    let message = format!("{error:#}");
    assert!(message.contains("layout probe exited with"));
    assert!(message.contains("layout-stderr-marker:"));
}

fn probe_font_document(body: &str) -> String {
    let font = STANDARD.encode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-layout/tests/fixtures/moli-ahem.woff2"
    )));
    format!(
        "<!doctype html><style>@font-face{{font-family:Probe;src:url(data:font/woff2;base64,{font})}}body{{margin:0;font:20px/20px Probe}}</style>{body}"
    )
}

const GEOMETRY: &str = r#"(async () => {
    await document.fonts.ready;
    return Object.fromEntries([...document.querySelectorAll('[id]')].map(element => {
        const rect = element.getBoundingClientRect();
        return [element.id, {x: rect.x, width: rect.width, height: rect.height}];
    }));
})()"#;

fn assert_dimension(value: &Value, id: &str, dimension: &str, expected: f64) {
    let actual = value[id][dimension]
        .as_f64()
        .expect("dimension must be numeric");
    assert!(
        (actual - expected).abs() < 0.05,
        "{id}.{dimension}: {actual} != {expected}"
    );
}

#[test]
fn auto_grid_tracks_honor_intrinsic_item_minima_before_measuring_row_height() -> Result<()> {
    let mut body = String::new();
    for keyword in ["min-content", "max-content"] {
        for width in ["auto", "0px", "50px"] {
            let key = format!("{keyword}-{width}");
            let grid_width = if keyword == "min-content" { 140 } else { 200 };
            body.push_str(&format!(
                "<section><div id='grid-{key}' style='display:grid;grid-template-columns:auto 100px;width:{grid_width}px'><div id='item-{key}' style='min-width:{keyword};width:{width}'>aaaa bbbbb ccc</div><div id='next-{key}'></div></div></section>"
            ));
        }
    }
    let value = run_layout(&probe_font_document(&body), GEOMETRY, true)?;
    for keyword in ["min-content", "max-content"] {
        for width in ["auto", "0px", "50px"] {
            let key = format!("{keyword}-{width}");
            let maximum = keyword == "max-content";
            let item_width = if maximum { 168.0 } else { 60.0 };
            assert_dimension(&value, &format!("item-{key}"), "width", item_width);
            assert_dimension(
                &value,
                &format!("item-{key}"),
                "height",
                if maximum { 20.0 } else { 60.0 },
            );
            assert_dimension(
                &value,
                &format!("grid-{key}"),
                "height",
                if maximum { 20.0 } else { 60.0 },
            );
            assert_dimension(
                &value,
                &format!("next-{key}"),
                "x",
                if maximum { 168.0 } else { 60.0 },
            );
        }
    }
    Ok(())
}

#[test]
fn inline_block_intrinsic_maximum_constrains_auto_width_and_preserves_minimum_sizing() -> Result<()>
{
    let html = probe_font_document(
        "<section><span id='maximum' style='display:inline-block;max-width:min-content'>aaaa bbbbb ccc</span></section><section><span id='minimum' style='display:inline-block;min-width:min-content;width:0'>aaaa bbbbb ccc</span></section><section><span id='auto-minimum' style='display:inline-block;min-width:min-content'>aaaa bbbbb ccc</span></section><section><span id='minimum-wins' style='display:inline-block;min-width:max-content;max-width:min-content'>aaaa bbbbb ccc</span></section>",
    );
    let value = run_layout(&html, GEOMETRY, true)?;
    for id in ["maximum", "minimum"] {
        assert_dimension(&value, id, "width", 60.0);
        assert_dimension(&value, id, "height", 60.0);
    }
    for id in ["auto-minimum", "minimum-wins"] {
        assert_dimension(&value, id, "width", 168.0);
        assert_dimension(&value, id, "height", 20.0);
    }
    Ok(())
}

#[test]
fn flex_percentage_padding_keeps_the_text_content_width_before_line_breaking() -> Result<()> {
    let mut body = String::new();
    for width in [240, 300, 400] {
        for padding in [0, 5, 10, 20] {
            for sizing in ["content-box", "border-box"] {
                body.push_str(&format!(
                    "<section><div style='display:flex;width:{width}px'><div id='item-{width}-{padding}-{sizing}' style='padding:0 {padding}%;box-sizing:{sizing}'>aaa bbb ccc</div></div></section>"
                ));
            }
        }
    }
    let value = run_layout(&probe_font_document(&body), GEOMETRY, true)?;
    for width in [240, 300, 400] {
        for padding in [0, 5, 10, 20] {
            for sizing in ["content-box", "border-box"] {
                let id = format!("item-{width}-{padding}-{sizing}");
                assert_dimension(
                    &value,
                    &id,
                    "width",
                    132.0 + 2.0 * f64::from(width * padding) / 100.0,
                );
                assert_dimension(&value, &id, "height", 20.0);
            }
        }
    }
    Ok(())
}

#[test]
fn intrinsic_width_keywords_keep_margin_space_for_float_and_inline_roles() -> Result<()> {
    let mut body = String::new();
    for role in ["float", "inline"] {
        for (keyword, _) in [
            ("min-content", 46.0),
            ("fit-content", 70.0),
            ("max-content", 94.0),
        ] {
            let display = if role == "float" {
                "float:left"
            } else {
                "display:inline-block"
            };
            body.push_str(&format!("<section style='width:80px;height:100px'><div id='{role}-{keyword}' style='{display};width:{keyword};margin:5px;padding:2px;border:3px solid'>aaa bbb</div><br></section>"));
        }
    }
    let value = run_layout(&probe_font_document(&body), GEOMETRY, true)?;
    for role in ["float", "inline"] {
        for (keyword, width) in [
            ("min-content", 46.0),
            ("fit-content", 70.0),
            ("max-content", 94.0),
        ] {
            assert_dimension(&value, &format!("{role}-{keyword}"), "width", width);
        }
    }
    Ok(())
}

#[test]
fn intrinsic_width_parent_keeps_float_grid_constraints_in_vertical_writing_modes() -> Result<()> {
    let mut body = String::from("<!doctype html><body>");
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for keyword in ["min-content", "max-content"] {
            body.push_str(&format!("<section style='writing-mode:{mode}'><div style='width:{keyword};height:{keyword}'><div id='grid-{mode}-{keyword}' style='display:grid;float:left;grid-template-columns:minmax(100px,200px);grid-template-rows:minmax(50px,100px)'></div></div></section>"));
        }
    }
    let value = run_layout(&body, GEOMETRY, false)?;
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for (keyword, width) in [("min-content", 100.0), ("max-content", 200.0)] {
            let id = format!("grid-{mode}-{keyword}");
            assert_dimension(&value, &id, "width", width);
            assert_dimension(&value, &id, "height", 100.0);
        }
    }
    Ok(())
}

#[test]
fn wrapped_vertical_flex_keeps_padding_in_its_scroll_extent() -> Result<()> {
    let mut body = String::from("<!doctype html><body>");
    for mode in ["vertical-rl", "vertical-lr"] {
        for wrap in ["wrap", "wrap-reverse"] {
            for padding in [0, 10] {
                body.push_str(&format!("<div id='flex-{mode}-{wrap}-{padding}' data-flex-probe style='display:inline-flex;writing-mode:{mode};direction:rtl;flex-flow:column {wrap};width:100px;height:100px;overflow:scroll;border:3px solid;padding:{padding}px;gap:10px;align-items:start'>"));
                for _ in 0..3 {
                    body.push_str("<div style='min-width:110px;min-height:110px'></div>");
                }
                body.push_str("</div>");
            }
        }
    }
    let expression = r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[data-flex-probe]')].map(e => [e.id, {width:e.scrollWidth,height:e.scrollHeight}])))"#;
    let value = run_layout(&body, expression, false)?;
    for mode in ["vertical-rl", "vertical-lr"] {
        for wrap in ["wrap", "wrap-reverse"] {
            for padding in [0, 10] {
                let id = format!("flex-{mode}-{wrap}-{padding}");
                assert_dimension(&value, &id, "width", 110.0 + f64::from(2 * padding));
                assert_dimension(&value, &id, "height", 350.0 + f64::from(2 * padding));
            }
        }
    }
    Ok(())
}

#[test]
fn wrapped_flex_distribution_keeps_child_positions_for_visible_and_scroll_overflow() -> Result<()> {
    let mut body = String::from("<!doctype html><body>");
    for (mode, direction, flow) in [
        ("horizontal-tb", "ltr", "row"),
        ("vertical-rl", "rtl", "column"),
    ] {
        for overflow in ["visible", "scroll"] {
            for align in ["stretch", "space-between", "space-around", "space-evenly"] {
                body.push_str(&format!("<div id='flex-{mode}-{overflow}-{align}' data-flex-probe style='display:flex;writing-mode:{mode};direction:{direction};flex-flow:{flow} wrap-reverse;width:100px;height:100px;overflow:{overflow};border:3px solid;padding:10px;gap:10px;align-items:start;align-content:{align};margin-top:300px'>"));
                for _ in 0..3 {
                    body.push_str("<div style='min-width:110px;min-height:110px'></div>");
                }
                body.push_str("</div>");
            }
        }
    }
    let expression = r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[data-flex-probe]')].map(e => {const top=e.getBoundingClientRect().top;return [e.id,Object.fromEntries([...e.children].map((child,i)=>['y'+i,child.getBoundingClientRect().top-top]))]})))"#;
    let value = run_layout(&body, expression, false)?;
    for mode in ["horizontal-tb", "vertical-rl"] {
        for overflow in ["visible", "scroll"] {
            for align in ["stretch", "space-between", "space-around", "space-evenly"] {
                // Fixed Chromium control values distinguish flex-start fallback from
                // safe center, including the opposite cross flow of vertical RTL.
                let expected = match (mode, align) {
                    ("horizontal-tb", "stretch" | "space-between") => [3.0, -117.0, -237.0],
                    ("horizontal-tb", _) => [253.0, 133.0, 13.0],
                    (_, "stretch" | "space-between") => [13.0, 133.0, 253.0],
                    _ => [-237.0, -117.0, 3.0],
                };
                let id = format!("flex-{mode}-{overflow}-{align}");
                for (index, y) in expected.into_iter().enumerate() {
                    assert_dimension(&value, &id, &format!("y{index}"), y);
                }
            }
        }
    }
    Ok(())
}
