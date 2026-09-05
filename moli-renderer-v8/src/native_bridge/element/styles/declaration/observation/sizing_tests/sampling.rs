//! A Grid used-track query can create the first geometry sample. Size reads
//! themselves cannot: their first result, including absence, belongs to one
//! observation. A later observation may therefore return different sizes
//! without any intervening DOM/style mutation.

use std::collections::BTreeMap;

use super::*;

const GRID: &str = r#"<div id=target class=sized style="display:grid;grid-template-columns:30px 1fr;grid-template-rows:20px 1fr"></div>"#;

fn inspector_properties(host: &JsContextHost, target: DomHandle) -> BTreeMap<String, String> {
    crate::native_bridge::element::computed_style_properties_for_inspector_handle(host, target)
        .unwrap()
        .into_iter()
        .collect()
}

#[test]
fn computed_size_cold_inspector_enumeration_transitions_to_a_stable_sample() {
    for real in [false, true] {
        let mut vm = fixture(GRID);
        if !real {
            vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let target = host.dom_host().element_handle_by_id("target").unwrap();
        let before = host.layout_snapshot_cache_observability_for_test();
        assert!(before.cached.is_none());
        let passes = host.layout_pass_observability_for_test().1;
        take_query_counts();
        let initial = inspector_properties(&host, target);
        assert!(initial.len() > 200);
        let size_names = ["width", "height", "inline-size", "block-size"];
        for (name, expected) in size_names
            .into_iter()
            .zip(["500px", "400px", "500px", "400px"])
        {
            assert_eq!(initial[name], expected, "cold {name}, real={real}");
        }
        assert_eq!(take_query_counts(), SizeQueryCounts::default());
        let sampled = host.layout_snapshot_cache_observability_for_test();
        assert_eq!(sampled.publishes, before.publishes + u64::from(real));
        assert_eq!(sampled.cached.is_some(), real);
        assert_eq!(
            host.layout_pass_observability_for_test().1,
            passes + u64::from(real)
        );
        if !real {
            assert_eq!(sampled, before);
        }

        let mut previous = None;
        for _ in 0..3 {
            let values = inspector_properties(&host, target);
            let expected = if real {
                ["120px", "80px", "120px", "80px"]
            } else {
                ["500px", "400px", "500px", "400px"]
            };
            for (name, expected) in size_names.into_iter().zip(expected) {
                assert_eq!(values[name], expected, "sampled {name}, real={real}");
            }
            assert_eq!(
                values.keys().collect::<Vec<_>>(),
                initial.keys().collect::<Vec<_>>()
            );
            for (name, value) in &initial {
                if !size_names.contains(&name.as_str()) {
                    assert_eq!(&values[name], value, "non-size property {name}");
                }
            }
            if let Some(previous) = previous {
                assert_eq!(values, previous, "same-sample declarations stay identical");
            }
            previous = Some(values);
            assert_eq!(
                take_query_counts(),
                SizeQueryCounts {
                    source_queries: usize::from(real),
                    ..Default::default()
                }
            );
            assert_eq!(
                host.layout_pass_observability_for_test().1,
                passes + u64::from(real)
            );
            let after = host.layout_snapshot_cache_observability_for_test();
            assert_eq!(after.publishes, sampled.publishes);
            assert_eq!(after.cached, sampled.cached);
        }
    }
}

#[test]
fn computed_size_observation_pins_first_sample_availability_not_the_held_js_wrapper() {
    for grid_first in [false, true] {
        let vm = fixture(GRID);
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let host = host.borrow();
        let target = host.dom_host().element_handle_by_id("target").unwrap();
        let passes = host.layout_pass_observability_for_test().1;
        let before = host.layout_snapshot_cache_observability_for_test();
        assert!(before.cached.is_none());
        {
            let read = ComputedStyleRead::new(&host, target);
            if !grid_first {
                assert_eq!(sizes(&read), ["500px", "400px", "500px", "400px"]);
                assert_eq!(host.layout_snapshot_cache_observability_for_test(), before);
            }
            assert_eq!(read.property("grid-template-columns"), "30px 90px");
            let sampled = host.layout_snapshot_cache_observability_for_test();
            assert_eq!(sampled.publishes, before.publishes + 1);
            take_query_counts();
            for _ in 0..20 {
                assert_eq!(
                    sizes(&read),
                    if grid_first {
                        ["120px", "80px", "120px", "80px"]
                    } else {
                        ["500px", "400px", "500px", "400px"]
                    }
                );
            }
            assert_eq!(
                take_query_counts(),
                SizeQueryCounts {
                    source_queries: usize::from(grid_first),
                    ..Default::default()
                }
            );
            assert_eq!(host.layout_snapshot_cache_observability_for_test(), sampled);
        }
        assert_eq!(
            sizes(&ComputedStyleRead::new(&host, target)),
            ["120px", "80px", "120px", "80px"]
        );
        assert_eq!(take_query_counts().source_queries, 1);
        assert_eq!(host.layout_pass_observability_for_test().1, passes + 1);
    }
}

#[test]
fn computed_size_document_batch_pins_one_sample_even_if_grid_is_read_mid_batch() {
    for grid_first in [false, true] {
        let vm = fixture(&format!("{GRID}<div id=other class=sized></div>"));
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let handles = {
            let host = host.borrow();
            ["target", "other"].map(|id| host.dom_host().element_handle_by_id(id).unwrap())
        };
        let properties = if grid_first {
            [
                "grid-template-columns",
                "width",
                "height",
                "inline-size",
                "block-size",
            ]
        } else {
            [
                "width",
                "grid-template-columns",
                "height",
                "inline-size",
                "block-size",
            ]
        }
        .map(String::from);
        let passes = vm.layout_pass_observability_for_test().1;
        assert!(
            vm.layout_snapshot_cache_observability_for_test()
                .3
                .is_none()
        );
        take_query_counts();
        let rows = vm.computed_style_property_values_for_document_snapshot(handles, &properties);
        assert_eq!(rows.len(), 2);
        for (row, tracks) in rows.iter().zip(["30px 90px", "none"]) {
            let expected = if grid_first {
                [tracks, "120px", "80px", "120px", "80px"]
            } else {
                ["500px", tracks, "400px", "500px", "400px"]
            };
            assert_eq!(row, &expected, "grid_first={grid_first}");
        }
        assert_eq!(take_query_counts().index_builds, usize::from(grid_first));
        let sampled = vm.layout_snapshot_cache_observability_for_test();
        assert!(sampled.3.is_some());
        let rows = vm.computed_style_property_values_for_document_snapshot(
            handles,
            &["width", "height", "inline-size", "block-size"].map(String::from),
        );
        for row in rows {
            assert_eq!(row, ["120px", "80px", "120px", "80px"]);
        }
        assert_eq!(
            take_query_counts().index_builds,
            1,
            "a new batch derives its own table"
        );
        assert_eq!(vm.layout_snapshot_cache_observability_for_test(), sampled);
        assert_eq!(vm.layout_pass_observability_for_test().1, passes + 1);
    }
}
