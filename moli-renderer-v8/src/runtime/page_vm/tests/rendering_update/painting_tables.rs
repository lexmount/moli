// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_collapsed_table_borders_with_chromium_geometry_and_pixels() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/collapsed-table-borders.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}table{position:absolute;border-collapse:collapse;table-layout:fixed;padding:0}td{box-sizing:border-box;padding:0}
#precedence{left:20px;top:20px;width:100px;border:4px solid rgb(90,0,90)}
#precedence colgroup{border-right:4px solid rgb(0,130,0)}
#precedence col{border-bottom:4px solid rgb(0,0,180)}
#precedence tbody{border-left:4px solid rgb(230,120,0)}
#precedence tr{border-top:4px solid rgb(0,160,160)}
#precedence td{width:50px;height:30px;background:rgb(240,240,240)}
#rules{left:20px;top:90px;width:120px;border:2px solid black}
#rules td{width:60px;height:30px;background:rgb(245,245,210)}
#wide-left{border-right:4px solid rgb(0,0,255)}#wide-right{border-left:8px solid rgb(255,0,0)}
#style-left{border-right:6px dashed rgb(0,150,0)}#style-right{border-left:6px solid rgb(0,0,255)}
#hidden-left{border-right:20px double rgb(120,0,120)}#hidden-right{border-left:1px hidden red}
#span{left:20px;top:220px;width:100px;border:0}
#span td{width:50px;height:30px;padding:0}
#spanning{border:6px solid rgb(0,140,0);background:rgb(210,255,210)}
#upper,#lower{border:2px solid rgb(220,0,0);background:rgb(255,220,220)}
#colspan{left:20px;top:300px;width:100px;border:0}
#colspan td{width:50px;height:30px;padding:0}
#across{border:6px solid rgb(0,140,0);background:rgb(210,255,210)}
#col-left,#col-right{border:2px solid rgb(220,0,0);background:rgb(255,220,220)}
</style>`;
document.body.innerHTML = `<table id=precedence><colgroup><col><col></colgroup><tbody><tr><td id=p0></td><td id=p1></td></tr></tbody></table>
<table id=rules><tbody><tr><td id=wide-left></td><td id=wide-right></td></tr><tr><td id=style-left></td><td id=style-right></td></tr><tr><td id=hidden-left></td><td id=hidden-right></td></tr></tbody></table>
<table id=span><tbody><tr><td id=spanning rowspan=2></td><td id=upper></td></tr><tr><td id=lower></td></tr></tbody></table>
<table id=colspan><tbody><tr><td id=across colspan=2></td></tr><tr><td id=col-left></td><td id=col-right></td></tr></tbody></table>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.vm_mut().publish_layout_for_test()?;

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['precedence','p0','p1','rules','wide-left','wide-right','style-left','style-right','hidden-left','hidden-right','span','spanning','upper','lower','colspan','across','col-left','col-right'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("precedence", [20.0, 20.0, 104.0, 34.0]),
            ("p0", [22.0, 22.0, 50.0, 30.0]),
            ("p1", [72.0, 22.0, 50.0, 30.0]),
            ("rules", [20.0, 90.0, 122.0, 92.0]),
            ("wide-left", [21.0, 91.0, 60.0, 30.0]),
            ("wide-right", [81.0, 91.0, 60.0, 30.0]),
            ("style-left", [21.0, 121.0, 60.0, 30.0]),
            ("style-right", [81.0, 121.0, 60.0, 30.0]),
            ("hidden-left", [21.0, 151.0, 60.0, 30.0]),
            ("hidden-right", [81.0, 151.0, 60.0, 30.0]),
            ("span", [20.0, 220.0, 104.0, 66.0]),
            ("spanning", [23.0, 223.0, 50.0, 60.0]),
            ("upper", [73.0, 223.0, 50.0, 30.0]),
            ("lower", [73.0, 253.0, 50.0, 30.0]),
            ("colspan", [20.0, 300.0, 100.0, 64.0]),
            ("across", [23.0, 303.0, 94.0, 30.0]),
            ("col-left", [23.0, 333.0, 47.0, 30.0]),
            ("col-right", [70.0, 333.0, 47.0, 30.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}"
                );
            }
        }

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(180, 380, 1.0))?
            .expect("collapsed table fixture must retain a layout root");
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "collapsed-table-border-fallback"
        }));
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        for (label, point, expected) in [
            ("row beats lower sources", (70, 21), [0, 160, 160, 255]),
            ("row-group beats columns", (21, 35), [230, 120, 0, 255]),
            ("column beats table", (70, 51), [0, 0, 180, 255]),
            ("column-group beats table", (122, 35), [0, 130, 0, 255]),
            ("wider edge wins", (80, 105), [255, 0, 0, 255]),
            ("solid beats equal dashed", (80, 135), [0, 0, 255, 255]),
            (
                "hidden suppresses wider double",
                (80, 165),
                [245, 245, 210, 255],
            ),
            (
                "rowspan suppresses its internal edge",
                (45, 253),
                [210, 255, 210, 255],
            ),
            (
                "neighbor keeps its horizontal edge",
                (95, 253),
                [220, 0, 0, 255],
            ),
            (
                "colspan suppresses its internal edge",
                (70, 318),
                [210, 255, 210, 255],
            ),
            (
                "next row keeps its vertical edge",
                (70, 348),
                [220, 0, 0, 255],
            ),
        ] {
            assert_eq!(pixel(point.0, point.1), expected, "{label}");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("collapsed table border fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paint_executes_clip_filter_and_gradient_mask_layers() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/layout-effects.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
.case{position:absolute;left:0;width:40px;height:20px;background:rgb(255,0,0)}
#clipped{top:0;clip-path:inset(0 20px 0 0)}
#filtered{top:30px;filter:brightness(0)}
#masked{top:60px}
#grouped{left:10px;top:90px;width:20px;box-shadow:8px 0 0 0 blue;outline:2px solid lime;opacity:.5}
#clip-shadow{left:10px;top:130px;width:20px;box-shadow:8px 0 0 0 blue;clip-path:inset(0)}
#closest{top:160px;width:40px;height:40px;clip-path:circle(closest-corner at 10px 10px)}
#farthest{left:50px;top:160px;width:40px;height:40px;clip-path:circle(farthest-corner at 10px 10px)}
</style>`;
document.body.innerHTML = '<div id=clipped class=case></div><div id=filtered class=case></div><div id=masked class=case></div><div id=grouped class=case></div><div id=clip-shadow class=case></div><div id=closest class=case></div><div id=farthest class=case></div>';
document.getElementById('masked').style.setProperty('mask-image','linear-gradient(to right,transparent 0%,transparent 49%,black 51%,black 100%)');
document.getElementById('masked').style.setProperty('mask-repeat','no-repeat');
'installed'
"#,
        )?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 220, 1.0))?
            .expect("effect fixture must retain a layout root");

        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::PushLayer {
                filter: Some(moli_layout::PaintFilter::Brightness(amount)),
                ..
            } if *amount == 0.0
        )));
        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::PushLayer {
                composite: moli_layout::PaintCompositeMode::DestIn,
                ..
            }
        )));
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "mask-image-resource-deferred"
                && diagnostic.code != "filter-url-reference-unsupported"
                && diagnostic.code != "clip-path-url-reference-unsupported"
                && diagnostic.code != "clip-path-corner-radius-unsupported"
        }));

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            &image.rgba[index..index + 4]
        };
        assert_eq!(pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(30, 10), [255, 255, 255, 255]);
        assert_eq!(pixel(10, 40), [0, 0, 0, 255]);
        assert_eq!(pixel(10, 70), [255, 255, 255, 255]);
        assert_eq!(pixel(30, 70), [255, 0, 0, 255]);
        let assert_channel_near = |actual: u8, expected: u8| {
            assert!(
                actual.abs_diff(expected) <= 2,
                "expected channel near {expected}, got {actual}"
            );
        };
        for (actual, expected) in pixel(15, 100).iter().zip([255, 128, 128, 255]) {
            assert_channel_near(*actual, expected);
        }
        for (actual, expected) in pixel(9, 100).iter().zip([128, 255, 128, 255]) {
            assert_channel_near(*actual, expected);
        }
        for (actual, expected) in pixel(36, 100).iter().zip([128, 128, 255, 255]) {
            assert_channel_near(*actual, expected);
        }
        assert_eq!(pixel(15, 140), [255, 0, 0, 255]);
        assert_eq!(pixel(36, 140), [255, 255, 255, 255]);
        assert_eq!(pixel(10, 170), [255, 0, 0, 255]);
        assert_eq!(pixel(30, 190), [255, 255, 255, 255]);
        assert_eq!(pixel(89, 199), [255, 0, 0, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("clip/filter/mask screenshot fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_culling_keeps_outset_ink_that_reaches_the_viewport() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/paint-culling-outsets.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white;height:220px}
.case{position:absolute;width:20px;height:10px;background:transparent}
#shadow{left:5px;top:60px;box-shadow:0 -15px 0 0 rgb(255,0,0)}
#outline{left:40px;top:57px;outline:5px solid rgb(0,128,0);outline-offset:5px}
#filtered{left:75px;top:55px;width:10px;background:rgb(0,0,255);filter:blur(8px)}
#discarded{left:5px;top:170px;background:rgb(255,255,0)}
</style>`;
document.body.innerHTML = '<div id=shadow class=case></div><div id=outline class=case></div><div id=filtered class=case></div><div id=discarded class=case></div>';
'installed'
"#,
        )?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(100, 50, 1.0))?
            .expect("culling fixture must retain a layout root");

        assert!(snapshot.fragments.iter().all(|fragment| {
            !matches!(
                fragment,
                moli_layout::PaintFragment::Fill {
                    brush: moli_layout::PaintBrush::Solid(color),
                    ..
                } if *color == moli_layout::PaintColor::new(1.0, 1.0, 0.0, 1.0)
            )
        }));
        let mut depth = 0usize;
        for fragment in &snapshot.fragments {
            match fragment {
                moli_layout::PaintFragment::PushLayer { .. }
                | moli_layout::PaintFragment::PushClip { .. } => depth += 1,
                moli_layout::PaintFragment::PopLayer => {
                    depth = depth.checked_sub(1).expect("paint stack underflow")
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "culled stacking contexts must remain balanced");

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(pixel(10, 47), [255, 0, 0, 255]);
        assert_eq!(pixel(45, 48), [0, 128, 0, 255]);
        let filtered = pixel(80, 48);
        assert!(
            filtered[2] > filtered[0] && filtered[2] > filtered[1],
            "blurred blue ink should reach the capture from below it: {filtered:?}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("outset paint culling fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_css_gradient_domains_hints_and_interpolation_like_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/css-gradient-domain.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}.case{position:absolute;width:100px;height:100px}
#negative{left:0;top:0;background:linear-gradient(to bottom,rgba(0,0,0,.5) -20%,transparent 30%),white}
#overflow{left:110px;top:0;background:linear-gradient(to bottom,rgb(255,0,0) 80%,rgb(0,0,255) 120%)}
#repeat{left:220px;top:0;background:repeating-linear-gradient(to bottom,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#radial-overflow{left:330px;top:0;background:radial-gradient(circle 50px at 50px 50px,rgb(255,0,0) 80%,rgb(0,0,255) 120%)}
#radial{left:0;top:110px;background:radial-gradient(circle 50px at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#conic{left:110px;top:110px;background:conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#hint{left:220px;top:110px;background:linear-gradient(to right,rgb(255,0,0) 0%,25%,rgb(0,0,255) 100%)}
#conic-overflow{left:330px;top:110px;background:conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) 80%,rgb(0,0,255) 120%)}
#degenerate{left:0;top:220px;background:linear-gradient(to right,rgb(255,0,0) -20%,rgb(0,0,255) -20%)}
#repeat-degenerate{left:110px;top:220px;background:repeating-linear-gradient(to right,rgb(255,0,0) 20%,rgb(0,0,255) 20%)}
#oklab{left:220px;top:220px;background:linear-gradient(to right in oklab,rgb(255,0,0),rgb(0,0,255))}
#repeat-radial{left:330px;top:220px;background:repeating-radial-gradient(circle 50px at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#repeat-conic{left:0;top:330px;background:repeating-conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) -20%,rgb(0,0,255) 30%)}
#p3-linear{left:110px;top:330px;background:linear-gradient(to right in display-p3-linear,rgb(255,0,0),rgb(0,0,255))}
#normal-conic{left:220px;top:330px;background:conic-gradient(from 0deg at 50px 50px,rgb(255,0,0) 0%,rgb(0,0,255) 100%)}
</style>`;
document.body.innerHTML = '<div id=negative class=case></div><div id=overflow class=case></div><div id=repeat class=case></div><div id=radial-overflow class=case></div><div id=radial class=case></div><div id=conic class=case></div><div id=hint class=case></div><div id=conic-overflow class=case></div><div id=degenerate class=case></div><div id=repeat-degenerate class=case></div><div id=oklab class=case></div><div id=repeat-radial class=case></div><div id=repeat-conic class=case></div><div id=p3-linear class=case></div><div id=normal-conic class=case></div>';
'installed'
"#,
        )?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(430, 430, 1.0))?
            .expect("gradient fixture must retain a layout root");

        let brushes = snapshot.fragments.iter().filter_map(|fragment| match fragment {
            moli_layout::PaintFragment::Fill { brush, .. } => Some(brush),
            _ => None,
        });
        let gradients = brushes
            .filter(|brush| !matches!(brush, moli_layout::PaintBrush::Solid(_)))
            .collect::<Vec<_>>();
        assert_eq!(gradients.len(), 15);
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::LinearGradient(gradient)
                if (gradient.start.y + 20.0).abs() <= 0.01
                    && (gradient.end.y - 30.0).abs() <= 0.01
                    && gradient.extend == moli_layout::PaintGradientExtend::Pad
                    && gradient.stops.first().is_some_and(|stop| stop.offset == 0.0)
                    && gradient.stops.last().is_some_and(|stop| stop.offset == 1.0)
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::RadialGradient(gradient)
                if gradient.start_radius == 0.0
                    && (gradient.end_radius - 0.3).abs() <= 0.01
                    && (gradient.transform.coefficients[4] - 50.0).abs() <= 0.01
                    && (gradient.transform.coefficients[5] - 50.0).abs() <= 0.01
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::ConicGradient(gradient)
                if gradient.center == moli_layout::PaintPoint::ZERO
                    && gradient.start_angle_radians == 0.0
                    && (gradient.end_angle_radians - std::f32::consts::TAU).abs() <= 0.01
                    && (gradient.transform.coefficients[4] - 50.0).abs() <= 0.01
                    && (gradient.transform.coefficients[5] - 50.0).abs() <= 0.01
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::LinearGradient(gradient)
                if gradient.stops.len() == 11
        )));
        assert!(gradients.iter().any(|brush| matches!(
            brush,
            moli_layout::PaintBrush::LinearGradient(gradient)
                if gradient.interpolation.color_space
                    == moli_layout::PaintGradientColorSpace::Oklab
        )));

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        let assert_pixel_near = |label: &str, point: (u32, u32), expected: [u8; 4]| {
            let actual = pixel(point.0, point.1);
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 2),
                "{label}: expected {expected:?}, got {actual:?}"
            );
        };
        for (label, point, expected) in [
            ("negative start", (50, 0), [179, 179, 179, 255]),
            ("negative middle", (50, 10), [205, 205, 205, 255]),
            ("negative end", (50, 29), [254, 254, 254, 255]),
            ("negative padded", (50, 50), [255, 255, 255, 255]),
            ("overflow padded", (160, 50), [255, 0, 0, 255]),
            ("overflow start", (160, 80), [251, 0, 3, 255]),
            ("overflow middle", (160, 99), [131, 0, 124, 255]),
            ("repeat first", (270, 0), [150, 0, 104, 255]),
            ("repeat second", (270, 30), [252, 0, 2, 255]),
            ("radial center", (50, 160), [145, 0, 109, 255]),
            ("radial transition", (60, 160), [45, 0, 209, 255]),
            ("radial pad", (70, 160), [0, 0, 255, 255]),
            ("conic top", (160, 120), [151, 0, 103, 255]),
            ("conic right", (200, 160), [24, 0, 230, 255]),
            ("conic bottom", (160, 200), [0, 0, 255, 255]),
            ("hint quarter", (245, 160), [127, 0, 129, 255]),
            ("hint middle", (270, 160), [74, 0, 181, 255]),
            ("hint third quarter", (295, 160), [36, 0, 220, 255]),
            ("degenerate nonrepeat", (50, 270), [0, 0, 255, 255]),
            ("degenerate repeat", (160, 270), [0, 0, 255, 255]),
            ("oklab quarter", (245, 270), [197, 74, 111, 255]),
            ("oklab middle", (270, 270), [139, 83, 163, 255]),
            ("oklab third quarter", (295, 270), [80, 71, 211, 255]),
            ("radial overflow pad", (419, 50), [255, 0, 0, 255]),
            ("radial overflow start", (420, 50), [248, 0, 6, 255]),
            ("radial overflow middle", (429, 50), [134, 0, 121, 255]),
            ("conic overflow pad", (380, 120), [255, 0, 0, 255]),
            ("conic overflow middle", (350, 130), [207, 0, 48, 255]),
            ("repeat radial center", (380, 270), [145, 0, 109, 255]),
            ("repeat radial next period", (400, 270), [198, 0, 56, 255]),
            ("repeat conic top", (50, 340), [152, 0, 103, 255]),
            ("repeat conic right", (90, 380), [24, 0, 230, 255]),
            ("repeat conic bottom", (50, 420), [154, 0, 101, 255]),
            ("display p3 linear quarter", (135, 380), [224, 0, 138, 255]),
            ("display p3 linear middle", (160, 380), [186, 0, 188, 255]),
            ("display p3 linear third quarter", (185, 380), [136, 0, 226, 255]),
            ("normal conic top", (270, 340), [254, 0, 0, 255]),
            ("normal conic right", (310, 380), [190, 0, 64, 255]),
            ("normal conic bottom", (270, 420), [128, 0, 127, 255]),
            ("normal conic left", (230, 380), [64, 0, 190, 255]),
        ] {
            assert_pixel_near(label, point, expected);
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSS gradient domain fixture should match Chromium");
}

#[tokio::test]
async fn child_document_layout_discovers_cssom_image_with_child_resource_owner() {
    run_page_vm_async_test(async move {
        let image_url =
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+3MxZ5wAAAABJRU5ErkJggg==";
        let hidden_image_url =
            "data:image/webp;base64,UklGRhwAAABXRUJQVlA4TA8AAAAvAUAAAAcQ/Y/+ByKi/wEA";
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/child-css-image-discovery.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        page_vm.vm_mut().eval(
            r#"
const frame = document.createElement('iframe');
document.body.appendChild(frame);
const child = frame.contentDocument;
child.body.innerHTML = '<div id=target style="width:20px;height:20px"></div><div id=hidden></div>';
globalThis.__childImageSheet = new frame.contentWindow.CSSStyleSheet();
globalThis.__childImageSheet.replaceSync('#target { background: red; }');
child.adoptedStyleSheets = [globalThis.__childImageSheet];
'installed'
"#,
        )?;

        let viewport = moli_layout::PaintViewport::new(80, 60, 1.0);
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("initial child document paint");
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("clean child document paint");

        page_vm.vm_mut().eval(&format!(
            "globalThis.__childImageSheet.replaceSync('#target {{ background-image: url({}); }} #hidden {{ display:none; background-image:url({}); }}')",
            serde_json::to_string(image_url)?,
            serde_json::to_string(hidden_image_url)?,
        ))?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("mutated child document paint");
        assert!(
            page_vm
                .vm()
                .css_image_resource_observability_for_test()
                .4
                .iter()
                .any(|url| url == image_url),
            "the recursive child layout must bind its computed URL to the child Document",
        );
        assert!(
            !page_vm
                .vm()
                .css_image_resource_observability_for_test()
                .4
                .iter()
                .any(|url| url == hidden_image_url),
            "a display:none subtree must not fetch a CSS image merely because it exists in the DOM",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("child CSS image discovery fixture should run");
}

#[tokio::test]
async fn css_image_discovery_runs_inside_each_fresh_paint_layout() {
    run_page_vm_async_test(async move {
        // A lossless 2x2 red WebP. Keep the HTTP fixture encoded so this
        // product path exercises metadata probing and bounded WebP decode.
        let raster_url = "data:image/webp;base64,UklGRhwAAABXRUJQVlA4TA8AAAAvAUAAAAcQ/Y/+ByKi/wEA"
            .to_owned();
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/css-url-image-layers.html")?,
        );
        page_vm
            .vm_mut()
            .set_layout_policy(crate::real_layout_test_policy());
        let local_executor = page_vm.local_executor.clone();

        local_executor.run(async move {
            let vector_url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="blue"/></svg>"#,
            )
        );
            let mask_url = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect x="20" width="20" height="20" fill="white"/></svg>"#,
            )
        );
            let pseudo_url = format!(
                "data:image/svg+xml;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(
                    br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="green"/></svg>"#,
                )
            );
            let css = format!(
                r#"
html,body{{margin:0;padding:0;background:white}}
.case{{position:absolute;left:0;width:40px;height:20px}}
#raster{{top:0;background-image:url("{raster_url}");background-size:10px 10px;background-repeat:repeat-x}}
#vector{{top:30px;background-image:url("{vector_url}");background-size:20px auto;background-repeat:repeat-x}}
#masked{{top:60px;background:red;mask-image:url("{mask_url}");mask-size:40px 20px;mask-repeat:no-repeat}}
#inline-line{{position:absolute;left:0;top:90px;font:20px/20px sans-serif}}
#inline-vector{{padding:0 10px;color:transparent;background-image:url("{vector_url}");background-size:20px 20px;background-repeat:repeat-x}}
#pseudo{{position:absolute;left:50px;top:0;width:20px;height:20px}}
#pseudo::before{{content:"";display:block;width:20px;height:20px;background-image:url("{pseudo_url}");background-size:20px 20px}}
"#
            );
            page_vm.vm_mut().eval(&format!(
                "document.head.innerHTML='<style id=fixture></style>';document.getElementById('fixture').textContent={};document.body.innerHTML='<div id=raster class=case></div><div id=vector class=case></div><div id=masked class=case></div><div id=inline-line><span id=inline-vector>X</span></div><div id=pseudo></div>';'installed'",
                serde_json::to_string(&css)?,
            ))?;
            page_vm.vm_mut().sync_live_document_style_sources();

            // The first paint demand discovers computed CSSOM URLs and queues
            // the bounded decoders. CSS images have no DOM load-event task; a
            // later screencast/screenshot samples immutable ready resources.
            let passes_before = page_vm.vm().layout_pass_observability_for_test().1;
            page_vm
                .vm_mut()
                .paint_layout_snapshot(
                    moli_layout::PaintViewport::new(80, 120, 1.0),
                    moli_layout::LayoutFlushReason::Screencast,
                )?
                .expect("CSS image fixture must retain a layout root");
            let passes_after_first = page_vm.vm().layout_pass_observability_for_test().1;
            assert_eq!(passes_after_first, passes_before + 1);
            let urls = [&raster_url, &vector_url, &mask_url, &pseudo_url];
            let completion_notify = page_vm.vm().css_image_completion_notify_for_test();
            let all_ready = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    // Register before checking state so a completion between
                    // the check and await cannot be lost.
                    let notified = completion_notify.notified();
                    if urls.iter().all(|url| {
                        page_vm
                            .vm()
                            .css_image_resource_is_ready_for_test(url)
                    }) {
                        break;
                    }
                    notified.await;
                }
            })
            .await
            .is_ok();
            assert!(
                all_ready,
                "bounded local CSS image decodes must complete: {:?}",
                page_vm.vm().css_image_resource_observability_for_test()
            );

            let snapshot = page_vm
                .vm_mut()
                .paint_layout_snapshot(
                    moli_layout::PaintViewport::new(80, 120, 1.0),
                    moli_layout::LayoutFlushReason::Screencast,
                )?
                .expect("ready CSS image fixture must retain a layout root");
            let passes_after_ready = page_vm.vm().layout_pass_observability_for_test().1;
            assert_eq!(
                passes_after_ready,
                passes_after_first + 1,
                "each explicit paint demand must still execute a fresh layout",
            );
            assert_eq!(snapshot.images.len(), 1);
            assert_eq!(snapshot.svg_images.len(), 4);
            assert!(snapshot.fragments.iter().any(|fragment| {
                matches!(fragment, moli_layout::PaintFragment::Image(_))
            }));
            assert!(snapshot.fragments.iter().any(|fragment| {
                matches!(fragment, moli_layout::PaintFragment::SvgImage(_))
            }));
            assert!(snapshot.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "background-image-resource-unavailable"
                    && diagnostic.code != "mask-image-resource-unavailable"
                    && diagnostic.code != "background-image-type-unsupported"
                    && diagnostic.code != "mask-image-type-unsupported"
            }));

            let image = moli_paint::raster_snapshot(&snapshot)?;
            let pixel = |x: u32, y: u32| {
                let index = ((y * image.width + x) * 4) as usize;
                &image.rgba[index..index + 4]
            };
            assert_eq!(pixel(5, 5), [255, 0, 0, 255]);
            assert_eq!(pixel(55, 5), [0, 128, 0, 255]);

            page_vm
                .vm_mut()
                .eval("document.getElementById('raster').style.backgroundPosition = '1px 0px'")?;
            let resources_before_style_only_paint =
                page_vm.vm().css_image_resource_observability_for_test();
            page_vm
                .vm_mut()
                .paint_layout_snapshot(
                    moli_layout::PaintViewport::new(80, 120, 1.0),
                    moli_layout::LayoutFlushReason::Screencast,
                )?
                .expect("style-mutated CSS image fixture must retain a layout root");
            assert_eq!(
                page_vm.vm().css_image_resource_observability_for_test(),
                resources_before_style_only_paint,
                "layout-local discovery must deduplicate already admitted Document/URL resources",
            );
            assert_eq!(pixel(35, 5), [255, 0, 0, 255]);
            assert_eq!(pixel(5, 15), [255, 255, 255, 255]);
            assert_eq!(pixel(5, 35), [0, 0, 255, 255]);
            assert_eq!(pixel(25, 35), [0, 0, 255, 255]);
            assert_eq!(pixel(5, 65), [255, 255, 255, 255]);
            assert_eq!(pixel(30, 65), [255, 0, 0, 255]);
            assert_eq!(pixel(5, 95), [0, 0, 255, 255]);
            Ok::<_, anyhow::Error>(())
        })
        .await
    })
    .await
    .expect("WebP and SVG CSS URL image-layer fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_table_cell_dimension_hints_and_avatar_columns() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-dimensions.html")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let fixture = include_str!("../../../../../tests/fixtures/table-cell-dimensions.html");
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(fixture)?,
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        for phase in 0..3 {
            page.vm_mut()
                .eval(&format!("setTableCellDimensionPhase({phase})"))?;
            let snapshot = page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 1100, 1.0))?
                .expect("table-cell dimension fixture must retain a layout root");
            let checks: serde_json::Value = serde_json::from_str(
                &page
                    .vm_mut()
                    .eval("JSON.stringify(collectTableCellDimensionChecks())")?,
            )?;
            let checks = checks.as_array().expect("table-cell dimension checks");
            assert_eq!(checks.len(), 43);
            let failures: Vec<_> = checks
                .iter()
                .filter(|check| check["actual"] != check["expected"])
                .collect();
            assert!(failures.is_empty(), "phase {phase}: {failures:#?}");

            let image = moli_paint::raster_snapshot(&snapshot)?;
            for check in checks {
                let Some(point) = check["pixel"].as_array() else {
                    continue;
                };
                let x = point[0].as_f64().expect("avatar pixel x") as u32;
                let y = point[1].as_f64().expect("avatar pixel y") as u32;
                let offset = ((y * image.width + x) * 4) as usize;
                assert_eq!(
                    &image.rgba[offset..offset + 4],
                    [31, 127, 63, 255],
                    "phase {phase}, avatar {} at ({x}, {y})",
                    check["id"],
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table-cell dimension hints should preserve avatar alignment");
}

#[tokio::test(flavor = "current_thread")]
async fn table_cell_absolute_descendants_use_final_geometry() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-absolute.html")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(r#"
document.head.innerHTML = `<style>
body { margin:0 } table { width:200px;border-spacing:0;table-layout:fixed }
td { padding:0;font-size:0;line-height:0;vertical-align:top }
</style>`;
document.body.innerHTML = `<table><tr>
<td id=cell style="position:relative">
  <div style="height:10px"></div><div id=inflow style="height:50%"></div>
  <div id=percent style="position:absolute;top:0;left:0;height:50%;width:10px"></div>
  <div id=fill style="position:absolute;inset:0"></div>
</td><td><div id=tall style="height:100px"></div></td>
</tr></table>`;
"#)?;
        for height in [100, 160, 100] {
            page.vm_mut().eval(&format!("document.getElementById('tall').style.height='{height}px'"))?;
            for _ in 0..2 {
                page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?.expect("table root");
                let actual = page.vm_mut().eval(r#"
['cell','inflow','percent','fill'].map(id => document.getElementById(id).getBoundingClientRect().height).join('|')
"#)?;
                assert_eq!(actual, format!("{height}|0|{}|{height}", height / 2),
                    "absolute descendants use the final cell box; normal-flow percentages remain indefinite");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("table cell absolute layout should match Chromium");
}

#[tokio::test(flavor = "current_thread")]
async fn table_cell_nested_percentage_baseline_reaches_parent_measurement() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-nested-baseline.html")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(r#"
document.head.innerHTML = `<style>
body { margin:0 } table { width:200px;border-spacing:0;table-layout:fixed }
td { padding:0;font-size:0;line-height:0;vertical-align:baseline }
</style>`;
document.body.innerHTML = `<table id=outer><tr><td>
<table id=inner style="width:100px"><tr>
  <td id=cell style="height:100px"><div id=percent style="height:50%;width:10px"></div></td>
  <td><div id=peer style="height:20px;width:10px"></div></td>
</tr></table>
</td><td><span id=reference style="display:inline-block;height:50px;width:10px"></span></td></tr></table>`;
"#)?;
        for height in [100, 160, 100] {
            page.vm_mut().eval(&format!("document.getElementById('cell').style.height='{height}px'"))?;
            for _ in 0..2 {
                page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?.expect("nested table root");
                let actual = page.vm_mut().eval(r#"(() => {
const rect = id => document.getElementById(id).getBoundingClientRect();
const outer = rect('outer');
return [outer.height,rect('inner').height,rect('percent').height,
        rect('peer').y-outer.y,rect('reference').y-outer.y].join('|');
})()"#)?;
                assert_eq!(actual, format!("{height}|{height}|{}|{}|{}", height / 2, height / 2 - 20, height / 2 - 50),
                    "outer row measurement must use the nested table's final percentage-dependent baseline");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("nested percentage baseline should match Chromium");
}

#[tokio::test(flavor = "current_thread")]
async fn table_cell_synthetic_baselines_ignore_positioned_overflow() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-flow-baseline.html")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(r#"
document.head.innerHTML = `<style>
body { margin:0 } table { width:200px;border-spacing:0;table-layout:fixed }
td { padding:0;font-size:0;line-height:0;vertical-align:baseline }
</style>`;
document.body.innerHTML = `<table><tr>
<td id=cell style="height:100px;position:relative">
  <div id=content style="height:10px;width:10px"></div>
  <div id=overlay style="position:absolute;inset:0"></div>
</td><td><span id=peer style="display:inline-block;height:20px;width:10px"></span></td>
</tr></table>`;
"#)?;
        for relative in [false, true, false] {
            for height in [100, 160, 100] {
                page.vm_mut().eval(&format!(r#"
document.getElementById('cell').style.height='{height}px';
document.getElementById('overlay').style.display='{}';
document.getElementById('content').style.cssText='height:10px;width:10px;{}';
"#, if relative { "none" } else { "block" }, if relative { "position:relative;top:50%" } else { "" }))?;
                for _ in 0..2 {
                    page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?.expect("table root");
                    let actual = page.vm_mut().eval(r#"
['cell','peer','content','overlay'].map(id => {
  const r=document.getElementById(id).getBoundingClientRect(); return [r.y,r.height].join(',');
}).join('|')
"#)?;
                    let content_y = 10 + if relative { height / 2 } else { 0 };
                    let overlay_height = if relative { 0 } else { height };
                    assert_eq!(actual, format!("0,{height}|0,20|{content_y},10|0,{overlay_height}"),
                        "relative={relative}: synthetic baselines use normal-flow geometry, while positioned descendants retain final cell geometry");
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("positioned overflow must not move adjacent cell baselines");
}

#[tokio::test(flavor = "current_thread")]
async fn table_cell_vertical_alignment_includes_floats() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-float-alignment.html")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        page.vm_mut().eval(r#"
document.head.innerHTML = `<style>
body { margin:0 } table { width:200px;border-spacing:0;table-layout:fixed }
td { padding:0;font-size:0;line-height:0 }
</style>`;
document.body.innerHTML = `<table><tr>
<td id=cell style="position:relative">
  <div id=float style="float:left;width:10px;height:30px"></div>
  <div id=block style="margin-left:15px;width:10px;height:20px"></div>
  <div id=overlay style="position:absolute;inset:0"></div>
</td><td><div id=tall></div></td></tr></table>`;
"#)?;
        for align in ["middle", "bottom"] {
            for mixed in [false, true] {
                for padding in [0, 5] {
                    for height in [100, 160, 100] {
                        page.vm_mut().eval(&format!(r#"
document.getElementById('cell').style.cssText='position:relative;vertical-align:{align};padding:{padding}px';
document.getElementById('block').style.display='{}';
document.getElementById('tall').style.height='{height}px';
"#, if mixed { "block" } else { "none" }))?;
                        for _ in 0..2 {
                            page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?.expect("table root");
                            let actual = page.vm_mut().eval(r#"
['cell','float','block','overlay'].map(id => {
  const r=document.getElementById(id).getBoundingClientRect(); return [r.y,r.height].join(',');
}).join('|')
"#)?;
                            let free = height - 2 * padding - 30;
                            let y = padding + if align == "middle" { free / 2 } else { free };
                            let block = if mixed { format!("{y},20") } else { "0,0".to_owned() };
                            assert_eq!(actual, format!("0,{height}|{y},30|{block}|0,{height}"),
                                "{align}, mixed={mixed}, padding={padding}: align floats and normal flow together without shifting the absolute overlay");
                        }
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("table-cell vertical alignment must include floats");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_table_row_heights_match_chromium() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/table-row-heights.html")?,
        );
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let fixture = include_str!("../../../../../tests/fixtures/table-row-heights.html");
        page.vm_mut().eval(&format!("document.open();document.write({});document.close()", serde_json::to_string(fixture)?))?;
        page.vm_mut().prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        page.vm_mut().eval("if (!cases.length) buildTableRowHeightCases()")?;
        let expected: serde_json::Value = serde_json::from_str(include_str!("../../../../../tests/fixtures/table-row-heights.chromium.json"))?;
        let mut failures = Vec::new();
        for phase in 0..3 {
            page.vm_mut().eval(&format!("setTableRowHeightPhase({phase})"))?;
            // Both initial layout and a repeated read must agree with the oracle.
            for read in 0..2 {
                page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?.expect("table layout root");
                let actual: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(collectTableRowHeights())")?)?;
                let actual = actual.as_array().unwrap();
                let expected_cases = expected["phases"][phase].as_array().unwrap();
                assert_eq!(actual.len(), expected_cases.len());
                for (actual, expected) in actual.iter().zip(expected_cases) {
                    assert_eq!(actual["name"], expected["name"]);
                    let name = actual["name"].as_str().unwrap();
                    let actual_rects = actual["rects"].as_array().unwrap();
                    let expected_rects = expected["rects"].as_array().unwrap();
                    assert_eq!(actual_rects.len(), expected_rects.len());
                    for (index, (actual, expected)) in actual_rects.iter().zip(expected_rects).enumerate() {
                        if (0..4).any(|axis| (actual[axis].as_f64().unwrap() - expected[axis].as_f64().unwrap()).abs() > 0.05) {
                            failures.push(format!("phase {phase}, read {read}, {name}, rect {index}: {actual} != {expected}"));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{} table geometry differences:\n{}", failures.len(), failures.join("\n"));
        // Check that final percentage-dependent baseline alignment reaches
        // paint as well as CSSOM, including a subsequent group-height change.
        page.vm_mut().eval(r#"
            for (const {name,owner} of cases) owner.style.display = name === 'percent-baseline' ? 'block' : 'none';
            const paintedCase = cases.find(c => c.name === 'percent-baseline').owner;
            paintedCase.querySelectorAll('td > div')[0].style.background = 'rgb(255,0,0)';
            paintedCase.querySelectorAll('td > div')[1].style.background = 'rgb(0,255,0)';
        "#)?;
        for height in [120u32, 160] {
            page.vm_mut().eval(&format!("paintedCase.querySelector('tbody').style.height = '{height}px'"))?;
            let snapshot = page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 200, 1.0))?.expect("painted table root");
            let image = moli_paint::raster_snapshot(&snapshot)?;
            for (x, y, color) in [(5, height / 4, [255, 0, 0, 255]), (105, height / 2 - 10, [0, 255, 0, 255])] {
                let offset = ((y * image.width + x) * 4) as usize;
                assert_eq!(&image.rgba[offset..offset + 4], color, "group height {height}, pixel ({x}, {y})");
            }
        }
        page.vm_mut().eval(r#"
            paintedCase.style.display = 'none';
            const borderedCase = cases.find(c => c.name === 'empty-groups-collapsed').owner;
            borderedCase.style.display = 'block';
            borderedCase.querySelector('thead').style.height = '80px';
            borderedCase.querySelector('tfoot').style.height = '80px';
        "#)?;
        let snapshot = page.vm_mut().screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 200, 1.0))?.expect("collapsed table root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        for (x, y) in [(100, 1), (100, 187), (1, 100)] {
            let offset = ((y * image.width + x) * 4) as usize;
            assert_eq!(&image.rgba[offset..offset + 4], [0, 0, 255, 255], "collapsed border at ({x}, {y})");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("table row heights should match Chromium");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_recascades_table_part_dimension_hints() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-part-dimensions.html")?,
        );
        page.vm_mut()
            .set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
        let fixture = include_str!("../../../../../tests/fixtures/table-part-dimensions.html");
        page.vm_mut().eval(&format!(
            "document.open();document.write({});document.close()",
            serde_json::to_string(fixture)?,
        ))?;
        page.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        for phase in 0..4 {
            page.vm_mut()
                .eval(&format!("setTablePartDimensionPhase({phase})"))?;
            page.vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
                .expect("table-part dimension fixture must retain a layout root");
            let checks: serde_json::Value = serde_json::from_str(
                &page
                    .vm_mut()
                    .eval("JSON.stringify(collectTablePartDimensionChecks())")?,
            )?;
            let checks = checks.as_array().expect("table-part dimension checks");
            assert_eq!(checks.len(), 71);
            let failures: Vec<_> = checks
                .iter()
                .filter(|check| check["actual"] != check["expected"])
                .collect();
            assert!(failures.is_empty(), "phase {phase}: {failures:#?}");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table-part dimensions should recascade and resize tables");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_recascades_nearest_table_cell_presentation_style() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-presentation-style.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
table{border-spacing:0}
td{font-size:0;border:0}
.content{width:10px;height:10px}
#author-cell{padding:7px}
</style>`;
document.body.innerHTML = `
<table><tr><td id=default-cell><div class=content></div></td></tr></table>
<table id=legacy-table cellpadding=4><tr><td id=legacy-cell><div class=content></div></td></tr></table>
<table cellpadding=3><tr><td><table cellpadding=5><tr><td id=nested-cell><div class=content></div></td></tr></table></td></tr></table>
<table id=author-table cellpadding=4><tr><td id=author-cell><div class=content></div></td></tr></table>`;
const orphan = document.createElement('td');
orphan.id = 'orphan-cell';
orphan.innerHTML = '<div class=content></div>';
document.body.append(orphan);
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let viewport = moli_layout::PaintViewport::new(200, 200, 1.0);
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("table-cell fixture must retain a layout root");
        let read_cells = r#"(() => {
const read = (id, includeGeometry = true) => {
  const cell = document.getElementById(id);
  const style = getComputedStyle(cell);
  const values = [style.paddingTop,style.paddingRight,style.paddingBottom,style.paddingLeft];
  if (includeGeometry) values.push(cell.offsetWidth,cell.offsetHeight);
  return values.join(',');
};
return [
  read('default-cell'),
  read('legacy-cell'),
  read('nested-cell'),
  read('author-cell'),
  read('orphan-cell', false),
].join('|');
})()
"#;
        assert_eq!(
            page_vm.vm_mut().eval(read_cells)?,
            "1px,1px,1px,1px,12,12|4px,4px,4px,4px,18,18|5px,5px,5px,5px,20,20|7px,7px,7px,7px,24,24|1px,1px,1px,1px",
            "table cells must use the nearest table's legacy padding while author CSS and the orphan-cell UA fallback remain authoritative",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('legacy-table').setAttribute('cellpadding','8')")?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("mutated table-cell fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(read_cells)?,
            "1px,1px,1px,1px,12,12|8px,8px,8px,8px,26,26|5px,5px,5px,5px,20,20|7px,7px,7px,7px,24,24|1px,1px,1px,1px",
            "cellpadding mutation must recascade descendant cells without crossing a nested table boundary",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('legacy-table').setAttribute('cellpadding','0')")?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("zero-cellpadding fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(read_cells)?,
            "1px,1px,1px,1px,12,12|0px,0px,0px,0px,10,10|5px,5px,5px,5px,20,20|7px,7px,7px,7px,24,24|1px,1px,1px,1px",
            "cellpadding=0 must remove the shared presentational declaration",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('legacy-table').removeAttribute('cellpadding')")?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("default-cellpadding fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(read_cells)?,
            "1px,1px,1px,1px,12,12|1px,1px,1px,1px,12,12|5px,5px,5px,5px,20,20|7px,7px,7px,7px,24,24|1px,1px,1px,1px",
            "removing cellpadding must restore the historical 1px default",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table-cell presentational-style fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_applies_legacy_table_width_spacing_and_colors() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-presentation-style.html")?,
        );
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 200,
            inner_height: 100,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))?;
        page_vm.vm_mut().eval(
            r##"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
td{padding:0}
.block{width:10px;height:10px}
</style>`;
document.body.innerHTML = `
<table id=legacy width="85%" bgcolor="#f6f6ef" cellspacing="0" cellpadding="0">
  <tr><td id=header bgcolor="#ff6600" valign="top" align="right"><div class=block></div></td></tr>
  <tr><td id=body-cell><div class=block></div></td></tr>
</table>`;
'installed'
"##,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        let viewport = moli_layout::PaintViewport::new(200, 100, 1.0);
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("legacy table fixture must retain a layout root");
        let read_table = r#"(() => {
const table = document.getElementById('legacy');
const tableStyle = getComputedStyle(table);
const cellStyle = getComputedStyle(document.getElementById('header'));
return [
  tableStyle.borderSpacing,
  tableStyle.backgroundColor,
  cellStyle.backgroundColor,
  cellStyle.textAlign,
  table.offsetWidth,
].join('|');
})()"#;
        assert_eq!(
            page_vm.vm_mut().eval(read_table)?,
            "0px|rgb(246, 246, 239)|rgb(255, 102, 0)|-moz-right|170",
            "legacy table attributes must participate in the presentation-hint cascade",
        );

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            &image.rgba[index..index + 4]
        };
        assert_eq!(pixel(160, 5), [255, 102, 0, 255]);
        assert_eq!(pixel(160, 15), [246, 246, 239, 255]);
        assert_eq!(pixel(180, 15), [255, 255, 255, 255]);

        page_vm.vm_mut().eval(
            r#"
const table = document.getElementById('legacy');
table.setAttribute('width', '50%');
table.setAttribute('cellspacing', '5');
table.setAttribute('bgcolor', '#010203');
const header = document.getElementById('header');
header.setAttribute('bgcolor', '#040506');
header.setAttribute('valign', 'bottom');
header.setAttribute('align', 'left');
"#,
        )?;
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(viewport)?
            .expect("mutated legacy table fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(read_table)?,
            "5px|rgb(1, 2, 3)|rgb(4, 5, 6)|-moz-left|100",
            "legacy table attribute mutations must recascade their presentation hints",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("legacy table presentational-style fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_aligns_legacy_table_block_descendants() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-descendant-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r##"
document.head.innerHTML = `<style>
html,body{margin:0}
table{border-spacing:0}
td{padding:0;width:100px;height:10px}
.box{width:10px;height:10px}
</style>`;
document.body.innerHTML = `
<table>
  <tr><td id=right align=right><div id=right-block class=box></div></td></tr>
  <tr><td id=left align=left style="direction:rtl"><div id=left-block class=box></div></td></tr>
</table>`;
'installed'
"##,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 100, 1.0))?
            .expect("legacy descendant-alignment fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
const right = document.getElementById('right').getBoundingClientRect();
const rightBlock = document.getElementById('right-block').getBoundingClientRect();
const left = document.getElementById('left').getBoundingClientRect();
const leftBlock = document.getElementById('left-block').getBoundingClientRect();
return [
  getComputedStyle(document.getElementById('right')).textAlign,
  right.width,
  rightBlock.x - right.x,
  getComputedStyle(document.getElementById('left')).textAlign,
  left.width,
  leftBlock.x - left.x,
].join('|');
})()"#,
            )?,
            "-moz-right|100|90|-moz-left|100|0",
            "legacy table alignment must affect fixed-width block descendants in both directions",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("legacy table descendant-alignment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_centers_table_headers_only_without_inherited_alignment() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-header-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r##"
document.head.innerHTML = `<style>
html,body{margin:0}
table{border-spacing:0}
th{padding:0;width:100px;height:10px;font-size:0}
.box{display:inline-block;width:10px;height:10px}
</style>`;
document.body.innerHTML = `
<table><tr><th id=default-header><span id=centered class=box></span></th></tr></table>
<table><tr align=right><th id=inherited-header><span id=right-aligned class=box></span></th></tr></table>`;
'installed'
"##,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 100, 1.0))?
            .expect("table-header alignment fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
const centered = document.getElementById('centered').getBoundingClientRect();
const defaultHeader = document.getElementById('default-header').getBoundingClientRect();
const rightAligned = document.getElementById('right-aligned').getBoundingClientRect();
const inheritedHeader = document.getElementById('inherited-header').getBoundingClientRect();
return [
  getComputedStyle(document.getElementById('default-header')).textAlign,
  centered.x - defaultHeader.x,
  getComputedStyle(document.getElementById('inherited-header')).textAlign,
  rightAligned.x - inheritedHeader.x,
].join('|');
})()"#,
            )?,
            "center|45|-moz-right|90",
            "table headers must center by default but inherit an explicit row alignment",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table-header alignment fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_nested_table_cell_height_and_vertical_alignment() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/nested-table-cell-layout.html")?,
        );
        page_vm.vm_mut().eval(
            r##"
document.head.innerHTML = `<style>
html,body{margin:0}
table{border-spacing:0}
td{padding:0}
#outer{width:300px;font:13.3333px Verdana,Arial,sans-serif}
.arrow{width:10px;height:10px;margin:3px 2px 6px;background:#777}
.box{width:10px;height:10px;background:red}
</style>`;
document.body.innerHTML = `
<table id=outer><tr><td>
  <table id=inner><tr id=row>
    <td valign=top><span>1.</span></td>
    <td valign=top><center><a><div id=arrow class=arrow></div></a></center></td>
    <td valign=middle><div id=middle-content class=box></div></td>
    <td valign=bottom><div id=bottom-content class=box></div></td>
  </tr></table>
</td></tr></table>`;
'installed'
"##,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 100, 1.0))?
            .expect("nested table fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
const row = document.getElementById('row').getBoundingClientRect();
const offset = id => document.getElementById(id).getBoundingClientRect().y - row.y;
return [row.height, offset('arrow'), offset('middle-content'), offset('bottom-content')].join('|');
})()"#,
            )?,
            "19|3|4.5|9",
            "nested table measurement must retain collapsed child margins and align cell content",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested table-cell layout fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paints_fresh_inline_svg_resources_with_computed_current_color() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-svg-replaced.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
#icon{position:absolute;left:0;top:0;color:red}
#icon.blue{color:blue}
#ratio{position:absolute;left:0;top:30px;color:green}
#feishu-time{position:absolute;left:60px;top:0;color:#646a73;font-size:16px}
#feishu-time.css-width{width:24px}
</style>`;
document.body.innerHTML = `
<svg id="icon" width="40" height="20" viewBox="0 0 4 2">
  <rect id="shape" width="2" height="2" fill="currentColor"></rect>
</svg>
<svg id="feishu-time" width="1em" height="1em" viewBox="0 0 24 24" data-icon="TimeOutlined">
  <rect width="24" height="24" fill="currentColor"></rect>
</svg>
<svg id="ratio" viewBox="0 0 1 1">
  <rect width="1" height="1" fill="currentColor"></rect>
</svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        assert_eq!(
            page_vm.vm_mut().eval(
                "[getComputedStyle(document.getElementById('feishu-time')).width,getComputedStyle(document.getElementById('feishu-time')).height].join('|')",
            )?,
            "16px|16px",
            "SVG presentation attributes must resolve 1em through the element's computed font size like Chromium",
        );
        // Before layout exists, CSSOM exposes the live style-only values.
        // Keep cascade precedence separate from sampled used-size semantics.
        assert_eq!(
            page_vm.vm_mut().eval(
                "const icon=document.getElementById('feishu-time');icon.setAttribute('width','2em');const fromAttribute=getComputedStyle(icon).width;icon.classList.add('css-width');const fromCss=getComputedStyle(icon).width;icon.classList.remove('css-width');const restored=getComputedStyle(icon).width;icon.setAttribute('width','1em');[fromAttribute,fromCss,restored].join('|')",
            )?,
            "32px|24px|32px",
            "mutated presentation attributes must recascade and author CSS must override them",
        );

        let first = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 190, 1.0))?
            .expect("inline SVG fixture must retain a layout root");
        assert_eq!(first.svg_images.len(), 3);
        assert_eq!(
            first
                .fragments
                .iter()
                .filter(|fragment| matches!(fragment, moli_layout::PaintFragment::SvgImage(_)))
                .count(),
            3
        );
        first
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::SvgImage(image) => Some(image.destination),
                _ => None,
            })
            .find(|destination| {
                (destination.width - 16.0).abs() <= 0.01
                    && (destination.height - 16.0).abs() <= 0.01
            })
            .expect("the Feishu-style 1em SVG must paint into a 16x16 destination");
        assert!(first.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "replaced-content-placeholder"
                && diagnostic.code != "svg-resource-unsupported"
        }));

        let first_raster = moli_paint::raster_snapshot(&first)?;
        let first_pixel = |x: u32, y: u32| {
            let index = ((y * first_raster.width + x) * 4) as usize;
            &first_raster.rgba[index..index + 4]
        };
        // The authored subtree has no serialized stylesheet. Red and green
        // therefore prove that the root's resolved Stylo `color` reached
        // usvg's inherited `currentColor` rather than falling back to black.
        assert_eq!(first_pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(first_pixel(30, 5), [255, 255, 255, 255]);
        assert_eq!(first_pixel(65, 5), [100, 106, 115, 255]);
        assert_eq!(first_pixel(80, 5), [255, 255, 255, 255]);
        assert_eq!(first_pixel(140, 40), [0, 128, 0, 255]);
        // The viewBox-only square stretches to the containing block's width
        // and transfers that width through its 1:1 ratio, like Chromium.
        assert_eq!(first_pixel(170, 40), [0, 128, 0, 255]);

        page_vm.vm_mut().eval(
            "document.getElementById('icon').classList.add('blue');document.getElementById('shape').setAttribute('x','2');document.getElementById('feishu-time').setAttribute('width','2em');'mutated'",
        )?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "getComputedStyle(document.getElementById('feishu-time')).width",
            )?,
            "16px",
            "CSSOM must reuse the existing used size until an explicit layout refresh",
        );
        let second = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 190, 1.0))?
            .expect("mutated inline SVG fixture must retain a layout root");
        assert_eq!(
            page_vm.vm_mut().eval(
                "getComputedStyle(document.getElementById('feishu-time')).width",
            )?,
            "32px",
            "fresh layout must sample the mutated style rather than the old CSSOM used size",
        );
        assert_eq!(second.svg_images.len(), 3);
        assert!(second.fragments.iter().any(|fragment| {
            matches!(
                fragment,
                moli_layout::PaintFragment::SvgImage(image)
                    if (image.destination.width - 32.0).abs() <= 0.01
                        && (image.destination.height - 16.0).abs() <= 0.01
            )
        }));
        assert!(first.svg_images.iter().all(|old| {
            second
                .svg_images
                .iter()
                .all(|fresh| !std::sync::Arc::ptr_eq(&old.image, &fresh.image))
        }));

        let second_raster = moli_paint::raster_snapshot(&second)?;
        let second_pixel = |x: u32, y: u32| {
            let index = ((y * second_raster.width + x) * 4) as usize;
            &second_raster.rgba[index..index + 4]
        };
        assert_eq!(second_pixel(5, 5), [255, 255, 255, 255]);
        assert_eq!(second_pixel(30, 5), [0, 0, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("fresh inline SVG replaced-resource fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn inline_text_clip_membership_survives_visual_edge_reordering() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/bidi-inline-text-clip.html")?,
        );
        // Chromium 145.0.7632.116 gives equal, visible ink in all four rows.
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}.case{font:32px/40px monospace;height:40px}
em{font-style:normal}.clip{padding-left:20px;background-image:linear-gradient(90deg,red,blue);background-repeat:no-repeat;background-clip:text;-webkit-text-fill-color:transparent}
.sibling{-webkit-text-fill-color:transparent}
</style>`;
document.body.innerHTML = '<div class=case><span class=clip style="direction:ltr">x</span><span class=sibling>W</span></div><div class=case><span class=clip style="direction:rtl">x</span><span class=sibling>W</span></div><div class=case><span class=clip style="direction:ltr"><em>x</em></span><span class=sibling>W</span></div><div class=case><span class=clip style="direction:rtl"><em>x</em></span><span class=sibling>W</span></div>';
'installed'
"#,
        )?;
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 160, 1.0))?
            .expect("text clip fixture must retain a layout root");
        let image = moli_paint::raster_snapshot(&snapshot)?;
        let colored_ink = [0, 40, 80, 120].map(|top| {
            (top..top + 40)
                .flat_map(|y| (0..image.width).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let offset = ((y * image.width + x) * 4) as usize;
                    let pixel = &image.rgba[offset..offset + 4];
                    pixel[3] == 255 && pixel[0].max(pixel[2]).saturating_sub(pixel[1]) > 20
                })
                .count()
        });
        assert!(colored_ink[0] > 50, "the control must paint visible text ink");
        assert_eq!(
            colored_ink,
            [colored_ink[0]; 4],
            "CSS direction and nested styles must preserve the text mask's membership"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("bidi text clip test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_paints_background_clip_text_with_transparent_webkit_fill() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/background-clip-text.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0;background:white}
#gradient{display:inline-block;padding:12px;font:32px/40px sans-serif;color:black;background-image:linear-gradient(90deg,rgb(255,0,0),rgb(0,0,255));background-repeat:no-repeat;background-clip:text;-webkit-text-fill-color:transparent}
#inline-line{font:32px/40px sans-serif;color:black}
#inline-gradient{background-image:linear-gradient(90deg,rgb(255,0,0),rgb(0,0,255));background-repeat:no-repeat;background-clip:text;-webkit-text-fill-color:transparent}
#inline-sibling{-webkit-text-fill-color:transparent}
</style>`;
document.body.innerHTML = '<div id=gradient><span id=child>MMMM</span></div><div id=inline-line><span id=inline-gradient>MMMM</span><span id=inline-sibling>WWWW</span></div>';
'installed'
"#,
        )?;

        let computed = page_vm.vm_mut().eval(
            r#"[
getComputedStyle(document.getElementById('gradient')).getPropertyValue('-webkit-text-fill-color'),
getComputedStyle(document.getElementById('child')).getPropertyValue('-webkit-text-fill-color'),
getComputedStyle(document.getElementById('inline-gradient')).getPropertyValue('-webkit-text-fill-color')
].join('|')"#,
        )?;
        assert_eq!(
            computed,
            "rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)|rgba(0, 0, 0, 0)",
            "the Stylo longhand must cascade and inherit before paint"
        );

        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 120, 1.0))?
            .expect("background-clip:text fixture must retain a layout root");
        assert!(
            snapshot
                .fragments
                .iter()
                .filter(|fragment| matches!(
                    fragment,
                    moli_layout::PaintFragment::PushLayer {
                        composite: moli_layout::PaintCompositeMode::DestIn,
                        ..
                    }
                ))
                .count()
                >= 2,
            "both atomic and flattened inline backgrounds must receive text masks"
        );
        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::GlyphRun(run) if run.color.alpha == 0.0
        )));
        assert!(snapshot.fragments.iter().any(|fragment| matches!(
            fragment,
            moli_layout::PaintFragment::GlyphRun(run)
                if run.color == moli_layout::PaintColor::BLACK
        )));
        let mut mask_depth = None::<usize>;
        let mut mask_glyph_count = 0usize;
        let mut mask_glyph_counts = Vec::new();
        for fragment in &snapshot.fragments {
            match fragment {
                moli_layout::PaintFragment::PushLayer {
                    composite: moli_layout::PaintCompositeMode::DestIn,
                    ..
                } if mask_depth.is_none() => {
                    mask_depth = Some(1);
                    mask_glyph_count = 0;
                }
                moli_layout::PaintFragment::PushLayer { .. }
                | moli_layout::PaintFragment::PushClip { .. } => {
                    if let Some(depth) = mask_depth.as_mut() {
                        *depth += 1;
                    }
                }
                moli_layout::PaintFragment::PopLayer => {
                    let closes_mask = mask_depth.as_mut().is_some_and(|depth| {
                        *depth -= 1;
                        *depth == 0
                    });
                    if closes_mask {
                        mask_glyph_counts.push(mask_glyph_count);
                        mask_depth = None;
                    }
                }
                moli_layout::PaintFragment::GlyphRun(run) if mask_depth.is_some() => {
                    mask_glyph_count += run.glyphs.len();
                }
                _ => {}
            }
        }
        assert_eq!(
            mask_glyph_counts,
            [4, 4],
            "the flattened inline mask must exclude the adjacent transparent sibling's glyphs"
        );
        assert!(snapshot.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "background-clip-text-fallback"
        }));

        let image = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            <[u8; 4]>::try_from(&image.rgba[index..index + 4]).expect("RGBA pixel")
        };
        assert_eq!(
            pixel(4, 4),
            [255, 255, 255, 255],
            "padding must stay transparent instead of exposing the gradient rectangle"
        );
        assert_eq!(
            pixel(13, 13),
            [255, 255, 255, 255],
            "blank line-box space must stay transparent instead of using the old content-box fallback"
        );
        assert_eq!(
            pixel(20, 50),
            [255, 255, 255, 255],
            "line-height leading must not expose the gradient outside glyph ink"
        );
        let colored_ink = image
            .rgba
            .as_chunks::<4>().0.iter()
            .filter(|pixel| {
                let [red, green, blue, alpha] = **pixel;
                alpha == 255
                    && (red.abs_diff(blue) > 20
                        || red.max(blue).saturating_sub(green) > 20)
            })
            .count();
        assert!(
            colored_ink > 50,
            "the text mask must retain visible gradient glyph ink; colored_ink={colored_ink}"
        );
        let colored_inline_ink = image
            .rgba
            .as_chunks::<4>().0.iter()
            .enumerate()
            .filter(|(index, pixel)| {
                let [red, green, blue, alpha] = **pixel;
                let y = *index as u32 / image.width;
                y >= 64
                    && alpha == 255
                    && (red.abs_diff(blue) > 20
                        || red.max(blue).saturating_sub(green) > 20)
            })
            .count();
        assert!(
            colored_inline_ink > 50,
            "a flattened inline box must paint gradient glyph ink through its owner IFC mask; colored_inline_ink={colored_inline_ink}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("background-clip:text screenshot fixture should run");
}
