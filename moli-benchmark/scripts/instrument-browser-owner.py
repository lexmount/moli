"""Apply measurement-only probes to an isolated, clean checkout of this revision.

Never apply this to the production worktree. Preserve the resulting git diff,
build independently, and report its timing overhead against the unmodified CLI.
"""
import argparse
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('worktree', type=Path)
parser.add_argument('--revision', required=True, help='expected full source commit SHA')
parser.add_argument('--baseline', action='store_true', help='main without BrowserOwner or projection fences')
parser.add_argument('--operation-counts', action='store_true', help='aggregate BrowserHandle operation names; counts only')
args = parser.parse_args()
if args.baseline and args.operation_counts:
    parser.error('the baseline has no BrowserHandle operation queue')
root = args.worktree.resolve()
assert root != Path(__file__).resolve().parents[2], 'instrument an isolated worktree'
assert len(args.revision) == 40 and all(c in '0123456789abcdef' for c in args.revision), 'use a full commit SHA'
assert Path(subprocess.check_output(['git', '-C', str(root), 'rev-parse', '--show-toplevel'], text=True).strip()).resolve() == root
actual = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
assert actual == args.revision, (actual, args.revision)
assert not subprocess.check_output(['git', '-C', str(root), 'status', '--porcelain'])


def replace(path, before, after, count=1):
    file = root / path
    text = file.read_text()
    assert text.count(before) == count, (path, before, text.count(before))
    file.write_text(text.replace(before, after))


trace = root / 'moli-trace/src/lib.rs'
with trace.open('a') as output:
    output.write(r'''
// Measurement checkout only. System still uses the original native allocator.
// These counters cover Rust GlobalAlloc requests, not V8/C++ allocations.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);
struct Split3BenchmarkAllocator;
#[global_allocator]
static SPLIT3_BENCHMARK_ALLOCATOR: Split3BenchmarkAllocator = Split3BenchmarkAllocator;
fn counted_allocation(size: usize) {
    if COUNT_ALLOCATIONS.load(Relaxed) {
        ALLOCATIONS.fetch_add(1, Relaxed);
        REQUESTED_BYTES.fetch_add(size as u64, Relaxed);
    }
}
unsafe impl GlobalAlloc for Split3BenchmarkAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() { counted_allocation(layout.size()); }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() { counted_allocation(layout.size()); }
        result
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout); }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(ptr, layout, size) };
        if !result.is_null() { counted_allocation(size); }
        result
    }
}
pub fn split3_benchmark_record(stage: &str, sequence: u64, value_ns: u128, depth: usize) {
    use std::io::Write;
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static OUTPUT: std::sync::OnceLock<std::sync::Mutex<std::io::BufWriter<std::fs::File>>> = std::sync::OnceLock::new();
    let now_ns = START.get_or_init(std::time::Instant::now).elapsed().as_nanos();
    let counters = [ALLOCATIONS.load(Relaxed), REQUESTED_BYTES.load(Relaxed)];
    let output = OUTPUT.get_or_init(|| std::sync::Mutex::new(std::io::BufWriter::with_capacity(
        64 * 1024,
        std::fs::OpenOptions::new().write(true).create_new(true).open(
            std::env::var_os("SPLIT3_TRACE_FILE").expect("measurement trace path")
        ).expect("create measurement trace")
    )));
    let mut output = output.lock().unwrap();
    writeln!(output, "SPLIT3_TRACE {stage} {sequence} {now_ns} {value_ns} {depth} {} {}", counters[0], counters[1]).unwrap();
    // Every measured phase is durable before returning its snapshot. The
    // unmeasured shutdown tail may stay buffered when the CLI is terminated.
    if matches!(stage, "snapshot" | "output-admission-rejected") {
        output.flush().unwrap();
    }
    if stage == "snapshot" { COUNT_ALLOCATIONS.store(true, Relaxed); }
}
''')
replace('moli-protocol/src/domains/heap_profiler.rs',
        'Some(HeapProfilerAction::MoliDiagnostics) => {',
        'Some(HeapProfilerAction::MoliDiagnostics) => {\n            moli_trace::split3_benchmark_record("snapshot", 0, 0, 0);')
replace('moli-renderer-v8/src/runtime/protocol_output/transport.rs', '        if !admitted {\n            self.terminate();', '        if !admitted {\n            let pending = self.diagnostics();\n            moli_trace::split3_benchmark_record(\n                "output-admission-rejected", bytes as u64,\n                pending.pending_bytes as u128, pending.pending_messages,\n            );\n            self.terminate();')
if args.baseline:
    print(f'Baseline allocation probes applied at {root}; independent owner/fence absent')
    raise SystemExit(0)
with (root / 'moli-renderer-v8/src/lib.rs').open('a') as output:
    output.write('\npub use moli_trace::split3_benchmark_record;\n')

