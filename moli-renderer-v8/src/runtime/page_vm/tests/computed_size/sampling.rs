//! Grid queries are geometry demands; the four size getters are snapshot-only.
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
async fn computed_size_held_getters_observe_grid_sampling_without_dom_mutation() {
    run_page_vm_async_test(async move {
        for real in [false, true] {
            for (property, raw, used) in [
                ("gridTemplateColumns", "1fr 3fr", "40px 120px"),
                ("gridTemplateRows", "40px", "40px"),
            ] {
                let mut page = page_with_size_fixture(GRID)?;
                if !real {
                    page.vm_mut()
                        .set_layout_policy(moli_page_types::LayoutPolicy::Mock);
                }
                let before = page.vm().layout_snapshot_cache_observability_for_test();
                let passes = page.vm().layout_pass_observability_for_test().1;
                assert!(before.3.is_none());
                let result = page.vm_mut().eval(&format!(
                    r#"JSON.stringify((() => {{
                    const held = getComputedStyle(document.getElementById('target'));
                    const sizes = () => [held.width,held.height,held.inlineSize,held.blockSize];
                    const before = sizes();
                    const tracks = held.{property};
                    return {{before, tracks, after:sizes(), repeated:sizes()}};
                }})())"#
                ))?;
                let result: serde_json::Value = serde_json::from_str(&result)?;
                let expected = if real {
                    json!(["160px", "40px", "160px", "40px"])
                } else {
                    json!(["auto", "auto", "auto", "auto"])
                };
                assert_eq!(
                    result,
                    json!({
                        "before":["auto","auto","auto","auto"],
                        "tracks":if real { used } else { raw },
                        "after":expected, "repeated":expected,
                    }),
                    "{property}, real={real}"
                );
                assert_eq!(
                    page.vm().layout_pass_observability_for_test().1,
                    passes + u64::from(real)
                );
                let after = page.vm().layout_snapshot_cache_observability_for_test();
                assert_eq!(after.2, before.2 + u64::from(real));
                assert_eq!(after.3.is_some(), real);
                if !real {
                    assert_eq!(after, before);
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("held CSSOM objects must not retain a cold geometry miss across getters");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_grid_sampling_reuses_old_geometry_until_explicit_refresh() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(GRID)?;
        page.vm_mut()
            .eval("globalThis.held=getComputedStyle(document.getElementById('target'));'held'")?;
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["auto", "auto", "auto", "auto"])
        );
        let passes = page.vm().layout_pass_observability_for_test().1;
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
            "40px 120px"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 1);
        assert_eq!(
            page.vm().layout_snapshot_cache_observability_for_test().2,
            sampled.2
        );
        publish_size_layout(&mut page)?;
        assert_eq!(
            held_sizes(&mut page)?,
            json!(["200px", "40px", "200px", "40px"])
        );
        assert_eq!(
            page.vm_mut().eval("held.gridTemplateColumns")?,
            "100px 100px"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, passes + 2);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid-created geometry follows the same snapshot lifecycle as explicit box reads");
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
