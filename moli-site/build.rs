use std::{
    collections::{BTreeMap, HashMap},
    env,
    fmt::Write as _,
    fs,
    path::PathBuf,
};

const LEAF_FLAG: u8 = 1;
const EXCEPTION_FLAG: u8 = 2;
const MAX_LABEL_LEN: usize = u8::MAX as usize;
const MAX_EDGE_COUNT: usize = u16::MAX as usize;
const MAX_LABEL_OFFSET: usize = 0x00ff_ffff;

#[derive(Default)]
struct TrieNode {
    flags: u8,
    children: BTreeMap<String, TrieNode>,
}

fn main() {
    const SOURCE_PATH: &str = "src/data/public_domains.txt";
    println!("cargo:rerun-if-changed={SOURCE_PATH}");

    let source = fs::read_to_string(SOURCE_PATH).expect("read vendored public suffix list");
    let (root, rule_count) = parse_rules(&source);
    let generated = generate_tables(&root, rule_count);
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("public_suffix_tables.rs");
    fs::write(output, generated).expect("write generated public suffix tables");
}

fn parse_rules(source: &str) -> (TrieNode, usize) {
    let mut root = TrieNode::default();
    let mut rule_count = 0;

    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        let (rule, is_exception) = match line.strip_prefix('!') {
            Some(rule) => (rule, true),
            None => (line, false),
        };
        assert!(!rule.is_empty(), "public suffix rule must not be empty");
        assert!(
            rule.is_ascii() && rule.bytes().all(|byte| !byte.is_ascii_uppercase()),
            "public suffix rule must be lowercase ASCII: {line}"
        );
        assert!(
            !rule.contains('*') || (rule.starts_with("*.") && rule[2..].find('*').is_none()),
            "public suffix wildcard must be the leftmost label: {line}"
        );

        let mut node = &mut root;
        let mut label_count = 0;
        for label in rule.rsplit('.') {
            assert!(
                !label.is_empty(),
                "public suffix label must not be empty: {line}"
            );
            assert!(
                label.len() <= MAX_LABEL_LEN,
                "public suffix label is too long: {label}"
            );
            node = node.children.entry(label.to_owned()).or_default();
            label_count += 1;
        }
        assert!(
            !is_exception || label_count > 1,
            "public suffix exception must contain more than one label: {line}"
        );
        assert_eq!(node.flags, 0, "duplicate public suffix rule: {line}");
        node.flags = LEAF_FLAG | if is_exception { EXCEPTION_FLAG } else { 0 };
        rule_count += 1;
    }

    assert!(
        rule_count > 0,
        "vendored public suffix list must not be empty"
    );
    (root, rule_count)
}

fn generate_tables(root: &TrieNode, rule_count: usize) -> String {
    // Nodes are visited breadth-first. Every edge appends exactly one child to
    // `nodes`, so an edge at index N always points to node N + 1. That removes
    // the child-index field from the generated edge table.
    let mut nodes = vec![root];
    let mut node_metadata = Vec::new();
    let mut edges = Vec::new();
    let mut label_offsets = HashMap::<&str, u32>::new();
    let mut labels = Vec::new();
    let mut node_index = 0;

    while node_index < nodes.len() {
        let node = nodes[node_index];
        let edge_start = edges.len();
        assert!(
            node.children.len() <= MAX_EDGE_COUNT,
            "a public suffix trie node has too many children"
        );

        for (label, child) in &node.children {
            let label_start = match label_offsets.get(label.as_str()) {
                Some(offset) => *offset,
                None => {
                    assert!(
                        labels.len() <= MAX_LABEL_OFFSET,
                        "public suffix label blob exceeds the 24-bit edge encoding"
                    );
                    let offset = labels.len() as u32;
                    labels.extend_from_slice(label.as_bytes());
                    label_offsets.insert(label, offset);
                    offset
                }
            };
            let encoded_edge = (label_start << 8) | label.len() as u32;
            edges.push(encoded_edge);
            nodes.push(child);
            assert_eq!(
                nodes.len() - 1,
                edges.len(),
                "breadth-first edge index must identify its child node"
            );
        }

        let metadata =
            edge_start as u64 | ((node.children.len() as u64) << 32) | ((node.flags as u64) << 48);
        node_metadata.push(metadata);
        node_index += 1;
    }

    assert_eq!(nodes.len(), node_metadata.len());
    assert_eq!(nodes.len(), edges.len() + 1);

    let mut output = String::new();
    writeln!(output, "#[cfg(test)]").unwrap();
    writeln!(output, "pub(super) const RULE_COUNT: usize = {rule_count};").unwrap();
    writeln!(
        output,
        "pub(super) static NODE_METADATA: [u64; {}] = [",
        node_metadata.len()
    )
    .unwrap();
    for chunk in node_metadata.chunks(8) {
        output.push_str("    ");
        for value in chunk {
            write!(output, "0x{value:016x}, ").unwrap();
        }
        output.push('\n');
    }
    output.push_str("];\n");

    writeln!(
        output,
        "pub(super) static EDGES: [u32; {}] = [",
        edges.len()
    )
    .unwrap();
    for chunk in edges.chunks(12) {
        output.push_str("    ");
        for value in chunk {
            write!(output, "0x{value:08x}, ").unwrap();
        }
        output.push('\n');
    }
    output.push_str("];\n");

    writeln!(
        output,
        "pub(super) static LABELS: [u8; {}] = [",
        labels.len()
    )
    .unwrap();
    for chunk in labels.chunks(24) {
        output.push_str("    ");
        for byte in chunk {
            write!(output, "0x{byte:02x}, ").unwrap();
        }
        output.push('\n');
    }
    output.push_str("];\n");

    output
}
