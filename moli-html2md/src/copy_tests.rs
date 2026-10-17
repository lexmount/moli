#[path = "../tests/support/mod.rs"]
mod support;

use crate::{
    Converter, Options,
    output::{Output, measure_copies},
};

#[test]
fn nested_table_fallback_copies_text_linearly() {
    let before = "x".repeat(128);
    let after = "y".repeat(128);
    let converter = Converter::new(Options {
        max_depth: usize::MAX,
        ..Options::default()
    });
    for heading in ["", "<tr><th>Header</th></tr>"] {
        let mut previous_copies = None;
        for depth in [64, 128, 256] {
            let html = format!(
                "{}leaf{}",
                format!("<table>{heading}<tr><td>{before}").repeat(depth),
                format!("{after}</td></tr></table>").repeat(depth)
            );
            let dom = support::Tree::parse(&html);
            let (actual, copied) = measure_copies(|| converter.convert(&dom, dom.root));
            assert_eq!(actual.matches(&before).count(), depth);
            assert_eq!(actual.matches(&after).count(), depth);
            assert_eq!(
                support::rendered_html(&actual).matches("<table>").count(),
                depth
            );
            // Count actual bytes copied when materializing fragments; completed
            // blocks move into parents without copying their text. This measures
            // work rather than elapsed time or recursion depth.
            assert!(
                copied <= 2 * actual.len(),
                "depth={depth}, output={}, copied={copied}",
                actual.len()
            );
            if let Some(previous) = previous_copies {
                assert!(
                    copied <= previous * 2 + 128,
                    "copy volume must grow linearly"
                );
            }
            previous_copies = Some(copied);
            println!(
                "header={}, depth={depth}, output={}, copied={copied}",
                !heading.is_empty(),
                actual.len()
            );
        }
    }
}

#[test]
fn dropping_nested_fragments_does_not_recurse() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut output = Output::from("leaf".to_owned());
            for _ in 0..20_000 {
                let mut parent = Output::default();
                parent.append(output);
                output = parent;
            }
            drop(output);
        })
        .expect("spawn small-stack test")
        .join()
        .expect("dropping fragments should be iterative");
}
