use super::{clean_output, run_moli, unique_temp_dir};
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use url::Url;

// A 1x1 PNG, so an image that loaded reports a natural width of 1.
const PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const PAGE: &str = r#"<!doctype html>
<link rel="stylesheet" href="style/page.css">
<h1 id="title">Report</h1>
<img id="pixel" src="images/pixel.png">
<p id="script">script did not run</p>
<script src="scripts/page.js"></script>
"#;

const PAGE_PROBE: &str = "[document.getElementById('script').textContent, \
    getComputedStyle(document.getElementById('title')).color, \
    document.getElementById('pixel').naturalWidth].join('|')";

fn write_local_page() -> Result<Url> {
    let dir = unique_temp_dir("file-urls")?;
    for subdirectory in ["style", "images", "scripts"] {
        std::fs::create_dir_all(dir.join(subdirectory))?;
    }
    std::fs::write(dir.join("page.html"), PAGE)?;
    std::fs::write(
        dir.join("style/page.css"),
        "#title { color: rgb(1, 2, 3); }",
    )?;
    std::fs::write(dir.join("images/pixel.png"), PIXEL_PNG)?;
    std::fs::write(
        dir.join("scripts/page.js"),
        "document.getElementById('script').textContent = 'script ran';",
    )?;
    Ok(Url::from_file_path(dir.join("page.html")).expect("temporary path is absolute"))
}

fn run_fetch_eval(url: &str, expression: &str, extra_args: &[&str]) -> Result<super::Output> {
    let mut args = vec![
        "moli",
        "fetch",
        "--log-level",
        "error",
        "--wait-until",
        "load",
        "--image",
        "--eval",
        expression,
    ];
    args.extend_from_slice(extra_args);
    args.push(url);
    run_moli(args)
}

#[test]
fn fetch_cli_refuses_file_urls_without_the_capability() -> Result<()> {
    let page = write_local_page()?;
    let output = run_fetch_eval(page.as_str(), PAGE_PROBE, &[])?;

    assert!(!output.status.success());
    assert!(
        clean_output(&output.stderr).contains(
            "Navigation to a local file URL requires an explicitly granted browser capability."
        ),
        "{}",
        clean_output(&output.stderr)
    );
    Ok(())
}

#[test]
fn fetch_cli_allow_file_urls_loads_a_file_document_with_relative_subresources() -> Result<()> {
    let page = write_local_page()?;
    let output = run_fetch_eval(page.as_str(), PAGE_PROBE, &["--allow-file-urls"])?;

    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    assert_eq!(clean_output(&output.stdout), "script ran|rgb(1, 2, 3)|1\n");
    Ok(())
}

#[test]
fn fetch_cli_allow_file_urls_keeps_files_away_from_web_documents() -> Result<()> {
    let page = write_local_page()?;
    let pixel = page.join("images/pixel.png")?;
    let pixel_data_url = format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(PIXEL_PNG)
    );
    // The data: image proves this document loads images at all, so a zero
    // width for the file image means it was refused rather than skipped.
    let web_document = format!(
        "data:text/html,<img id=\"control\" src=\"{pixel_data_url}\">\
         <img id=\"pixel\" src=\"{pixel}\"><script>\
         window.popup = window.open('{page}', 'local');\
         window.fetched = fetch('{pixel}').then(() => 'read', () => 'refused');\
         </script>"
    );
    // `--wait-until load` has settled both images; the popup needs a moment.
    let probe = "Promise.all([window.fetched, new Promise(resolve => setTimeout(resolve, 200))])\
        .then(([fetched]) => [\
            document.getElementById('control').naturalWidth,\
            document.getElementById('pixel').naturalWidth,\
            fetched,\
            (() => {\
                try {\
                    return window.popup.document.getElementById('title') ? 'popup read' : 'popup empty';\
                } catch (error) {\
                    return 'popup refused';\
                }\
            })(),\
        ].join('|'))";
    let output = run_fetch_eval(&web_document, probe, &["--allow-file-urls"])?;

    assert!(output.status.success(), "{}", clean_output(&output.stderr));
    let result = clean_output(&output.stdout);
    assert!(result.starts_with("1|0|refused|popup "), "{result}");
    assert!(!result.contains("popup read"), "{result}");
    Ok(())
}
