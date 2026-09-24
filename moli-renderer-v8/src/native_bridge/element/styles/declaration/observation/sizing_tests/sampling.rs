//! Grid, size, and inspector reads consume one published layout. None can
//! create the first sample, including when Grid is read first in a batch.

use super::*;
use std::collections::BTreeMap;

const GRID: &str = r#"<div id=target class=sized style="display:grid;grid-template-columns:30px 1fr;grid-template-rows:20px 1fr"></div>"#;

fn inspector_properties(host: &JsContextHost, target: DomHandle) -> BTreeMap<String, String> {
    crate::native_bridge::element::computed_style_properties_for_inspector_handle(host, target)
        .unwrap()
        .into_iter()
        .collect()
}

#[test]
fn computed_size_inspector_enumeration_waits_for_explicit_publication() {
    for real in [false, true] {
        let mut vm = fixture(GRID);
        if !real {
            vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        }
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let target = host
            .borrow()
            .dom_host()
            .element_handle_by_id("target")
            .unwrap();
        let passes = vm.layout_pass_observability_for_test().1;
        let initial = inspector_properties(&host.borrow(), target);
        assert!(initial.len() > 200);
        for _ in 0..3 {
            assert_eq!(inspector_properties(&host.borrow(), target), initial);
            assert_eq!(vm.layout_pass_observability_for_test().1, passes);
            assert!(
                vm.layout_snapshot_cache_observability_for_test()
                    .3
                    .is_none()
            );
        }
        if real {
            vm.publish_layout_for_test().unwrap();
        }
        let size_names = ["width", "height", "inline-size", "block-size"];
        for _ in 0..3 {
            take_query_counts();
            let values = inspector_properties(&host.borrow(), target);
            for (name, expected) in size_names.into_iter().zip(if real {
                ["120px", "80px", "120px", "80px"]
            } else {
                ["500px", "400px", "500px", "400px"]
            }) {
                assert_eq!(values[name], expected, "{name}, real={real}");
                assert_eq!(
                    initial[name],
                    if name == "width" || name == "inline-size" {
                        "500px"
                    } else {
                        "400px"
                    }
                );
            }
            assert_eq!(
                values.keys().collect::<Vec<_>>(),
                initial.keys().collect::<Vec<_>>()
            );
            for (name, value) in &initial {
                if !size_names.contains(&name.as_str()) && !name.starts_with("grid-template-") {
                    assert_eq!(&values[name], value, "non-geometric property {name}");
                }
            }
            assert_eq!(
                values["grid-template-columns"],
                if real { "30px 90px" } else { "30px 1fr" }
            );
            assert_eq!(take_query_counts().source_queries, usize::from(real));
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                passes + u64::from(real)
            );
        }
    }
}

#[test]
fn computed_size_observation_stays_cold_regardless_of_grid_read_order() {
    for grid_first in [false, true] {
        let mut vm = fixture(GRID);
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let target = host
            .borrow()
            .dom_host()
            .element_handle_by_id("target")
            .unwrap();
        let passes = vm.layout_pass_observability_for_test().1;
        {
            let host = host.borrow();
            let read = ComputedStyleRead::new(&host, target);
            take_query_counts();
            if !grid_first {
                assert_eq!(sizes(&read), ["500px", "400px", "500px", "400px"]);
            }
            assert_eq!(read.property("grid-template-columns"), "30px 1fr");
            for _ in 0..20 {
                assert_eq!(sizes(&read), ["500px", "400px", "500px", "400px"]);
            }
            assert_eq!(take_query_counts(), SizeQueryCounts::default());
        }
        assert_eq!(vm.layout_pass_observability_for_test().1, passes);
        vm.publish_layout_for_test().unwrap();
        let host = host.borrow();
        take_query_counts();
        let read = ComputedStyleRead::new(&host, target);
        for _ in 0..20 {
            assert_eq!(sizes(&read), ["120px", "80px", "120px", "80px"]);
        }
        assert_eq!(read.property("grid-template-columns"), "30px 90px");
        assert_eq!(take_query_counts().source_queries, 1);
        assert_eq!(host.layout_pass_observability_for_test().1, passes + 1);
    }
}

#[test]
fn computed_size_document_batch_never_publishes_layout_regardless_of_grid_order() {
    for grid_first in [false, true] {
        let mut vm = fixture(&format!("{GRID}<div id=other class=sized></div>"));
        let host = vm.context_host_weak_for_test().upgrade().unwrap();
        let handles = ["target", "other"]
            .map(|id| host.borrow().dom_host().element_handle_by_id(id).unwrap());
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
        for published in [false, true] {
            if published {
                vm.publish_layout_for_test().unwrap();
            }
            take_query_counts();
            let rows =
                vm.computed_style_property_values_for_document_snapshot(handles, &properties);
            assert_eq!(rows.len(), 2);
            let tracks = if published { "30px 90px" } else { "30px 1fr" };
            for (row, tracks) in rows.iter().zip([tracks, "none"]) {
                let (width, height) = if published {
                    ("120px", "80px")
                } else {
                    ("500px", "400px")
                };
                let expected = if grid_first {
                    [tracks, width, height, width, height]
                } else {
                    [width, tracks, height, width, height]
                };
                assert_eq!(
                    row, &expected,
                    "grid_first={grid_first}, published={published}"
                );
            }
            assert_eq!(take_query_counts().index_builds, usize::from(published));
            assert_eq!(
                vm.layout_pass_observability_for_test().1,
                passes + u64::from(published)
            );
        }
    }
}
