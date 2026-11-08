use super::*;

#[tokio::test(flavor = "current_thread")]
async fn markdown_keeps_code_after_custom_element_upgrade() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/docs/page").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<h1>Files</h1><mdn-code-example id="example"><pre><code class="language-js">const file = "note.txt";\nconsole.log(file);</code></pre></mdn-code-example>';
        "#).unwrap();
        let before = page.render_page_dump(options(false));
        assert!(before.contains("```js\nconst file = \"note.txt\";\nconsole.log(file);\n```"), "{before}");
        page.vm_mut().eval(r#"
            customElements.define('mdn-code-example', class extends HTMLElement {
                connectedCallback() {
                    this.attachShadow({mode: 'open'}).append(this.querySelector('pre'));
                }
            });
        "#).unwrap();
        assert_eq!(page.vm_mut().eval("document.querySelectorAll('pre').length").unwrap(), "0");
        let snapshot = "JSON.stringify([document.body.innerHTML, document.getElementById('example').shadowRoot.innerHTML])";
        let live = page.vm_mut().eval(snapshot).unwrap();
        for strip_css in [false, true] {
            for with_base in [false, true] {
                for with_frames in [false, true] {
                    let mut dump = options(strip_css);
                    dump.with_base = with_base;
                    dump.with_frames = with_frames;
                    assert_eq!(page.render_page_dump(dump), before, "strip_css={strip_css}, with_base={with_base}, with_frames={with_frames}");
                    assert_eq!(page.vm_mut().eval(snapshot).unwrap(), live, "export must preserve the live light and shadow trees");
                }
            }
        }
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<p>Plain document after shadow removal</p>';
        "#).unwrap();
        for strip_css in [false, true] {
            assert_eq!(
                page.render_page_dump(options(strip_css)),
                "Plain document after shadow removal"
            );
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_uses_slot_order_and_fallback_without_duplicate_light_content() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = `<p>Before</p><x-article id=host>
                <p>Default content</p><p slot=lead>Named content</p>
                <pre slot=unused>Unassigned code</pre>
            </x-article><p>After</p>`;
            document.getElementById('host').attachShadow({mode: 'open'}).innerHTML = `
                <h2>Shadow heading</h2>
                <slot name=lead>Wrong named fallback</slot>
                <slot>Wrong default fallback</slot>
                <slot name=absent><p>Empty slot fallback</p></slot>
                <slot name=lead><p>Second slot fallback</p></slot>`;
        "#).unwrap();
        let snapshot = "JSON.stringify([document.body.innerHTML, document.getElementById('host').shadowRoot.innerHTML])";
        let live = page.vm_mut().eval(snapshot).unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            let mut previous = 0;
            for text in ["Before", "Shadow heading", "Named content", "Default content", "Empty slot fallback", "Second slot fallback", "After"] {
                assert_eq!(output.matches(text).count(), 1, "{text}: {output}");
                let position = output.find(text).unwrap();
                assert!(position >= previous, "slot composition order: {output}");
                previous = position;
            }
            for text in ["Wrong named fallback", "Wrong default fallback", "Unassigned code"] {
                assert!(!output.contains(text), "unrendered light/fallback content leaked: {output}");
            }
            assert_eq!(page.vm_mut().eval(snapshot).unwrap(), live);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_reads_nested_closed_shadow_roots_and_resolves_resource_urls() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/docs/page").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<x-outer id=outer>Unassigned outer text</x-outer>';
            {
                const outer = document.getElementById('outer').attachShadow({mode: 'closed'});
                outer.innerHTML = '<section><x-inner>Unassigned inner text</x-inner></section>';
                const inner = outer.querySelector('x-inner').attachShadow({mode: 'closed'});
                inner.innerHTML = '<pre><code class=language-js>console.log("nested");</code></pre><p><a href=../reference>Reference</a></p>';
            }
        "#).unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            assert!(output.contains("```js\nconsole.log(\"nested\");\n```"), "{output}");
            assert_eq!(output.matches("console.log").count(), 1, "{output}");
            assert!(output.contains("[Reference](https://example.test/reference)"), "{output}");
            assert!(!output.contains("Unassigned"), "{output}");
            assert_eq!(page.vm_mut().eval("document.getElementById('outer').shadowRoot === null").unwrap(), "true");
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_uses_shadow_styles_and_live_host_visibility() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<x-example id=host></x-example><p>Outside</p>';
            document.getElementById('host').attachShadow({mode: 'open'}).innerHTML = `
                <style>.hidden { display: none }</style>
                <pre><code>Visible source</code></pre>
                <p class=hidden>Hidden shadow text</p>`;
        "#).unwrap();
        assert_eq!(page.vm_mut().eval("getComputedStyle(document.getElementById('host').shadowRoot.querySelector('.hidden')).display").unwrap(), "none");
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            assert!(output.contains("Visible source"), "{output}");
            assert!(!output.contains("Hidden shadow text"), "{output}");
        }
        page.vm_mut().eval("document.getElementById('host').style.display = 'none'").unwrap();
        for strip_css in [false, true] {
            assert_eq!(page.render_page_dump(options(strip_css)), "Outside");
        }
        page.vm_mut().eval(r#"
            document.getElementById('host').style.display = 'block';
            document.getElementById('host').shadowRoot.querySelector('.hidden').style.display = 'block';
        "#).unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            assert!(output.contains("Visible source"), "{output}");
            assert!(output.contains("Hidden shadow text"), "{output}");
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_tracks_manual_slot_reassignment() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<x-manual id=host><pre id=source>Assigned source</pre><p>Unassigned text</p></x-manual>';
            {
                const host = document.getElementById('host');
                const root = host.attachShadow({mode: 'open', slotAssignment: 'manual'});
                root.innerHTML = '<slot id=first><p>First fallback</p></slot><slot id=second><p>Second fallback</p></slot>';
                root.getElementById('first').assign(document.getElementById('source'));
            }
        "#).unwrap();
        let first = page.render_page_dump(options(false));
        assert_eq!(first.matches("Assigned source").count(), 1, "{first}");
        assert!(first.contains("Second fallback"), "{first}");
        assert!(!first.contains("First fallback") && !first.contains("Unassigned text"), "{first}");
        page.vm_mut().eval("document.getElementById('host').shadowRoot.getElementById('second').assign(document.getElementById('source'))").unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            assert_eq!(output.matches("Assigned source").count(), 1, "{output}");
            assert!(output.contains("First fallback"), "{output}");
            assert!(!output.contains("Second fallback") && !output.contains("Unassigned text"), "{output}");
        }
        assert_eq!(page.vm_mut().eval("document.getElementById('source').parentNode.id").unwrap(), "host");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_reads_shadow_slots_and_live_updates_without_mutating_the_dom() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = `<div id=host><span slot=title>Title</span><p>Body</p><span slot=unused>Unassigned</span></div>`;
            globalThis.markdownShadow = document.querySelector('#host').attachShadow({mode:'closed'});
            markdownShadow.innerHTML = `<h2><slot name=title>Default title</slot></h2><main><slot>Default body</slot></main><p><slot name=empty>Fallback</slot></p>`;
        "#).unwrap();
        let snapshot = "JSON.stringify([document.body.innerHTML, markdownShadow.innerHTML, Array.from(markdownShadow.querySelectorAll('slot'), slot => slot.assignedNodes().map(node => node.textContent))])";
        let before = page.vm_mut().eval(snapshot).unwrap();
        let dom_version = page.vm().document_runtime.dom_host().dom_version();
        for with_frames in [false, true] {
            for with_base in [false, true] {
                for strip_css in [false, true] {
                    let mut config = options(strip_css);
                    config.with_frames = with_frames;
                    config.with_base = with_base;
                    assert_eq!(page.render_page_dump(config), "## Title\n\nBody\n\nFallback");
                    assert_eq!(page.vm().document_runtime.dom_host().dom_version(), dom_version);
                    assert_eq!(page.vm_mut().eval(snapshot).unwrap(), before);
                }
            }
        }
        page.vm_mut().eval(r#"
            document.querySelector('[slot=title]').slot = 'unused';
            document.querySelector('p').textContent = 'Updated body';
            markdownShadow.querySelector('[name=empty]').textContent = 'Updated fallback';
        "#).unwrap();
        assert_eq!(page.render_page_dump(options(false)), "## Default title\n\nUpdated body\n\nUpdated fallback");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_shadow_visibility_follows_slots_and_shadow_hosts() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = `<div id=host><span slot=restored style="visibility:visible">Restored</span><span><span>Inherited hidden</span></span></div><p>Outside</p>`;
            globalThis.markdownShadow = document.querySelector('#host').attachShadow({mode:'open'});
            markdownShadow.innerHTML = `<section style="display:none"><slot name=restored></slot><slot></slot></section>`;
        "#).unwrap();
        for strip_css in [false, true] {
            assert_eq!(page.render_page_dump(options(strip_css)), "Outside");
        }
        page.vm_mut().eval("markdownShadow.querySelector('section').style = 'visibility:hidden'").unwrap();
        for strip_css in [false, true] {
            assert_eq!(page.render_page_dump(options(strip_css)), "Restored\n\nOutside");
        }
        page.vm_mut().eval("document.querySelector('#host').style = 'display:none'").unwrap();
        assert_eq!(page.render_page_dump(options(false)), "Outside");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_manual_slots_preserve_order_nested_roots_and_fallback() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = `<div id=host><p id=first>First</p><p id=second>Second</p><div id=nested>Unassigned nested text</div><p>Unassigned</p></div>`;
            globalThis.markdownShadow = document.querySelector('#host').attachShadow({mode:'closed', slotAssignment:'manual'});
            markdownShadow.innerHTML = '<slot><p>Fallback</p></slot>';
            document.querySelector('#nested').attachShadow({mode:'open'}).innerHTML = '<p>Nested</p>';
            markdownShadow.querySelector('slot').assign(document.querySelector('#second'), document.querySelector('#nested'), document.querySelector('#first'));
        "#).unwrap();
        assert_eq!(page.render_page_dump(options(false)), "Second\n\nNested\n\nFirst");
        page.vm_mut().eval("markdownShadow.querySelector('slot').assign(document.querySelector('#first'), document.querySelector('#second'))").unwrap();
        assert_eq!(page.render_page_dump(options(true)), "First\n\nSecond");
        page.vm_mut().eval("markdownShadow.querySelector('slot').assign()").unwrap();
        assert_eq!(page.render_page_dump(options(false)), "Fallback");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn markdown_shadow_strip_options_preserve_live_controls_and_attributes() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/article/").unwrap());
        page.vm_mut().eval(r#"
            document.body.innerHTML = '<div id=host></div>';
            globalThis.markdownShadow = document.querySelector('#host').attachShadow({mode:'open'});
            markdownShadow.innerHTML = `<nav>Navigation</nav><p><a href=docs style="display:block" onclick="return false">Docs</a></p><textarea>Default</textarea><p style="display:none">Hidden</p>`;
            markdownShadow.querySelector('textarea').value = 'Edited';
        "#).unwrap();
        let snapshot = "JSON.stringify([markdownShadow.innerHTML, markdownShadow.querySelector('textarea').value])";
        let before = page.vm_mut().eval(snapshot).unwrap();
        for strip_css in [false, true] {
            let output = page.render_page_dump(options(strip_css));
            assert!(output.contains("Navigation"), "{output}");
            assert!(output.contains("[Docs](https://example.test/article/docs)"), "{output}");
            assert!(output.contains("Edited"), "{output}");
            assert!(!output.contains("Default") && !output.contains("Hidden"), "{output}");
            let mut config = options(strip_css);
            config.strip.ui = true;
            config.strip.js = true;
            assert_eq!(page.render_page_dump(config), "[Docs](https://example.test/article/docs)");
            assert_eq!(page.vm_mut().eval(snapshot).unwrap(), before);
        }
    }).await;
}
