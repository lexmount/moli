use super::*;
use crate::runtime::page_surface::{RendererContentStripOptions, RendererHtmlSerializeOptions};

#[tokio::test(flavor = "current_thread")]
async fn html_dump_reads_live_frames_and_keeps_dom_indices_and_observers_unchanged() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), Url::parse("https://example.test/docs/page?a=1&b=2").unwrap());
        page.vm_mut().eval(r#"
            document.head.innerHTML = '<base href="/authored" target="_blank"><style>p{color:red}</style>';
            document.body.innerHTML = '<nav>MENU</nav><p style="color:red" onclick="void 0">KEEP</p><script>mainSecret=1</script><div id=host></div><iframe id=frame src="about:blank" data-moli-frame-url="authored"></iframe>';
            globalThis.dumpShadow = document.getElementById('host').attachShadow({mode:'closed'});
            dumpShadow.innerHTML = '<style>b{color:red}</style><b style="color:red">SHADOW</b>';
        "#).unwrap();
        crate::runtime::page_vm::tests::materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
        page.vm_mut().eval(r#"
            document.getElementById('frame').contentDocument.body.innerHTML = '<p style="color:red" onclick="void 0">CURRENT</p><script>childSecret=1</script>';
            globalThis.dumpMutations = 0;
            globalThis.dumpObservers = [document, dumpShadow, document.getElementById('frame').contentDocument].map(root => {
                const observer = new MutationObserver(records => dumpMutations += records.length);
                observer.observe(root, {subtree:true,childList:true,attributes:true,characterData:true});
                return observer;
            });
        "#).unwrap();
        let snapshot = r#"JSON.stringify([document.documentElement.outerHTML,dumpShadow.innerHTML,document.getElementById('frame').contentDocument.documentElement.outerHTML,document.baseURI])"#;
        let before = page.vm_mut().eval(snapshot).unwrap();
        let version = page.vm().document_runtime.dom_host().dom_version();
        for bits in 0..8 {
            for with_base in [false, true] {
                for with_frames in [false, true] {
                    let strip = RendererContentStripOptions { js: bits & 1 != 0, css: bits & 2 != 0, ui: bits & 4 != 0 };
                    let access = crate::dom::native::serialization_probe::DomAccessScope::begin();
                    let output = page.serialize_html(RendererHtmlSerializeOptions { strip, with_base, with_frames, ..Default::default() }).unwrap();
                    assert_eq!(access.finish(), crate::dom::native::serialization_probe::DomAccessCounts::default());
                    assert!(output.contains("KEEP") && output.contains("SHADOW"), "{output}");
                    assert_eq!(output.contains("CURRENT"), with_frames, "{output}");
                    assert_eq!(output.contains("MENU"), !strip.ui, "{output}");
                    assert_eq!(output.contains("mainSecret"), !strip.js, "{output}");
                    assert_eq!(output.contains("onclick="), !strip.js, "{output}");
                    assert_eq!(output.contains("color:red"), !strip.css, "{output}");
                    assert_eq!(output.contains("href=\"https://example.test/docs/page?a=1&amp;b=2\""), with_base, "{output}");
                    assert_eq!(page.vm().document_runtime.dom_host().dom_version(), version);
                    assert_eq!(page.vm_mut().eval(snapshot).unwrap(), before);
                    assert_eq!(page.vm_mut().eval("dumpMutations + dumpObservers.reduce((n, observer) => n + observer.takeRecords().length, 0)").unwrap(), "0");
                }
            }
        }
        let access = crate::dom::native::serialization_probe::DomAccessScope::begin();
        assert!(page.serialize_html(RendererHtmlSerializeOptions { with_frames: true, max_output_bytes: Some(10), ..Default::default() }).is_err());
        assert_eq!(access.finish(), crate::dom::native::serialization_probe::DomAccessCounts::default());
        assert_eq!(page.vm_mut().eval(snapshot).unwrap(), before);
        let again = page.serialize_html(RendererHtmlSerializeOptions { with_frames: true, ..Default::default() }).unwrap();
        assert!(again.contains("CURRENT"));
    }).await;
}