owner = 'moli-core/src/browser/owner.rs'
replace(owner, 'Execute(BrowserOperation),', 'Execute(BrowserOperation, std::time::Instant),')
replace(owner, 'BrowserOwnerMessage::Execute(Box::new', 'BrowserOwnerMessage::measured(Box::new', 3)
replace(owner, 'macro_rules! forward_context_read {', '''impl BrowserOwnerMessage {
    fn measured(operation: BrowserOperation) -> Self {
        Self::Execute(operation, std::time::Instant::now())
    }
}

macro_rules! forward_context_read {''')
replace(owner, 'Some(BrowserOwnerMessage::Execute(operation)) => operation(&mut browser),', '''Some(BrowserOwnerMessage::Execute(operation, queued_at)) => {
                                    let elapsed = queued_at.elapsed().as_nanos();
                                    moli_renderer_v8::split3_benchmark_record("owner-queue", 0, elapsed, rx.len() + 1);
                                    operation(&mut browser);
                                },''')
replace(owner, "type BrowserLocalOperation = Box<dyn FnOnce(&mut Browser) + 'static>;", '''struct BrowserLocalOperation {
    queued_at: std::time::Instant,
    operation: Box<dyn FnOnce(&mut Browser) + 'static>,
}

impl BrowserLocalOperation {
    fn new(operation: impl FnOnce(&mut Browser) + 'static) -> Self {
        let operation = Box::new(operation);
        Self { operation, queued_at: std::time::Instant::now() }
    }
}''')
replace(owner, '''if let Some(operation) = operation {
                                    operation(&mut browser);
                                }''', '''if let Some(operation) = operation {
                                    let elapsed = operation.queued_at.elapsed().as_nanos();
                                    moli_renderer_v8::split3_benchmark_record("owner-local-queue", 0, elapsed, local_rx.len() + 1);
                                    (operation.operation)(&mut browser);
                                }''')
local_sends = 0
for file in [root / owner, *(root / 'moli-core/src/browser/owner').glob('*.rs')]:
    text = file.read_text()
    count = text.count('.send(Box::new(move |browser| {')
    if count:
        file.write_text(text.replace('.send(Box::new(move |browser| {', '.send(crate::browser::owner::BrowserLocalOperation::new(move |browser| {'))
        local_sends += count
# Includes the native evaluation helper under cfg(test).
assert local_sends == 12, local_sends

commit = 'moli-core/src/browser/owner/navigation.rs'
replace(commit, '        let commit = self\n', '        let commit_started = std::time::Instant::now();\n        let commit = self\n')
replace(commit, '        let document = crate::browser::DocumentHandle::new(contents, commit.document);', '''        moli_renderer_v8::split3_benchmark_record(
            "document-commit", commit.lifecycle.browser_sequence.get(),
            commit_started.elapsed().as_nanos(), 0,
        );
        let document = crate::browser::DocumentHandle::new(contents, commit.document);''')
replace('moli-protocol/src/conn/state/devtools_renderer_channel.rs', '''        let pending = self
            .pending_document_projection
            .take()
            .expect("validated pending Document projection");''', '''        moli_trace::split3_benchmark_record("document-projection", fence.browser_sequence().get(), 0, 0);
        let pending = self
            .pending_document_projection
            .take()
            .expect("validated pending Document projection");''')
print(f'Measurement patch applied at {root}; {local_sends} local and 3 external enqueue sites')

if args.operation_counts:
    replace(owner, '''    ) -> Result<R, String> {
        let (result_tx, result_rx) = std_mpsc::sync_channel(1);''', '''    ) -> Result<R, String> {
        moli_renderer_v8::split3_benchmark_call(std::any::type_name_of_val(&operation));
        let (result_tx, result_rx) = std_mpsc::sync_channel(1);''')
    replace('moli-trace/src/lib.rs',
            'if stage == "snapshot" { COUNT_ALLOCATIONS.store(true, Relaxed); }',
            'if stage == "snapshot" { split3_dump_calls(); COUNT_ALLOCATIONS.store(true, Relaxed); }')
    replace('moli-renderer-v8/src/lib.rs',
            'pub use moli_trace::split3_benchmark_record;',
            'pub use moli_trace::split3_benchmark_record;\npub use moli_trace::split3_benchmark_call;')
    with trace.open('a') as output:
        output.write(r'''
// Aggregate operation names; no per-call formatting or file I/O.
fn split3_calls() -> &'static std::sync::Mutex<std::collections::BTreeMap<&'static str, u64>> {
    static CALLS: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeMap<&'static str, u64>>> = std::sync::OnceLock::new();
    CALLS.get_or_init(Default::default)
}
pub fn split3_benchmark_call(name: &'static str) {
    *split3_calls().lock().unwrap().entry(name).or_default() += 1;
}
fn split3_dump_calls() {
    use std::io::Write;
    let path = std::path::PathBuf::from(std::env::var_os("SPLIT3_TRACE_FILE").unwrap()).with_extension("calls.txt");
    let mut output = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    for (name, count) in split3_calls().lock().unwrap().iter() {
        writeln!(output, "{count} {name}").unwrap();
    }
}
''')
