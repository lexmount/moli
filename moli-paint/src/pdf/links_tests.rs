use moli_layout::{
    PaintColor, PaintFragment, PaintLink, PaintLinkTarget, PaintPoint, PaintRect, PaintShape,
    PaintSnapshot, PaintTransform2D, PaintViewport,
};

use super::{PdfOptions, pdf_snapshot};

fn snapshot() -> PaintSnapshot {
    PaintSnapshot::new(PaintViewport::new(300, 600, 1.0), PaintColor::WHITE)
}
fn options() -> PdfOptions {
    PdfOptions {
        width: 300.0,
        height: 300.0,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
        ..Default::default()
    }
}
fn link(snapshot: &mut PaintSnapshot, rect: PaintRect, destination: Option<PaintPoint>) {
    snapshot.links.push(PaintLink {
        before_fragment: snapshot.fragments.len(),
        rect,
        transform: PaintTransform2D::IDENTITY,
        target: PaintLinkTarget {
            url: "https://example.com/doc#target".into(),
            destination,
        },
    });
}
fn document(snapshot: &PaintSnapshot, options: &PdfOptions) -> lopdf::Document {
    lopdf::Document::load_mem(&pdf_snapshot(snapshot, options).unwrap()).unwrap()
}
fn annotations(document: &lopdf::Document, page: u32) -> Vec<&lopdf::Dictionary> {
    let page = document
        .get_dictionary(document.get_pages()[&page])
        .unwrap();
    page.get(b"Annots")
        .ok()
        .into_iter()
        .flat_map(|o| o.as_array().unwrap())
        .map(|o| {
            let dict = document.get_dictionary(o.as_reference().unwrap()).unwrap();
            assert_eq!(dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Link");
            dict
        })
        .collect()
}
fn bounds(annotation: &lopdf::Dictionary) -> Vec<f32> {
    annotation
        .get(b"Rect")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_float().unwrap())
        .collect()
}
fn destination_page(document: &lopdf::Document, annotation: &lopdf::Dictionary) -> lopdf::ObjectId {
    let (_, dest) = document
        .dereference(annotation.get(b"Dest").unwrap())
        .unwrap();
    dest.as_array().unwrap()[0].as_reference().unwrap()
}

#[test]
fn links_preserve_separate_lines_and_deduplicate_descendant_regions() {
    let mut snapshot = snapshot();
    link(&mut snapshot, PaintRect::new(10.0, 20.0, 100.0, 15.0), None);
    link(&mut snapshot, PaintRect::new(12.0, 21.0, 96.0, 13.0), None);
    link(&mut snapshot, PaintRect::new(10.0, 50.0, 60.0, 15.0), None);
    let doc = document(&snapshot, &options());
    let annots = annotations(&doc, 1);
    assert_eq!(annots.len(), 2);
    for annot in &annots {
        let action = annot.get(b"A").unwrap().as_dict().unwrap();
        assert_eq!(action.get(b"S").unwrap().as_name().unwrap(), b"URI");
        assert_eq!(
            action.get(b"URI").unwrap().as_str().unwrap(),
            b"https://example.com/doc#target"
        );
        assert_eq!(
            annot.get(b"QuadPoints").unwrap().as_array().unwrap().len(),
            8
        );
        let rect = bounds(annot);
        assert!((rect[3] - rect[1] - 15.0).abs() < 0.01);
    }
    assert!(annotations(&doc, 2).is_empty());
}

#[test]
fn links_deduplicate_descendants_before_or_after_their_wrapped_anchor() {
    for descendants_first in [false, true] {
        let mut snapshot = snapshot();
        let parents = [
            PaintRect::new(10.0, 20.0, 100.0, 15.0),
            PaintRect::new(10.0, 50.0, 60.0, 15.0),
        ];
        for descendants in [descendants_first, !descendants_first] {
            for rect in parents {
                if descendants {
                    // Exceed the old 64-region lookback: every character of a
                    // long line can arrive before the enclosing inline region.
                    for index in 0..rect.width as usize {
                        link(
                            &mut snapshot,
                            PaintRect::new(rect.x + index as f32, rect.y, 1.0, rect.height),
                            None,
                        );
                    }
                } else {
                    link(&mut snapshot, rect, None);
                }
            }
        }
        let doc = document(&snapshot, &options());
        let annots = annotations(&doc, 1);
        assert_eq!(annots.len(), 2, "descendants first: {descendants_first}");
        for (annot, expected) in annots
            .iter()
            .zip([[10.0, 265.0, 110.0, 280.0], [10.0, 235.0, 70.0, 250.0]])
        {
            for (actual, expected) in bounds(annot).into_iter().zip(expected) {
                assert!((actual - expected).abs() < 0.01);
            }
        }
    }
}

