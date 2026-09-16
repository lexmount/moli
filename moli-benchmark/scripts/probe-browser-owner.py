"""Measure public commit/output timing and retained output on a pinned Moli CLI.

Wire timings are not BrowserOwner queue residence or internal projection lag.
The optional measurement patch records those separately in owner-trace.log.
Run with the benchmark package on PYTHONPATH.
"""
import argparse
import asyncio
import hashlib
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from moli_benchmark.raw_cdp import connect_routed_raw_cdp
from moli_benchmark.sampling import snapshot_resources
from moli_benchmark.target_serve import start_target_serve, stop_target_serve


class Fixture(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/worker.js':
            mime = 'text/javascript'
            body = '''let emitted = 0;
            onconnect = event => {
              const port = event.ports[0];
              port.onmessage = event => {
                const payload = 'x'.repeat(8192);
                for (let i = 0; i < event.data; i++) console.log(emitted++, payload);
                port.postMessage({count: event.data, total: emitted});
              };
              port.start();
            };'''
        else:
            mime = 'text/html'
            body = ('<!doctype html><title>owner probe</title><script>'
                    f'globalThis.marker={json.dumps(self.path)};'
                    'console.log("split3-nav", marker);</script><p>' + 'x' * 2048)
        encoded = body.encode()
        self.send_response(200)
        self.send_header('Content-Type', mime)
        self.send_header('Content-Length', str(len(encoded)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_args):
        pass


def host_snapshot():
    counters = {}
    for line in Path('/proc/vmstat').read_text().splitlines():
        key, value = line.split()
        if key in ('pswpin', 'pswpout'):
            counters[key] = int(value)
    return {'monotonic': time.monotonic(), 'load_average': os.getloadavg(),
            'swap_pages': counters}


async def probe(args):
    args.output.mkdir(parents=True, exist_ok=False)
    for key in ('ALL_PROXY', 'HTTP_PROXY', 'HTTPS_PROXY', 'all_proxy', 'http_proxy', 'https_proxy'):
        os.environ.pop(key, None)
    os.environ['SPLIT3_TRACE_FILE'] = str((args.output / 'owner-trace.log').resolve())
    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    origin = f'http://127.0.0.1:{server.server_port}'
    serve = client = None
    with args.binary.open('rb') as binary:
        digest = hashlib.file_digest(binary, 'sha256').hexdigest()
    report = {'binary': str(args.binary.resolve()), 'sha256': digest,
              'targets': args.targets, 'navigations_per_target': args.navigations,
              'output_batches': args.output_batches, 'console_payload_bytes': 8192,
              'host_before': host_snapshot(),
              'navigation': [], 'history_commands_ms': [], 'retention': []}
    try:
        serve = start_target_serve('moli-full-cdp', args.binary.resolve(), 20,
                                   ('--http-cache-dir', str((args.output / 'http-cache').resolve())))
        client = await connect_routed_raw_cdp(serve.endpoint)

        async def command(method, params=None, session=None):
            result = await client.command(method, params, session_id=session, timeout=20)
            return result.response['result']

        context = (await command('Target.createBrowserContext'))['browserContextId']
        await command('Target.setDiscoverTargets', {'discover': True})
        pages = []
        for _ in range(args.targets):
            target = (await command('Target.createTarget', {'url': 'about:blank', 'browserContextId': context}))['targetId']
            session = (await command('Target.attachToTarget', {'targetId': target, 'flatten': True}))['sessionId']
            await command('Page.enable', session=session)
            await command('Runtime.enable', session=session)
            pages.append((target, session))

        async def navigate(page_index, turn):
            _, session = pages[page_index]
            path = f'/page/{page_index}/{turn}'
            before = client.current_sequence
            started = time.perf_counter()
            response = await client.command('Page.navigate', {'url': origin + path}, session_id=session, timeout=20)
            assert 'errorText' not in response.response['result'], response.response
            frame = await client.wait_for_event('Page.frameNavigated', after_sequence=before, session_id=session,
                                               predicate=lambda p: p.get('params', {}).get('frame', {}).get('url') == origin + path, timeout=20)
            output = await client.wait_for_event('Runtime.consoleAPICalled', after_sequence=before, session_id=session,
                                                predicate=lambda p: [v.get('value') for v in p.get('params', {}).get('args', [])] == ['split3-nav', path], timeout=20)
            assert frame.sequence < output.sequence, 'new document output must follow its frame projection'
            value = await command('Runtime.evaluate', {'expression': 'marker', 'returnByValue': True}, session)
            assert value['result']['value'] == path, value
            report['navigation'].append({'page': page_index, 'turn': turn,
                                         'command_ms': response.elapsed_ms,
                                         'frame_observed_ms': (frame.received_monotonic - started) * 1000,
                                         'output_observed_ms': (output.received_monotonic - started) * 1000,
                                         'frame_sequence': frame.sequence, 'output_sequence': output.sequence})

        for turn in range(-2, args.navigations):
            await asyncio.gather(*(navigate(index, turn) for index in range(args.targets)))
        for _ in range(4):
            reads = await asyncio.gather(*(client.command('Page.getNavigationHistory', session_id=pages[i % args.targets][1]) for i in range(64)))
            for read in reads:
                assert read.response['result']['entries'], read.response
                report['history_commands_ms'].append(read.elapsed_ms)

        async def snapshot(label):
            report['retention'].append({'label': label, 'resources': snapshot_resources(serve.process.pid),
                                        'diagnostics': await command('HeapProfiler.moliDiagnostics', session=pages[0][1]),
                                        'page_v8_heap': await command('Runtime.getHeapUsage', session=pages[0][1])})
            calls = args.output / 'owner-trace.calls.txt'
            if calls.exists():
                report['retention'][-1]['owner_operation_counts'] = {
                    name: int(count) for count, name in
                    (line.split(' ', 1) for line in calls.read_text().splitlines())}

        await snapshot('before-worker-output')
        await snapshot('idle-snapshot-control')
        worker_session = pages[-1][1]

        async def emit_worker(count):
            expression = f'''new Promise(resolve => {{
                const worker = globalThis.probeWorker ||= new SharedWorker('/worker.js', 'output-retention');
                worker.port.onmessage = event => resolve(event.data);
                worker.port.start(); worker.port.postMessage({count});
            }})'''
            result = await command('Runtime.evaluate', {'expression': expression, 'awaitPromise': True, 'returnByValue': True}, worker_session)
            assert result['result']['value']['count'] == count, result
            return result['result']['value']['total']

        total = 0
        for count in args.output_batches:
            emitted = await emit_worker(count)
            assert emitted == total + count, (emitted, total, count)
            total = emitted
            await snapshot(f'worker-output-total-{total}')
        targets = (await command('Target.getTargets'))['targetInfos']
        worker = next(t['targetId'] for t in targets if t['type'] == 'shared_worker' and t['url'] == origin + '/worker.js')
        observer = (await command('Target.attachToTarget', {'targetId': worker, 'flatten': True}))['sessionId']
        report['creator_v8_heap_before_gc'] = await command('Runtime.getHeapUsage', session=worker_session)
        report['worker_v8_heap_before_gc'] = await command('Runtime.getHeapUsage', session=observer)
        await command('HeapProfiler.collectGarbage', session=observer)
        await command('HeapProfiler.collectGarbage', session=worker_session)
        report['creator_v8_heap_after_gc'] = await command('Runtime.getHeapUsage', session=worker_session)
        report['worker_v8_heap_after_gc'] = await command('Runtime.getHeapUsage', session=observer)
        await snapshot('worker-observer-attached-after-gc')
        before = client.current_sequence
        await command('Runtime.enable', session=observer)
        await client.wait_for_event('Runtime.consoleAPICalled', after_sequence=before, session_id=observer,
                                    predicate=lambda p: p['params']['args'][0].get('value') == total - 1, timeout=20)
        replayed = [message.payload['params']['args'][0]['value']
                    for message in client.messages_since(before)
                    if message.payload.get('method') == 'Runtime.consoleAPICalled'
                    and message.payload.get('sessionId') == observer]
        assert replayed == list(range(total - len(replayed), total)), replayed
        report['worker_replayed_records'] = len(replayed)
        report['worker_replayed_range'] = [replayed[0], replayed[-1]]
        await snapshot('worker-observer-enabled')
        before = client.current_sequence
        live_total = await emit_worker(256)
        assert live_total == total + 256, live_total
        await client.wait_for_event('Runtime.consoleAPICalled', after_sequence=before, session_id=observer,
                                    predicate=lambda p: p['params']['args'][0].get('value') == live_total - 1, timeout=20)
        live = [message.payload['params']['args'][0]['value']
                for message in client.messages_since(before)
                if message.payload.get('method') == 'Runtime.consoleAPICalled'
                and message.payload.get('sessionId') == observer]
        assert live == list(range(total, live_total)), live
        report['worker_live_records'] = len(live)
        await snapshot('worker-observer-live-output')
        await command('Target.detachFromTarget', {'sessionId': observer})
        before = client.current_sequence
        assert (await command('Target.closeTarget', {'targetId': pages[-1][0]}))['success']
        await client.wait_for_event('Target.targetDestroyed', after_sequence=before,
                                    predicate=lambda p: p['params']['targetId'] == worker, timeout=20)
        await snapshot('after-worker-owner-close')
        await command('Target.disposeBrowserContext', {'browserContextId': context})
        report['after_context_disposal'] = snapshot_resources(serve.process.pid)
        report['ok'] = True
    except BaseException as error:
        report['ok'] = False
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        if client is not None:
            (args.output / 'wire.json').write_text(json.dumps(client.recorded_messages()) + '\n')
            await client.close()
        if serve is not None:
            report['serve'] = stop_target_serve(serve, include_resource_samples=True)
            (args.output / 'server.log').write_text('\n'.join(serve.logs) + '\n')
        await asyncio.to_thread(server.shutdown)
        server.server_close()
        report['host_after'] = host_snapshot()
        (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'ok': report['ok'], 'output': str(args.output), 'navigations': len(report['navigation'])}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--targets', type=int, default=4)
    parser.add_argument('--navigations', type=int, default=25)
    parser.add_argument('--output-batches', type=int, nargs='+', default=[512, 1536, 2048])
    args = parser.parse_args()
    if args.targets < 2 or args.navigations < 1:
        parser.error('at least two targets and one navigation are required')
    if any(count < 1 for count in args.output_batches):
        parser.error('output batches must contain positive counts')
    asyncio.run(probe(args))
