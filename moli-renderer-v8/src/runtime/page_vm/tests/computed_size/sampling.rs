//! Grid and size queries only read published layout; explicit output publishes it.
//! Unlike an internal synchronous property batch, a held JavaScript CSSOM
//! wrapper starts a new read for each getter and must see newly sampled boxes.

use super::*;

const GRID: &str = r#"<main id=parent style="width:160px"><div id=target style="display:grid;grid-template-columns:1fr 3fr;grid-template-rows:40px"></div></main>"#;

fn held_sizes(page: &mut PageVm) -> anyhow::Result<serde_json::Value> {
    read_without_layout(
        page,
        "[held.width,held.height,held.inlineSize,held.blockSize]",
    )
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_held_getters_wait_for_explicit_publication() {
    run_page_vm_async_test(async move {
        for real in [false, true] {
            let mut page = page_with_size_fixture(GRID)?;
            if !real {
                page.vm_mut()
                    .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
            }
            page.vm_mut().eval(
                "globalThis.held=getComputedStyle(document.getElementById('target'));'held'",
            )?;
            let passes = page.vm().layout_pass_observability_for_test().1;
            for _ in 0..3 {
                assert_eq!(
                    read_without_layout(
                        &mut page,
                        "[held.gridTemplateColumns,held.gridTemplateRows]"
                    )?,
                    json!(["1fr 3fr", "40px"])
                );
                assert_eq!(
                    held_sizes(&mut page)?,
                    json!(["auto", "auto", "auto", "auto"])
                );
            }
            assert!(
                page.vm()
                    .layout_snapshot_cache_observability_for_test()
                    .3
                    .is_none()
            );
            if real {
                publish_size_layout(&mut page)?;
            }
            for _ in 0..3 {
                assert_eq!(
                    held_sizes(&mut page)?,
                    if real {
                        json!(["160px", "40px", "160px", "40px"])
                    } else {
                        json!(["auto", "auto", "auto", "auto"])
                    }
                );
                assert_eq!(
                    read_without_layout(
                        &mut page,
                        "[held.gridTemplateColumns,held.gridTemplateRows]"
                    )?,
                    json!([if real { "40px 120px" } else { "1fr 3fr" }, "40px"])
                );
                assert_eq!(
                    page.vm().layout_pass_observability_for_test().1,
                    passes + u64::from(real)
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("held CSSOM objects observe the next explicit publication");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_grid_sampling_refreshes_dirty_geometry_on_first_demand() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(GRID)?;
        page.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 320,
            inner_height: 240,
            device_pixel_ratio: 1.0,
            ..Default::default()
        }))?;
        page.vm_mut()
            .eval("globalThis.held=getComputedStyle(document.getElementById('target'));'held'")?;
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["auto", "auto", "auto", "auto"])
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            read_without_layout(&mut page, "held.gridTemplateColumns")?,
            json!("1fr 3fr")
        );
        publish_size_layout(&mut page)?;
        assert_eq!(
            page.vm_mut().eval("held.gridTemplateColumns")?,
            "40px 120px"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["160px", "40px", "160px", "40px"])
        );
        let sampled = page.vm().layout_snapshot_cache_observability_for_test();
        page.vm_mut().eval(
            r#"
            document.getElementById('parent').style.width='200px';
            document.getElementById('target').style.gridTemplateColumns='1fr 1fr';
            document.getElementById('target').style.color='red';
            'mutated'
        "#,
        )?;
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm().layout_snapshot_cache_observability_for_test(),
            sampled,
            "style mutation must only mark the retained snapshot dirty"
        );
        assert_eq!(
            read_without_layout(&mut page, "held.color")?,
            json!("rgb(255, 0, 0)")
        );
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["160px", "40px", "160px", "40px"])
        );
        assert_eq!(
            page.vm_mut().eval("held.gridTemplateColumns")?,
            "100px 100px"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["200px", "40px", "200px", "40px"])
        );
        let refreshed = page.vm().layout_snapshot_cache_observability_for_test();
        assert_eq!(
            refreshed.2,
            sampled.2 + 1,
            "the first exact demand must publish one replacement tree"
        );

        page.vm_mut().eval("'clean turn'")?;
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
        assert_eq!(
            page.vm().layout_snapshot_cache_observability_for_test(),
            refreshed,
            "a clean turn must keep the replacement tree reusable"
        );
        assert_eq!(
            page.vm_mut().eval("held.gridTemplateColumns")?,
            "100px 100px"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);

        publish_size_layout(&mut page)?;
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["200px", "40px", "200px", "40px"])
        );
        assert_eq!(
            page.vm_mut().eval("held.gridTemplateColumns")?,
            "100px 100px"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 3);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid geometry follows explicit visual publication");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_grid_sampling_does_not_warm_another_page() {
    run_page_vm_async_test(async move {
        let mut first = page_with_size_fixture(GRID)?;
        let mut second = page_with_size_fixture(GRID)?;
        for page in [&mut first, &mut second] {
            page.vm_mut().eval(
                "globalThis.held=getComputedStyle(document.getElementById('target'));'held'",
            )?;
            assert_eq!(held_sizes(page)?, json!(["auto", "auto", "auto", "auto"]));
        }
        let second_cache = second.vm().layout_snapshot_cache_observability_for_test();
        let second_passes = second.vm().layout_pass_observability_for_test().1;
        publish_size_layout(&mut first)?;
        assert_eq!(
            first.vm_mut().eval("held.gridTemplateColumns")?,
            "40px 120px"
        );
        assert_eq!(
            held_sizes(&mut first)?,
            json!(["160px", "40px", "160px", "40px"])
        );
        assert_eq!(
            held_sizes(&mut second)?,
            json!(["auto", "auto", "auto", "auto"])
        );
        assert_eq!(
            second.vm().layout_snapshot_cache_observability_for_test(),
            second_cache
        );
        assert_eq!(
            second.vm().layout_pass_observability_for_test().1,
            second_passes
        );
        assert_eq!(second.vm_mut().eval("held.gridTemplateRows")?, "40px");
        assert_eq!(
            second.vm().layout_pass_observability_for_test().1,
            second_passes
        );
        assert_eq!(
            held_sizes(&mut second)?,
            json!(["auto", "auto", "auto", "auto"])
        );
        publish_size_layout(&mut second)?;
        assert_eq!(
            held_sizes(&mut second)?,
            json!(["160px", "40px", "160px", "40px"])
        );
        assert_eq!(
            second.vm().layout_pass_observability_for_test().1,
            second_passes + 1
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("one Page's geometry publication must not change another Page's cold size reads");
}