#[test]
fn link_deduplication_keeps_different_targets_and_clipped_gaps() {
    let mut snapshot = snapshot();
    for destination in [Some(PaintPoint::new(20.0, 30.0)), None] {
        link(
            &mut snapshot,
            PaintRect::new(10.0, 20.0, 100.0, 15.0),
            destination,
        );
    }
    link(&mut snapshot, PaintRect::new(10.0, 20.0, 100.0, 15.0), None);
    snapshot.links.last_mut().unwrap().target.url = "https://other.example/".into();
    // Identical input rectangles in different clip scopes leave two separated
    // clickable regions. Deduplicate only after clipping, without filling gaps.
    for rect in [
        PaintRect::new(0.0, 50.0, 40.0, 30.0),
        PaintRect::new(60.0, 50.0, 40.0, 30.0),
    ] {
        snapshot.push_fragment(PaintFragment::PushClip {
            shape: PaintShape::Rect(rect),
            transform: PaintTransform2D::IDENTITY,
        });
        link(&mut snapshot, PaintRect::new(10.0, 50.0, 100.0, 15.0), None);
        snapshot.push_fragment(PaintFragment::PopLayer);
    }
    let doc = document(&snapshot, &options());
    let annots = annotations(&doc, 1);
    assert_eq!(annots.len(), 5);
    assert!(annots[0].has(b"Dest"));
    assert!(annots[1].has(b"A"));
    assert_eq!(
        annots[2]
            .get(b"A")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"URI")
            .unwrap()
            .as_str()
            .unwrap(),
        b"https://other.example/"
    );
    for (annot, expected) in annots[3..]
        .iter()
        .zip([[10.0, 235.0, 40.0, 250.0], [60.0, 235.0, 100.0, 250.0]])
    {
        for (actual, expected) in bounds(annot).into_iter().zip(expected) {
            assert!((actual - expected).abs() < 0.01);
        }
    }
}

#[test]
fn disjoint_link_lines_preserve_all_hit_regions() {
    let mut snapshot = PaintSnapshot::new(PaintViewport::new(300, 60_000, 1.0), PaintColor::WHITE);
    for line in 0..6_000 {
        link(
            &mut snapshot,
            PaintRect::new(10.0, line as f32 * 10.0, 100.0, 5.0),
            None,
        );
    }
    assert_eq!(super::links::collect(&snapshot).unwrap().len(), 6_000);
}

#[test]
fn dense_link_columns_print_separate_regions_and_remove_duplicates() {
    let mut snapshot = snapshot();
    for column in 0..1_600 {
        link(
            &mut snapshot,
            PaintRect::new(10.0 + column as f32 / 8.0, 20.0, 0.0625, 10.0),
            None,
        );
    }
    // Keep checking duplicates after the dense run of non-containing regions.
    link(
        &mut snapshot,
        PaintRect::new(10.0 + 1_599.0 / 8.0, 20.0, 0.0625, 10.0),
        None,
    );
    let doc = document(&snapshot, &options());
    let annots = annotations(&doc, 1);
    assert_eq!(annots.len(), 1_600);
    for annot in annots {
        let rect = bounds(annot);
        assert!((rect[2] - rect[0] - 0.0625).abs() < 0.01);
        assert_eq!(
            annot
                .get(b"A")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"URI")
                .unwrap()
                .as_str()
                .unwrap(),
            b"https://example.com/doc#target"
        );
    }
}

#[test]
fn many_repeated_links_print_as_one_annotation() {
    let mut snapshot = snapshot();
    link(&mut snapshot, PaintRect::new(10.0, 20.0, 100.0, 15.0), None);
    snapshot.links.resize(100_001, snapshot.links[0].clone());
    let doc = document(&snapshot, &options());

    assert_eq!(annotations(&doc, 1).len(), 1);
}

#[test]
fn pagination_clips_links_and_remaps_internal_destinations_to_selected_pages() {
    let mut snapshot = snapshot();
    link(
        &mut snapshot,
        PaintRect::new(10.0, 280.0, 50.0, 40.0),
        Some(PaintPoint::new(20.0, 350.0)),
    );
    let doc = document(&snapshot, &options());
    for page in [1, 2] {
        let annots = annotations(&doc, page);
        assert_eq!(annots.len(), 1);
        let rect = bounds(annots[0]);
        assert!((rect[3] - rect[1] - 20.0).abs() < 0.01);
        assert_eq!(destination_page(&doc, annots[0]), doc.get_pages()[&2]);
    }
    let selected = document(
        &snapshot,
        &PdfOptions {
            page_ranges: std::iter::once(1..=1).collect(),
            ..options()
        },
    );
    assert_eq!(
        destination_page(&selected, annotations(&selected, 1)[0]),
        selected.get_pages()[&1]
    );
    let selected = document(
        &snapshot,
        &PdfOptions {
            page_ranges: std::iter::once(0..=0).collect(),
            ..options()
        },
    );
    let annots = annotations(&selected, 1);
    assert!(!annots[0].has(b"Dest"));
    assert!(
        annots[0].has(b"A"),
        "an omitted destination uses the original URL"
    );
}

