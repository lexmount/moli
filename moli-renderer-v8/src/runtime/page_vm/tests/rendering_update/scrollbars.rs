use super::*;

#[tokio::test(flavor = "current_thread")]
async fn hidden_scrollbars_preserve_geometry_scrolling_and_computed_css_in_all_frames() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/hidden-scrollbars.html")?,
        );
        page.vm_mut().set_scrollbars_hidden(true);
        page.vm_mut().set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 1280,
            inner_height: 720,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))?;
        page.vm_mut().eval(r#"
document.head.innerHTML = `<style>
html,body { margin:0; padding:0; }
html { scrollbar-width:auto !important; }
#viewport { width:100vw; height:2000px; background:rgb(0,255,0); }
.scroller { position:absolute; top:0; width:200px; height:100px; overflow:scroll;
  scrollbar-width:auto !important; scrollbar-color:red blue; }
.content { width:400px; height:300px; background:red; }
#auto { left:0; }
#thin { left:220px; direction:rtl; scrollbar-width:thin !important; }
#vertical { left:440px; writing-mode:vertical-rl; }
#shadow-host { position:absolute; left:660px; top:0; }
#pseudo { position:absolute; left:880px; top:0; }
#pseudo::before { content:''; display:block; width:200px; height:100px; overflow:scroll;
  scrollbar-color:blue blue; background:red; }
iframe { position:absolute; left:0; top:180px; width:300px; height:200px; border:0; }
</style>`;
document.body.innerHTML = `<div id=viewport></div>
<div id=auto class=scroller><div class=content></div></div>
<div id=thin class=scroller><div class=content></div></div>
<div id=vertical class=scroller><div class=content></div></div>
<div id=shadow-host></div><div id=pseudo></div><iframe id=frame></iframe>`;
const shadow = document.getElementById('shadow-host').attachShadow({mode:'open'});
shadow.innerHTML = `<div id=scroller style='width:200px;height:100px;overflow:auto'>
  <div style='width:400px;height:300px;background:red'></div></div>`;
const child = document.getElementById('frame').contentDocument;
child.documentElement.style.cssText = 'scrollbar-width:thin';
child.body.style.margin = '0';
child.body.innerHTML = `<div style='width:100vw;height:2000px'></div>
  <iframe id=nested style='position:absolute;left:0;top:0;width:120px;height:80px;border:0'></iframe>`;
const nested = child.getElementById('nested').contentDocument;
nested.body.style.margin = '0';
nested.body.innerHTML = `<div style='width:100vw;height:2000px'></div>`;
'installed'
"#)?;
        page.vm_mut().sync_live_document_style_sources();
        let snapshot = page.vm_mut().screenshot_layout_snapshot(
            moli_layout::PaintViewport::new(1280, 720, 1.0),
        )?.expect("fixture layout");
        {
            let host = page.vm().context_host_weak_for_test().upgrade().expect("context host");
            let host = host.borrow();
            assert!(host.with_latest_layout_tree_for_document(host.document_handle(), |tree| {
                for (x, y) in [(190.0, 30.0), (1270.0, 30.0), (1070.0, 30.0)] {
                    assert!(tree.scrollbar_hit_test(moli_layout::LayoutPoint::new(x, y)).is_none());
                }
            }).is_some());
        }
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let offset = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[offset..offset + 4]).unwrap()
        };
        assert_eq!(pixel(1270, 500), [0, 255, 0, 255], "viewport has no scrollbar paint");
        assert_eq!(pixel(190, 70), [255, 0, 0, 255], "element has no scrollbar paint");
        assert_eq!(pixel(1070, 70), [255, 0, 0, 255], "pseudo has no scrollbar paint");

        let metrics: serde_json::Value = serde_json::from_str(&page.vm_mut().eval(r#"JSON.stringify({
  root: [innerWidth, document.documentElement.clientWidth, document.documentElement.scrollWidth],
  elements: ['auto','thin','vertical'].map(id => {
    const e = document.getElementById(id);
    return [e.clientWidth,e.clientHeight,e.clientLeft,e.clientTop,e.scrollWidth,e.scrollHeight];
  }),
  shadow: (() => {
    const e = document.getElementById('shadow-host').shadowRoot.getElementById('scroller');
    return [e.clientWidth,e.clientHeight,e.scrollWidth,e.scrollHeight];
  })(),
  frames: (() => {
    const child = document.getElementById('frame').contentDocument;
    const nested = child.getElementById('nested').contentDocument;
    return [child.documentElement.clientWidth,child.documentElement.scrollWidth,
      nested.documentElement.clientWidth,nested.documentElement.scrollWidth];
  })(),
  css: [getComputedStyle(document.getElementById('auto')).scrollbarWidth,
    getComputedStyle(document.getElementById('thin')).scrollbarWidth,
    getComputedStyle(document.getElementById('auto')).scrollbarGutter]
})"#)?)?;
        assert_eq!(metrics, json!({
            "root": [1280,1280,1280],
            "elements": [[200,100,0,0,400,300],[200,100,0,0,400,300],[200,100,0,0,400,300]],
            "shadow": [200,100,400,300],
            "frames": [300,300,120,120],
            "css": ["auto","thin","auto"]
        }));
        let scroll = page.vm_mut().eval(r#"
const scroller = document.getElementById('auto');
scroller.scrollLeft = 20;
scroller.scrollTop = 40;
window.scrollTo(0,100);
JSON.stringify([scroller.scrollLeft,scroller.scrollTop,scrollY])
"#)?;
        assert_eq!(scroll, "[20,40,100]");
        page.vm_mut().eval("document.documentElement.style.scrollbarGutter = 'stable both-edges'")?;
        page.vm_mut().publish_layout_for_test()?;
        // Root clientWidth excludes only painted scrollbar UI, while stable
        // gutters continue to reduce the root's layout box.
        assert_eq!(page.vm_mut().eval("JSON.stringify([document.documentElement.clientWidth,document.documentElement.getBoundingClientRect().width])")?, "[1280,1250]", "explicit viewport gutters remain reserved");
        page.vm_mut().eval("document.documentElement.style.scrollbarGutter = 'auto'")?;
        page.vm_mut().eval("document.getElementById('viewport').style.width = '1400px'")?;
        page.vm_mut().publish_layout_for_test()?;
        assert_eq!(page.vm_mut().eval("JSON.stringify([document.documentElement.clientWidth,document.documentElement.scrollWidth])")?, "[1280,1400]", "real horizontal overflow remains observable");
        Ok::<_, anyhow::Error>(())
    }).await.expect("hidden scrollbar layout");
}
