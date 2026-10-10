#!/usr/bin/env python3
"""Benchmark typed AX and CDP payload construction using the same generated DOM.

Build both revisions with `cargo build --release -p moli`, then pass their
separate target/release/deps directories. The standalone Rust harness uses the
system allocator and synthetic visible styles; the CLI benchmark measures the
complete browser pipeline with its default allocator and real layout.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

NATIVE_SOURCE = r"""use std::{io, time::Instant};
use moli_dom::{NodeId, native::{DomHost, NativeDom}, accessibility::{AccessibilityStyle, AccessibilityStyleSource, AccessibilityInput, AccessibilityFrameState, AccessibilityRequest, accessibility_nodes_for_document, accessibility_payloads_for_document}};
struct Styles;
impl AccessibilityStyleSource for Styles {
    fn element_style(&mut self, _: NodeId) -> Option<AccessibilityStyle> {
        Some(AccessibilityStyle { display_none: false, visibility_visible: true, hides_contents: false, block_level: false })
    }
}
fn element(doc: &mut DomHost, parent: NodeId, tag: &str) -> NodeId {
    let node = doc.create_element(tag);
    assert!(doc.append_child(parent, node));
    node
}
fn text(doc: &mut DomHost, parent: NodeId, data: &str) {
    let node = doc.create_text_node(data);
    assert!(doc.append_child(parent, node));
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let case = args[1].as_str();
    let path = args[2].as_str();
    let count: usize = args[3].parse().unwrap();
    let mut doc = DomHost::from_dom(NativeDom::new_html(url::Url::parse("https://example.test/").unwrap()));
    let root = doc.document_node_id();
    let host = element(&mut doc, root, if case == "markers" { "ul" } else { "main" });
    for index in 0..count {
        match case {
            "rows" => {
                let row = element(&mut doc, host, "div");
                let link = element(&mut doc, row, "a");
                doc.set_attribute(link, "href", &format!("/item/{index}"));
                text(&mut doc, link, &format!("Item {index}"));
                let span = element(&mut doc, row, "span");
                text(&mut doc, span, " description");
            }
            "values" => {
                let input = element(&mut doc, host, "input");
                doc.set_attribute(input, "aria-label", "Text input");
                doc.set_attribute(input, "value", "Input value");
            }
            "ignored" => {
                let div = element(&mut doc, host, "div");
                doc.set_attribute(div, "aria-hidden", "true");
                let button = element(&mut doc, div, "button");
                text(&mut doc, button, "Ignored action");
            }
            "markers" => {
                let item = element(&mut doc, host, "li");
                text(&mut doc, item, "List item");
            }
            _ => panic!("unknown case"),
        }
    }
    let mut styles = Styles;
    let mut input = AccessibilityInput::new(&mut styles, AccessibilityFrameState::Active);
    let mut backend_id = |node: NodeId| Some(u32::try_from(node.index() + 1).unwrap());
    let started = Instant::now();
    match path {
        "typed" => {
            let nodes = accessibility_nodes_for_document(&doc, &mut input, root, AccessibilityRequest::Tree { max_depth: None }, &mut backend_id).unwrap();
            eprintln!("{{\"nodes\":{},\"build_seconds\":{}}}", nodes.len(), started.elapsed().as_secs_f64());
            serde_json::to_writer(io::stdout().lock(), &nodes).unwrap();
        }
        "protocol" => {
            let nodes = accessibility_payloads_for_document(&doc, &mut input, root, AccessibilityRequest::Tree { max_depth: None }, &mut backend_id).unwrap();
            eprintln!("{{\"nodes\":{},\"build_seconds\":{}}}", nodes.len(), started.elapsed().as_secs_f64());
            serde_json::to_writer(io::stdout().lock(), &nodes).unwrap();
        }
        _ => panic!("unknown path"),
    }
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before-deps', type=Path, required=True)
    parser.add_argument('--after-deps', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--count', type=int, default=20000)
    parser.add_argument('--repeats', type=int, default=5)
    args = parser.parse_args()
    if platform.system() != 'Linux':
        parser.error('peak RSS reporting currently requires Linux')
    if args.count < 1 or args.repeats < 1:
        parser.error('--count and --repeats must be positive')
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=True)
    source = root / 'native-storage.rs'
    source.write_text(NATIVE_SOURCE)
    manifest = {'count': args.count, 'repeats': args.repeats, 'libraries': {}}
    for label, deps in [('before', args.before_deps.resolve()), ('after', args.after_deps.resolve())]:
        command = ['rustc', '--edition=2024', '--crate-name', 'native_ax_storage', str(source),
                   '-L', f'dependency={deps}', '-C', 'panic=abort', '-C', 'opt-level=3',
                   '-C', 'codegen-units=1', '-D', 'warnings', '-o', str(root/f'native-{label}')]
        manifest['libraries'][label] = {}
        for crate in ('moli_dom', 'serde_json', 'url'):
            candidates = list(deps.glob(f'lib{crate}-*.rlib'))
            if not candidates:
                parser.error(f'missing {crate} release library in {deps}')
            rlib = max(candidates, key=lambda path: path.stat().st_mtime)
            with rlib.open('rb') as library:
                digest = hashlib.file_digest(library, 'sha256').hexdigest()
            manifest['libraries'][label][crate] = {'path': str(rlib), 'sha256': digest}
            command += ['--extern', f'{crate}={rlib}']
        subprocess.run(command, check=True)
    (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    records = []
    for label in ('before', 'after'):
        subprocess.run(['ax-storage', 'rows', 'typed', '100'],
                       executable=str(root/f'native-{label}'),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    with (root/'native-results.jsonl').open('w') as log:
        for case in ('rows', 'values', 'ignored', 'markers'):
            for path in ('typed', 'protocol'):
                hashes = set()
                for repeat in range(args.repeats):
                    order = ('before','after') if repeat % 2 == 0 else ('after','before')
                    for label in order:
                        dest = root/f'native-{case}-{path}-{repeat}-{label}.out'
                        meta = dest.with_suffix('.meta')
                        started = time.monotonic()
                        with dest.open('wb') as out, meta.open('wb') as err:
                            process = subprocess.Popen(['ax-storage',case,path,str(args.count)],
                                                       executable=str(root/f'native-{label}'),
                                                       stdout=out,stderr=err)
                            _, status, usage = os.wait4(process.pid,0)
                            process.returncode = os.waitstatus_to_exitcode(status)
                        if process.returncode:
                            raise RuntimeError(meta.read_text())
                        wall = time.monotonic()-started
                        with dest.open('rb') as out:
                            digest = hashlib.file_digest(out,'sha256').hexdigest()
                        hashes.add(digest)
                        record = dict(case=case,path=path,repeat=repeat,binary=label,
                                      maxrss_bytes=usage.ru_maxrss*1024,wall_seconds=wall,
                                      sha256=digest,**json.loads(meta.read_text()))
                        records.append(record)
                        line=json.dumps(record)
                        log.write(line+'\n')
                        log.flush()
                        print(line,flush=True)
                assert len(hashes)==1,(case,path,hashes)
    lines=['# Native AX construction benchmark','',
           'Same generated DOM and style source; Rust system allocator; no V8 or real layout.',
           f'Median of {args.repeats} alternating runs. Build time isolates the AX API call; peak RSS covers the process including DOM construction and streaming serialization.','',
           '| Case | Path | Nodes | RSS before MiB | RSS after MiB | Build before | Build after | Build change |',
           '|---|---|---:|---:|---:|---:|---:|---:|']
    for case,path in dict.fromkeys((r['case'],r['path']) for r in records):
        data={label:[r for r in records if (r['case'],r['path'],r['binary'])==(case,path,label)] for label in ('before','after')}
        rss={label:statistics.median(r['maxrss_bytes']/2**20 for r in rows) for label,rows in data.items()}
        build={label:statistics.median(r['build_seconds'] for r in rows) for label,rows in data.items()}
        lines.append(f"| {case} | {path} | {data['before'][0]['nodes']} | {rss['before']:.1f} | {rss['after']:.1f} | {build['before']:.4f}s | {build['after']:.4f}s | {(build['after']/build['before']-1)*100:+.1f}% |")
    lines += ['', 'Before/after output SHA-256 matches for every case and path.', '']
    report='\n'.join(lines)
    (root/'native-report.md').write_text(report)
    print(report)


if __name__ == '__main__':
    main()
