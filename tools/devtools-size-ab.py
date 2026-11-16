#!/usr/bin/env python3
"""Compare the same Limo consumer with renderer DevTools enabled and disabled.

Uses this checkout's patches and local moli-core. Both variants share a lockfile,
target directory and release profile; the existing Limo executable is preserved.
Run from the Moli root: python3 tools/devtools-size-ab.py --limo-root ../limo
API contracts need no Limo checkout: python3 tools/devtools-size-ab.py --check-only
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys


def elf_sections(path):
    data = path.read_bytes()
    header = struct.unpack_from('<16sHHIQQQIHHHHHH', data)
    assert header[0][:6] == b'\x7fELF\x02\x01', 'Expected little-endian ELF64'
    sections = [struct.unpack_from('<IIQQQQIIQQ', data, header[6] + i * header[11])
                for i in range(header[12])]
    strings = sections[header[13]]
    names = data[strings[4]:strings[4] + strings[5]]
    return {names[s[0]:].split(b'\0', 1)[0].decode(): s[5] for s in sections}


def audit_symbols(path, devtools):
    """Check linked implementations, including drop glue, rather than strings."""
    symbols = path.read_text()
    frontend = (
        '::initialize_inspector_session_after_attach',
        '::prepare_frontend',
        '::run_dom_inspector_edit',
        '::service_worker_runtime::service::devtools_commands::',
        'RendererDevToolsSessionOutputHost',
        'WorkerInspectorSessionRetirement',
        'WorkerInspectorOutboundState',
        '::script_vm::input_dispatch::',
        '::page_vm::frontend_node_bindings::',
        '::page_vm::dom_mutations::',
        'WorkerInspectorClient as v8::inspector::V8InspectorClientImpl>::run_message_loop_on_pause',
    )
    matches = {name: [line for line in symbols.splitlines() if name in line]
               for name in frontend}
    retained = {name: lines for name, lines in matches.items() if lines}
    native_inspector = 'v8_inspector::V8Inspector::create(' in symbols
    if not native_inspector:
        raise RuntimeError('The native V8 Inspector backend must remain linked')
    if not devtools and retained:
        raise RuntimeError(f'Frontend implementations remain linked: {list(retained)}')
    return {'native_v8_inspector': native_inspector,
            'frontend_implementations': retained,
            'checked_frontend_patterns': list(frontend)}


def runtime_fingerprint(moli):
    paths = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '--',
         'Cargo.toml', 'Cargo.lock', 'moli-types', 'moli-core', 'moli-renderer-v8',
         'moli-protocol/Cargo.toml'], cwd=moli, text=True).splitlines()
    digest = hashlib.sha256()
    for name in sorted(set(paths)):
        path = moli / name
        if path.suffix == '.rs' or path.name in ('Cargo.toml', 'Cargo.lock'):
            digest.update(name.encode() + b'\0' + path.read_bytes() + b'\0')
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--limo-root', type=Path,
                        help='Limo checkout for release size and runtime comparison; optional with --check-only')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--online', action='store_true')
    parser.add_argument('--order', choices=('on-first', 'off-first'), default='off-first')
    parser.add_argument('--skip-contracts', action='store_true', help='Reuse previously checked API contracts')
    parser.add_argument('--check-only', action='store_true', help='Check feature/API contracts without linking')
    args = parser.parse_args()
    if args.limo_root is None and not args.check_only:
        parser.error('--limo-root is required for the release comparison; use --check-only for standalone API checks')
    moli = Path(__file__).resolve().parent.parent
    source_fingerprint = runtime_fingerprint(moli)
    limo = args.limo_root.resolve() if args.limo_root else None
    target = limo / 'target' if limo else Path(os.environ.get('CARGO_TARGET_DIR', moli / 'target'))
    output = (args.output or target / 'devtools-ab').resolve()
    app = output / 'app'
    (app / 'src').mkdir(parents=True, exist_ok=True)
    (output / 'tmp').mkdir(exist_ok=True)
    if limo:
        for source in ('main.rs', 'server.rs'):
            shutil.copyfile(limo / 'src' / source, app / 'src' / source)
        shutil.copyfile(moli / 'tools/devtools-size-probe.rs', app / 'src/devtools_probe.rs')
        manifest = (limo / 'Cargo.toml').read_text().split('[patch.crates-io]')[0]
    else:
        (app / 'src/main.rs').write_text('fn main() {}\n')
        manifest = '''[package]
name = "moli-devtools-contracts"
version = "0.0.0"
edition = "2024"

[dependencies]
moli-core = { default-features = false }

[profile.release]
panic = "abort"
codegen-units = 1
'''
    lines = manifest.splitlines()
    for i, line in enumerate(lines):
        if line.startswith('moli-core = '):
            lines[i] = f'moli-core = {{ path = "{moli / "moli-core"}", default-features = false }}'
    # Append the dependency before the release profile rather than under it.
    dependency_end = next(i for i, line in enumerate(lines) if line.startswith('[profile.'))
    lines.insert(dependency_end, f'moli-command-types = {{ path = "{moli / "moli-types/moli-command-types"}", default-features = false }}\n')
    lines.insert(dependency_end, f'moli-renderer-v8 = {{ path = "{moli / "moli-renderer-v8"}", default-features = false }}\n')
    manifest = '\n'.join(lines) + '\n\n[workspace]\n\n[features]\ndefault = []\ndevtools = ["moli-core/devtools", "moli-command-types/devtools"]\ntypes-devtools-only = ["moli-command-types/devtools"]\nrenderer-devtools-only = ["moli-renderer-v8/devtools"]\nace-only = ["moli-core/ace-protocol"]\n\n'
    patches = (moli / 'Cargo.toml').read_text()
    patches = patches[patches.index('[patch.crates-io]'):patches.index('[workspace.lints.clippy]')]
    patches = patches.replace('path = "vendor/', f'path = "{moli}/vendor/')
    manifest += patches
    if limo:
        manifest += '\n[[bin]]\nname = "devtools-probe"\npath = "src/devtools_probe.rs"\n'
    contracts = {
        'runtime-contract': '''fn main() {
    let _ = moli_core::page::Page::start_evaluate_runtime_expression;
    let _ = moli_core::page::Page::start_capture_screenshot;
    let _ = moli_core::page::Page::start_print_to_pdf;
    let _ = moli_core::runtime::Browser::wait_for_selector;
    let _ = moli_core::runtime::Browser::wait_for_script_truthy;
    let _: Option<moli_command_types::RuntimeRealmInfo> = None;
}''',
        'devtools-types-contract': '''use moli_command_types::devtools::{InspectorCommand, InspectorCommandHandles, NativeProtocolResponse};
struct Handles;
impl InspectorCommandHandles for Handles {
    type ResponseSender = ();
    type RuntimeOutput = ();
    type SessionDetachGuard = ();
}
fn main() {
    let _: InspectorCommand<Handles> = InspectorCommand::RuntimeEnableEvents;
    let _ = NativeProtocolResponse::<()>::success(Default::default());
}''',
        'devtools-api-contract': '''fn main() {
    let _ = moli_core::RendererCdpCall::new;
    let _ = moli_core::RendererPageCommand::Inspector;
    let _ = moli_core::RendererPageCommand::Native;
    let _ = moli_core::RendererPageCommand::dispatch_runtime_protocol_message;
    let _ = moli_core::page::Page::start_runtime_protocol_message;
}''',
        'devtools-dom-contract': '''fn main() {
    let _ = moli_core::RendererPageCommand::DocumentBidiNodeBinding {
        inspector_session_id: None,
        shared_id: String::new(),
    };
    let _ = moli_core::RendererPageCommand::DocumentPerformSearch {
        inspector_session_id: None,
        query: String::new(),
        include_user_agent_shadow_dom: false,
        include_whitespace: false,
    };
}''',
        'devtools-worker-contract': '''struct Handles;
impl moli_command_types::devtools::WorkerCommandHandles for Handles {
    type SessionLifetime = ();
    type ResponseSender = ();
    type ProtocolResponseSender = ();
}
fn main() {
    let _: moli_command_types::devtools::WorkerCommand<Handles> =
        moli_command_types::devtools::WorkerCommand::RunIfWaitingForDebugger;
}''',
    }
    removed_apis = {
        'native-call': ('moli_core::RendererCdpCall::new', 'RendererCdpCall'),
        'inspector-command': ('moli_core::RendererPageCommand::Inspector', 'Inspector'),
        'native-command': ('moli_core::RendererPageCommand::Native', 'Native'),
        'raw-dispatch': ('moli_core::RendererPageCommand::dispatch_runtime_protocol_message', 'dispatch_runtime_protocol_message'),
        'page-dispatch': ('moli_core::page::Page::start_runtime_protocol_message', 'start_runtime_protocol_message'),
        'input': ('moli_core::page::Page::start_dispatch_key_event_with_location_outcome', 'start_dispatch_key_event_with_location_outcome'),
        'network': ('moli_core::page::Page::start_set_fetch_subresource_interception', 'start_set_fetch_subresource_interception'),
    }
    for name, (expression, _) in removed_apis.items():
        contracts[f'devtools-{name}-contract'] = f'fn main() {{ let _ = {expression}; }}'
    for name, source in contracts.items():
        (app / f'src/{name}.rs').write_text(source + '\n')
        manifest += f'\n[[bin]]\nname = "{name}"\npath = "src/{name}.rs"\n'
    (app / 'Cargo.toml').write_text(manifest)
    if not (app / 'Cargo.lock').exists():
        shutil.copyfile((limo or moli) / 'Cargo.lock', app / 'Cargo.lock')

    scratch = os.environ.get('TMPDIR') or str(output / 'tmp')
    env = dict(os.environ, CARGO_TARGET_DIR=str(target),
               TMPDIR=scratch, TMP=scratch, TEMP=scratch,
               no_proxy='127.0.0.1,localhost,::1',
               NO_PROXY='127.0.0.1,localhost,::1')
    if limo:
        env['LIMO_TEST_ROOT'] = str(limo)

    def run(command, name, timeout=None):
        print(' '.join(str(x) for x in command), flush=True)
        with (output / f'{name}.log').open('w') as log:
            subprocess.run(command, cwd=moli, env=env, stdout=log,
                           stderr=subprocess.STDOUT, check=True, timeout=timeout)

    common = ([] if args.check_only else ['--release']) + ['--manifest-path', str(app / 'Cargo.toml'),
              '--no-default-features', '--jobs', str(args.jobs)]
    if not args.online:
        common += ['--offline']
    if not args.skip_contracts:
        run(['cargo', 'check', *common, '--bin', 'runtime-contract'], 'contract-runtime-off')
        for feature in ['types-devtools-only', 'renderer-devtools-only', 'ace-only']:
            run(['cargo', 'check', *common, '--bin', 'runtime-contract', '--features', feature],
                f'contract-runtime-{feature}')
        negative_contracts = [('devtools-types-contract', 'moli_command_types::devtools'),
                               ('devtools-api-contract', 'RendererCdpCall'),
                               ('devtools-dom-contract', 'DocumentBidiNodeBinding'),
                               ('devtools-worker-contract', 'RunIfWaitingForDebugger')]
        negative_contracts += [(f'devtools-{name}-contract', expected) for name, (_, expected) in removed_apis.items()]
        for name, expected in negative_contracts:
            run(['cargo', 'check', *common, '--bin', name, '--features', 'devtools'], f'contract-{name}-on')
            with (output / f'contract-{name}-off.log').open('w') as log:
                result = subprocess.run(['cargo', 'check', *common, '--bin', name], cwd=moli,
                                        env=env, stdout=log, stderr=subprocess.STDOUT)
            diagnostic = (output / f'contract-{name}-off.log').read_text()
            if result.returncode == 0 or expected not in diagnostic:
                raise RuntimeError(f'{name}: expected an unavailable DevTools API in the off build; see {output}')
    if args.check_only:
        print('Feature contracts passed: runtime APIs compile; DevTools types/APIs exist only with devtools.')
        return
    results = {}
    order = ('off', 'on') if args.order == 'off-first' else ('on', 'off')
    for mode in order:
        flags = ['--features', 'devtools'] if mode == 'on' else []
        tree = subprocess.check_output(
            ['cargo', 'tree', '--manifest-path', str(app / 'Cargo.toml'),
             '--no-default-features', '--prefix', 'none', '--edges', 'normal,build',
             '--format', '{p} {f}', *flags], cwd=moli, env=env, text=True)
        (output / f'dependencies-{mode}.txt').write_text(tree)
        if mode == 'off' and any(line.startswith('moli-protocol-cdp v') for line in tree.splitlines()):
            raise RuntimeError('The off consumer still depends on moli-protocol-cdp')
        unstripped = output / f'limo-{mode}.unstripped'
        binary = output / f'limo-{mode}'
        run(['cargo', 'rustc', *common, '--bin', 'limo', *flags, '--',
             f'--emit=link={unstripped}', f'-C', f'link-arg=-Wl,-Map={output / f"limo-{mode}.map"}'],
            f'build-{mode}')
        if runtime_fingerprint(moli) != source_fingerprint:
            raise RuntimeError('Runtime source changed during the build; rerun on a frozen checkout')
        subprocess.run(['strip', '--strip-all', '-o', str(binary), str(unstripped)], check=True)
        run(['cargo', 'build', *common, '--bin', 'devtools-probe', *flags], f'probe-build-{mode}')
        shutil.copy2(limo / 'target/release/devtools-probe', output / f'probe-{mode}')
        env['MOLI_PROBE_ARTIFACT_DIR'] = str(output / f'probe-artifacts-{mode}')
        run([str(output / f'probe-{mode}')], f'probe-{mode}', timeout=30)
        env['LIMO_TEST_BIN'] = str(binary)
        run([sys.executable, str(moli / 'tools/devtools-size-smoke.py')], f'smoke-{mode}', timeout=180)
        last = json.loads((output / f'smoke-{mode}.log').read_text().splitlines()[-1])
        smoke = json.loads(Path(last['report']).read_text())
        sections = elf_sections(binary)
        with (output / f'symbols-{mode}.txt').open('w') as log:
            subprocess.run(['nm', '-S', '--size-sort', '--demangle', str(unstripped)],
                           stdout=log, check=True)
        audit = audit_symbols(output / f'symbols-{mode}.txt', mode == 'on')
        results[mode] = {'bytes': binary.stat().st_size,
                         'mib': round(binary.stat().st_size / 2**20, 4),
                         'unstripped_bytes': unstripped.stat().st_size,
                         'sections': sections, 'smoke': smoke,
                         'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                         'symbol_audit': audit,
                         'probe': (output / f'probe-{mode}.log').read_text()}
        (output / 'comparison.json').write_text(json.dumps(results, indent=2))
    if runtime_fingerprint(moli) != source_fingerprint:
        raise RuntimeError('Runtime source changed during the comparison; rerun on a frozen checkout')
    png_hashes = {mode: hashlib.sha256((output / f'probe-artifacts-{mode}/probe.png').read_bytes()).hexdigest()
                  for mode in ('on', 'off')}
    if len(set(png_hashes.values())) != 1:
        raise RuntimeError('Screenshot output changed with DevTools disabled')
    results['png_sha256'] = png_hashes
    results['moli_runtime_source_sha256'] = source_fingerprint
    results['moli_checkout'] = str(moli)
    results['limo_revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=limo, text=True).strip()
    results['limo_source_sha256'] = {name: hashlib.sha256((app / 'src' / name).read_bytes()).hexdigest()
                                     for name in ('main.rs', 'server.rs', 'devtools_probe.rs')}
    results['compiler'] = subprocess.check_output(['rustc', '--version'], cwd=moli, text=True).strip()
    results['cargo_lock_sha256'] = hashlib.sha256((app / 'Cargo.lock').read_bytes()).hexdigest()
    results['moli_patch_sha256'] = hashlib.sha256(subprocess.check_output(['git', 'diff', 'HEAD', '--', 'Cargo.toml', 'moli-types', 'moli-core', 'moli-renderer-v8', 'moli-protocol'], cwd=moli)).hexdigest()
    results['scope'] = 'Compile-time external DevTools command types, dispatch and protocol adapters; V8 Inspector, host evaluation, screenshots and PDF are preserved.'
    results['moli_revision'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=moli, text=True).strip()
    results['saved_bytes'] = results['on']['bytes'] - results['off']['bytes']
    results['saved_mib'] = round(results['saved_bytes'] / 2**20, 4)
    results['saved_percent'] = round(100 * results['saved_bytes'] / results['on']['bytes'], 3)
    results['behavior_differences'] = [a['name'] for a, b in zip(results['on']['smoke']['results'], results['off']['smoke']['results'])
                                      if a['returncode'] != b['returncode'] or a['passed'] != b['passed']]
    (output / 'comparison.json').write_text(json.dumps(results, indent=2))
    if results['behavior_differences']:
        raise SystemExit(f"Runtime regressions: {results['behavior_differences']}; see {output / 'comparison.json'}")
    print(json.dumps({key: results[key] for key in ('moli_revision', 'saved_bytes', 'saved_mib', 'saved_percent', 'behavior_differences')}, indent=2))


if __name__ == '__main__':
    main()
