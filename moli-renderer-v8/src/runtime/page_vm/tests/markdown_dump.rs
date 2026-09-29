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
        page.vm_mut().eval(r##"
            document.head.innerHTML = '<style>.off{display:none}.invisible{visibility:hidden}.restored{visibility:visible}#external-override{color:black}</style>';
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
                <p>Visible value <font id="legacy-hidden" color="white">hidden suffix</font> 8</p>
                <p>CSS override <font id="css-override" color="white" style="color:black">VISIBLE</font></p>
                <p>Stylesheet override <font id="external-override" color="white">ALSO VISIBLE</font></p>
                <p style="background:black;color:white">White on black stays visible</p>
                <p style="opacity:0.5">Faded text</p>
                <p style="opacity:0;animation-name:spin">Animated hidden text</p>
                <style>@KEYFRAMES fade-in { from { opacity:0 } to { opacity:1 } }</style>
                <p style="opacity:0;animation:fade-in 0.25s ease-in forwards">Animated revealed text</p>
                <style>@keyframes overridden { to { opacity:1 } } @keyframes overridden { from { opacity:0 } to { opacity:0 } }</style>
                <p style="opacity:0;animation:overridden 1s forwards">Overridden animation text</p>
                <style>@keyframes spin { from { transform:rotate(0deg) } to { transform:rotate(1turn) } }</style>
                <p style="opacity:0;animation:spin 1s forwards">Animated rotation text</p>
                <p style="opacity:0;animation-name:fade-in,spin;animation-duration:0s,1s;animation-fill-mode:forwards">Zero-duration reveal text</p>
                <p style="opacity:0;animation:fade-in 1s infinite forwards">Repeating reveal text</p>
                <p style="opacity:0;animation:fade-in 1s reverse forwards">Reverse reveal text</p>
                <div><span style="display:inline-block">Active</span><span style="display:inline-block">Reviewed</span></div>
                <p>Mass 2.4 × 10<span style="vertical-align:super">−17</span> J; m<span style="vertical-align:sub">n</span></p>
                <p>Account <span style="position:relative;top:-2px">settings</span></p>
                <img style="width:1px;height:1px" src="/tracking.gif">
                <img style="width:1px;height:1px" src="/status.gif" alt="Upload complete">
                <img style="opacity:0" data-src="/article.jpg" src="/spacer.gif" alt="Article photo">
                <div class="invisible">Hidden ancestor <span>Inherited hidden</span><span class="restored">Restored child</span></div>
                <button role="tab" aria-controls="panel">Specifications</button>
                <section role="tabpanel" id="panel" class="off"><p>Panel content</p></section>
                <section role="tabpanel" id="aria-panel" class="off" aria-hidden="true"><p>ARIA panel content</p></section>
                <button aria-expanded="false" aria-controls="more">More</button>
                <section id="more" class="off">Disclosure content</section>
                <details><summary>Details</summary>Closed details content</details>
                <button aria-expanded="false" aria-controls="dialog">Preferences</button>
                <div role="dialog" id="dialog" class="off">Cookie template</div>`;
        "##).unwrap();
        assert_eq!(page.vm_mut().eval("getComputedStyle(document.getElementById('legacy-hidden')).color").unwrap(), "rgb(255, 255, 255)");
        assert_eq!(page.vm_mut().eval("getComputedStyle(document.getElementById('css-override')).color").unwrap(), "rgb(0, 0, 0)");
        assert_eq!(page.vm_mut().eval("getComputedStyle(document.getElementById('external-override')).color").unwrap(), "rgb(0, 0, 0)");
        assert_eq!(page.vm_mut().eval("document.getElementById('legacy-hidden').setAttribute('color','black'); getComputedStyle(document.getElementById('legacy-hidden')).color").unwrap(), "rgb(0, 0, 0)");
        page.vm_mut().eval("document.getElementById('legacy-hidden').setAttribute('color','white')").unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            for kept in ["Visible body", "Restored child", "Panel content", "ARIA panel content", "Disclosure content", "Closed details content", "Followers 2", "Score 313131", "Parser trap", "Accessible helper", "Faded text", "Animated revealed text", "Visible value 8", "CSS override VISIBLE", "Stylesheet override ALSO VISIBLE", "White on black stays visible", "![Article photo](https://example.test/article.jpg)"] {
                assert!(output.contains(kept), "missing {kept}: {output}");
            }
            assert!(output.contains("10<sup>−17</sup>"), "{output}");
            assert!(output.contains("m<sub>n</sub>"), "{output}");
            assert!(output.contains("Active\nReviewed"), "{output}");
            assert!(!output.contains("tracking.gif"), "{output}");
            assert!(output.contains("Upload complete"), "{output}");
            for omitted in ["Translation pending", "Share metadata", "Hidden ancestor", "Inherited hidden", "Cookie template", "Followers 92", "Animated hidden text", "Overridden animation text", "Animated rotation text", "Zero-duration reveal text", "Repeating reveal text", "Reverse reveal text", "Transparent parent", "Transparent child", "hidden suffix", "product.name", "Vue template"] {
                assert!(!output.contains(omitted), "leaked {omitted}: {output}");
            }
            assert!(output.contains("Account settings"), "{output}");
            assert!(!output.contains("<sup>settings</sup>"), "{output}");
        }
        page.vm_mut().eval("document.getElementById('waiting').className = ''").unwrap();
        assert!(page.render_page_dump(options(false)).contains("Translation pending"));
        assert_eq!(page.vm_mut().eval("document.querySelector('style').textContent.includes('.off')").unwrap(), "true");
        page.vm_mut().eval(r#"
            document.documentElement.style.opacity = '0';
            document.body.innerHTML = '<article>Whole-page loading veil</article>';
        "#).unwrap();
        assert!(page.render_page_dump(options(false)).contains("Whole-page loading veil"));
        page.vm_mut().eval(r#"
            document.documentElement.style.opacity = '';
            document.body.style.visibility = 'hidden';
            document.body.innerHTML = '<article style="visibility:visible">Font loading veil<div style="visibility:hidden">Independent hidden text</div></article>';
        "#).unwrap();
        let root_visibility = page.render_page_dump(options(false));
        assert!(root_visibility.contains("Font loading veil"), "{root_visibility}");
        assert!(!root_visibility.contains("Independent hidden text"), "{root_visibility}");
        page.vm_mut().eval("document.body.style.visibility = ''").unwrap();
        page.vm_mut().eval(r#"
            document.head.innerHTML = '<style>.action{display:block}</style>';
            document.body.innerHTML = '<a class="action" href="/accept">Accept</a><a class="action" href="/reject">Reject</a>';
        "#).unwrap();
        let actions = page.render_page_dump(options(false));
        assert!(actions.contains("[Accept](https://example.test/accept)\n\n[Reject](https://example.test/reject)"), "{actions}");
        page.vm_mut().eval(r##"
            document.head.innerHTML = `<script>
                function revealElement(what) {
                    const target = typeof what === 'object' ? what : document.getElementById(what);
                    target.style.display = 'block';
                }
                function inspectElement(what) { console.log(what.textContent); }
                function revealUnrelated(what) { sidebar.style.display = 'block'; console.log(what); }
            </script>`;
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
        page.vm_mut().eval(r##"
            document.body.innerHTML = `
                <button aria-expanded="false" aria-controls="history">+</button>
                <div style="display:none">Hidden unrelated sibling<div><section id="history">Expandable history<div style="display:none">Nested hidden template</div><div role="dialog" style="display:none">Nested dialog template</div></section></div></div>
                <button aria-expanded="false" aria-controls="visibility-panel">Visibility panel</button>
                <section id="visibility-panel" style="visibility:hidden">Visibility disclosure</section>
                <section id="unreferenced" style="display:none">Hidden template</section>`;
        "##).unwrap();
        let expanded = page.render_page_dump(options(false));
        assert!(expanded.contains("Expandable history"), "{expanded}");
        assert!(expanded.contains("Visibility disclosure"), "{expanded}");
        assert!(!expanded.contains("Hidden unrelated sibling"), "{expanded}");
        assert!(!expanded.contains("Nested hidden template"), "{expanded}");
        assert!(!expanded.contains("Nested dialog template"), "{expanded}");
        assert!(!expanded.contains("Hidden template"), "{expanded}");
        page.vm_mut().eval(r##"
            document.body.innerHTML = `
                <button aria-expanded="false" aria-controls="nested-panel">Nested panel</button>
                <section id="nested-panel" style="visibility:hidden">Lead <p><span>Full disclosure body</span></p><div style="visibility:hidden"><span>Nested hidden duplicate</span></div></section>`;
        "##).unwrap();
        let nested_visibility = page.render_page_dump(options(false));
        assert!(nested_visibility.contains("Full disclosure body"), "{nested_visibility}");
        assert!(!nested_visibility.contains("Nested hidden duplicate"), "{nested_visibility}");
        page.vm_mut().eval(r##"
            document.body.innerHTML = `
                <button onclick="console.log('tracking')">Save</button>
                <section id="tracking" style="display:none">Hidden telemetry state</section>
                <button onclick="console.log(&quot;document.getElementById('diagnostic')&quot;)">Log</button>
                <section id="diagnostic" style="display:none">Hidden diagnostic state</section>
                <button onclick="console.log(document.getElementById('readonly').textContent)">Read</button>
                <section id="readonly" style="display:none">Hidden read-only state</section>
                <button onclick="/* document.getElementById('commented') */ console.log('clicked')">Comment</button>
                <section id="commented" style="display:none">Hidden commented state</section>
                <button onclick="document.querySelector('#history').style.display='block'">History</button>
                <section id="history" style="display:none">Query-selected history</section>
                <button onclick="return revealElement(document.getElementById('records'))">Records</button>
                <section id="records" style="display:none">Function-revealed records</section>
                <button onclick="if(false) document.getElementById('unreachable').style.display='block'">Never</button>
                <section id="unreachable" style="display:none">Unreachable hidden state</section>
                <button onclick="showNext(document.getElementById('anchor'))">Next</button>
                <section id="anchor" style="display:none">Hidden anchor metadata</section>
                <section style="display:none">Actual next disclosure</section>
                <button onclick="inspectElement(document.getElementById('inspected'))">Inspect</button>
                <section id="inspected" style="display:none">Function-read hidden state</section>
                <button onclick="revealUnrelated(document.getElementById('unrelated-target'))">Other</button>
                <section id="unrelated-target" style="display:none">Unrelated mutation hidden state</section>`;
        "##).unwrap();
        let related = page.render_page_dump(options(false));
        assert!(!related.contains("Hidden telemetry state"), "{related}");
        assert!(!related.contains("Hidden diagnostic state"), "{related}");
        assert!(!related.contains("Hidden read-only state"), "{related}");
        assert!(!related.contains("Hidden commented state"), "{related}");
        assert!(!related.contains("Query-selected history"), "{related}");
        assert!(!related.contains("Function-revealed records"), "{related}");
        assert!(!related.contains("Unreachable hidden state"), "{related}");
        assert!(!related.contains("Hidden anchor metadata"), "{related}");
        assert!(!related.contains("Actual next disclosure"), "{related}");
        assert!(!related.contains("Function-read hidden state"), "{related}");
        assert!(!related.contains("Unrelated mutation hidden state"), "{related}");
        page.vm_mut().eval(r#"
            document.body.innerHTML = `
                <p style="color:white;background-image:linear-gradient(black,black)">Visible gradient text</p>
                <div style="color:white;background-image:linear-gradient(black,black)"><p>Visible inherited gradient text</p></div>
                <div style="color:white;background:white"><img src="/photo.png" alt="Photo"></div>`;
        "#).unwrap();
        let painted = page.render_page_dump(options(false));
        assert!(painted.contains("Visible gradient text"), "{painted}");
        assert!(painted.contains("Visible inherited gradient text"), "{painted}");
        assert!(painted.contains("![Photo](https://example.test/photo.png)"), "{painted}");
        page.vm_mut().eval(r##"
            document.body.innerHTML = `
                <p style="color:white;background-color:rgba(0,0,0,0.5)">Visible on grey</p>
                <p style="color:white;background:white;text-shadow:0 0 2px black">Visible shadow</p>
                <div style="color:white;background-image:linear-gradient(black,black)"><p style="background:white">Hidden on opaque child</p></div>
                <article><p>A detailed review... <a href="#complete">Read more</a></p><div hidden><div id="complete" style="display:none">A detailed review includes the full conclusion.</div></div></article>`;
        "##).unwrap();
        let composite = page.render_page_dump(options(false));
        assert!(composite.contains("Visible on grey"), "{composite}");
        assert!(composite.contains("Visible shadow"), "{composite}");
        assert!(!composite.contains("Hidden on opaque child"), "{composite}");
        assert!(composite.contains("full conclusion"), "{composite}");
        assert!(!composite.contains("A detailed review..."), "{composite}");
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<input id="field" value="Initial"><input id="check" type="checkbox"><textarea id="notes">Original notes</textarea>';
            document.getElementById('field').value = 'Edited';
            document.getElementById('check').checked = true;
            document.getElementById('notes').value = 'Edited notes';
        "#).unwrap();
        let controls = page.render_page_dump(options(false));
        assert!(controls.contains("Edited"), "{controls}");
        assert!(controls.contains("☑"), "{controls}");
        assert!(controls.contains("Edited notes"), "{controls}");
        assert!(!controls.contains("Initial"), "{controls}");
        assert!(!controls.contains("Original notes"), "{controls}");
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<div title="Group"><textarea id="notes">Old default</textarea></div>';
            document.getElementById('notes').value = '';
        "#).unwrap();
        let cleared = page.render_page_dump(options(false));
        assert!(!cleared.contains("Old default"), "{cleared}");
        page.vm_mut().eval(r#"
            document.body.innerHTML = `
                <img srcset="small.png 1x, large.png 2x" alt="Responsive">
                <img srcset="https://cdn.example/c_fill,w_640/photo.jpg 1x" alt="Comma URL">
                <img data-srcset="lazy-small.png 1x, lazy-large.png 2x" alt="Lazy responsive">
                <img srcset="safe.jpg 1x, invalid.jpg +2x" alt="Invalid density">
                <img srcset="invalid.jpg test(a, phantom.jpg 4x, b), safe-parentheses.jpg 1x" alt="Parentheses">`;
        "#).unwrap();
        let responsive = page.render_page_dump(options(false));
        assert!(responsive.contains("![Responsive](https://example.test/large.png)"), "{responsive}");
        assert!(responsive.contains("![Comma URL](https://cdn.example/c_fill,w_640/photo.jpg)"), "{responsive}");
        assert!(responsive.contains("![Lazy responsive](https://example.test/lazy-large.png)"), "{responsive}");
        assert!(responsive.contains("![Invalid density](https://example.test/safe.jpg)"), "{responsive}");
        assert!(responsive.contains("![Parentheses](https://example.test/safe-parentheses.jpg)"), "{responsive}");
        assert!(!responsive.contains("phantom.jpg"), "{responsive}");
        page.vm_mut().eval("document.documentElement.style.display = 'none'").unwrap();
        assert!(page.render_page_dump(options(false)).is_empty());
    }).await;
}
