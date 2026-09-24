use super::*;
use crate::runtime::page_surface::{
    RendererPageDumpFormat, RendererPageDumpOptions, RendererPageDumpStripOptions,
};

fn options(strip_css: bool) -> RendererPageDumpOptions {
    RendererPageDumpOptions {
        format: RendererPageDumpFormat::Markdown,
        strip: RendererPageDumpStripOptions {
            css: strip_css,
            ..Default::default()
        },
        with_base: false,
        with_frames: false,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_uses_live_visibility_and_preserves_disclosure_content() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/content").unwrap());
        page.vm_mut().eval(r#"
            document.head.innerHTML = '<style>.off{display:none}.invisible{visibility:hidden}.restored{visibility:visible}</style>';
            document.body.innerHTML = `
                <h1>Article</h1><p>Visible body</p>
                <div class="off" id="waiting">Translation pending</div>
                <div hidden>Share metadata</div>
                <p>Followers <span style="opacity:0">9</span><span>2</span></p>
                <p>Score <span aria-hidden="true">3131</span><span class="offscreen">31</span></p>
                <div style="position:fixed;left:-9000px;top:-9000px">Parser trap</div>
                <span style="position:absolute;left:-1px;top:-1px">Accessible helper</span>
                <div style="opacity:0">Transparent parent <span style="opacity:1">Transparent child</span></div>
                <div ng-cloak>Buy {{product.name}} for {{product.price | currency}}</div>
                <div v-cloak>Vue template {{message}}</div>
                <p>Visible value <font color="white">hidden suffix</font> 8</p>
                <p style="background:black;color:white">White on black stays visible</p>
                <p style="opacity:0.5">Faded text</p>
                <div><span style="display:inline-block">Active</span><span style="display:inline-block">Reviewed</span></div>
                <p>Mass 2.4 × 10<span style="position:relative;top:-0.5em;line-height:0">−17</span> J; m<span style="position:relative;bottom:-0.25em;line-height:0">n</span></p>
                <img style="width:1px;height:1px" src="/tracking.gif">
                <img style="width:1px;height:1px" src="/status.gif" alt="Upload complete">
                <img style="opacity:0" data-src="/article.jpg" src="/spacer.gif" alt="Article photo">
                <div class="invisible">Hidden ancestor <span>Inherited hidden</span><span class="restored">Restored child</span></div>
                <button role="tab" aria-controls="panel">Specifications</button>
                <section role="tabpanel" id="panel" class="off"><p>Panel content</p></section>
                <button aria-expanded="false" aria-controls="more">More</button>
                <section id="more" class="off">Disclosure content</section>
                <details><summary>Details</summary>Closed details content</details>
                <button aria-expanded="false" aria-controls="dialog">Preferences</button>
                <div role="dialog" id="dialog" class="off">Cookie template</div>`;
        "#).unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            for kept in ["Visible body", "Restored child", "Panel content", "Disclosure content", "Closed details content", "Followers 2", "Score 31", "Accessible helper", "Faded text", "Visible value 8", "White on black stays visible", "![Article photo](https://example.test/article.jpg)"] {
                assert!(output.contains(kept), "missing {kept}: {output}");
            }
            assert!(output.contains("10<sup>−17</sup>"), "{output}");
            assert!(output.contains("m<sub>n</sub>"), "{output}");
            assert!(output.contains("Active\nReviewed"), "{output}");
            assert!(!output.contains("tracking.gif"), "{output}");
            assert!(output.contains("Upload complete"), "{output}");
            for omitted in ["Translation pending", "Share metadata", "Hidden ancestor", "Inherited hidden", "Cookie template", "Followers 92", "Score 3131", "Parser trap", "Transparent parent", "Transparent child", "product.name", "Vue template", "hidden suffix"] {
                assert!(!output.contains(omitted), "leaked {omitted}: {output}");
            }
        }
        page.vm_mut().eval("document.getElementById('waiting').className = ''").unwrap();
        assert!(page.render_page_dump(options(false)).contains("Translation pending"));
        assert_eq!(page.vm_mut().eval("document.querySelector('style').textContent.includes('.off')").unwrap(), "true");
        page.vm_mut().eval(r#"
            document.head.innerHTML = '<style>.action{display:block}</style>';
            document.body.innerHTML = '<a class="action" href="/accept">Accept</a><a class="action" href="/reject">Reject</a>';
        "#).unwrap();
        let actions = page.render_page_dump(options(false));
        assert!(actions.contains("[Accept](https://example.test/accept)\n\n[Reject](https://example.test/reject)"), "{actions}");
        page.vm_mut().eval(r##"
            document.body.innerHTML = `
                <article><p>A detailed review starts... <a data-src="#complete" href="javascript:;">Read more</a></p>
                <div id="complete" style="display:none">A detailed review starts here and retains the final conclusion.</div></article>
                <article><p>A different excerpt... <a href="#unrelated">Read more</a></p>
                <div id="unrelated" style="display:none">Unrelated hidden template</div></article>
                <article><p>Complete short review.</p><div style="display:none">Complete short review.</div></article>
                <article><p>Complete short review.</p></article>
                <article><p>Privacy policy... <a data-target="#privacy">Read more</a></p>
                <div role="dialog" id="privacy" style="display:none">Privacy policy with cookie settings</div></article>`;
        "##).unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            assert_eq!(output.matches("A detailed review starts").count(), 1, "{output}");
            assert!(output.contains("retains the final conclusion."), "{output}");
            assert!(!output.contains("A detailed review starts..."), "{output}");
            assert!(output.contains("A different excerpt..."), "{output}");
            assert!(!output.contains("Unrelated hidden template"), "{output}");
            assert_eq!(output.matches("Complete short review.").count(), 2, "{output}");
            assert!(output.contains("Privacy policy..."), "{output}");
            assert!(!output.contains("cookie settings"), "{output}");
        }
        page.vm_mut().eval("document.documentElement.style.display = 'none'").unwrap();
        assert!(page.render_page_dump(options(false)).is_empty());
    }).await;
}