#[test]
fn transformed_clips_and_embedded_snapshots_keep_links_in_their_scopes() {
    let mut child = snapshot();
    link(
        &mut child,
        PaintRect::new(5.0, 10.0, 100.0, 60.0),
        Some(PaintPoint::new(8.0, 20.0)),
    );
    let mut snapshot = snapshot();
    snapshot.append_embedded_snapshot(
        child,
        PaintRect::new(0.0, 0.0, 50.0, 40.0),
        PaintTransform2D {
            coefficients: [1.0, 0.0, 0.5, 1.0, 30.0, 50.0],
        },
    );
    let doc = document(&snapshot, &options());
    let annots = annotations(&doc, 1);
    assert_eq!(annots.len(), 1);
    let rect = bounds(annots[0]);
    for (actual, expected) in rect.iter().zip([40.0, 210.0, 100.0, 240.0]) {
        assert!((actual - expected).abs() < 0.01, "{rect:?}");
    }
    let quad = annots[0].get(b"QuadPoints").unwrap().as_array().unwrap();
    assert_ne!(
        quad[0], quad[6],
        "skewed hit region must not become its bounding rectangle"
    );
    let (_, dest) = doc.dereference(annots[0].get(b"Dest").unwrap()).unwrap();
    let dest = dest.as_array().unwrap();
    assert_eq!(dest[2].as_float().unwrap(), 48.0);
    assert_eq!(dest[3].as_float().unwrap(), 230.0);
}

#[test]
fn invisible_and_fully_clipped_links_are_absent_and_invalid_metadata_fails() {
    let mut snapshot = snapshot();
    snapshot.push_fragment(PaintFragment::PushClip {
        shape: PaintShape::Rect(PaintRect::new(0.0, 0.0, 30.0, 30.0)),
        transform: PaintTransform2D::IDENTITY,
    });
    link(&mut snapshot, PaintRect::new(60.0, 0.0, 40.0, 20.0), None);
    snapshot.push_fragment(PaintFragment::PopLayer);
    snapshot.push_fragment(PaintFragment::PushLayer {
        opacity: 0.0,
        blend_mode: Default::default(),
        composite: Default::default(),
        filter: None,
        clip: PaintShape::Rect(PaintRect::new(0.0, 0.0, 300.0, 300.0)),
        transform: PaintTransform2D::IDENTITY,
    });
    link(&mut snapshot, PaintRect::new(10.0, 10.0, 40.0, 20.0), None);
    snapshot.push_fragment(PaintFragment::PopLayer);
    let doc = document(&snapshot, &options());
    assert!(annotations(&doc, 1).is_empty());
    snapshot.links[0].before_fragment = usize::MAX;
    assert!(pdf_snapshot(&snapshot, &options()).is_err());
    snapshot.links[0].before_fragment = 1;
    snapshot.links[0].target.destination = Some(PaintPoint::new(f32::NAN, 0.0));
    assert!(pdf_snapshot(&snapshot, &options()).is_err());
}

#[test]
fn curved_clips_do_not_create_clickable_bounding_box_corners() {
    let mut snapshot = snapshot();
    let radius = moli_layout::PaintCornerRadius { x: 25.0, y: 25.0 };
    snapshot.push_fragment(PaintFragment::PushClip {
        shape: PaintShape::RoundedRect {
            rect: PaintRect::new(10.0, 10.0, 50.0, 50.0),
            radii: moli_layout::PaintCornerRadii {
                top_left: radius,
                top_right: radius,
                bottom_left: radius,
                bottom_right: radius,
            },
        },
        transform: PaintTransform2D::IDENTITY,
    });
    link(&mut snapshot, PaintRect::new(10.0, 10.0, 50.0, 50.0), None);
    snapshot.push_fragment(PaintFragment::PopLayer);
    let doc = document(&snapshot, &options());
    let annots = annotations(&doc, 1);
    assert!(!annots.is_empty());
    assert!(annots.iter().all(|a| {
        let r = bounds(a);
        !(r[0] <= 11.0 && r[2] >= 11.0 && r[1] <= 289.0 && r[3] >= 289.0)
    }));
    assert!(annots.iter().any(|a| {
        let r = bounds(a);
        r[0] < 35.0 && r[2] > 35.0 && r[1] <= 265.0 && r[3] >= 265.0
    }));
}
