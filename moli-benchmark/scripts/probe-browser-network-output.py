"""Verify native Network throughput and Log delivery on a pinned release.

Run with the benchmark package on PYTHONPATH, once with and without --log-enabled.
Uses 32 concurrent 4 MiB bodies and the existing 20-second public CDP deadline.
Retains all wire events and server failures, including unsuccessful attempts.
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
from moli_benchmark.target_serve import start_target_serve, stop_target_serve


class Fixture(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    block = b'x' * 16384
    requests = []
    failures = []

    def do_GET(self):
        path = self.path
        self.requests.append(path)
        try:
            if path.startswith('/body/'):
                size = 4 * 1024 * 1024
                self.send_response(200)
                self.send_header('Content-Type', 'application/octet-stream')
                self.send_header('Content-Length', str(size))
                self.send_header('Cache-Control', 'no-store')
                self.end_headers()
                for _ in range(size // len(self.block)):
                    self.wfile.write(self.block)
            elif path.startswith('/missing/'):
                data = b'missing'
                self.send_response(404)
                self.send_header('Content-Length', str(len(data)))
                self.send_header('Cache-Control', 'no-store')
                self.end_headers()
                self.wfile.write(data)
            else:
                data = b'<!doctype html><title>network throughput</title>'
                self.send_response(200)
                self.send_header('Content-Type', 'text/html')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError) as error:
            self.failures.append({'path': path, 'error': repr(error)})

    def log_message(self, *_args):
        pass


async def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    for key in ('ALL_PROXY', 'HTTP_PROXY', 'HTTPS_PROXY', 'all_proxy', 'http_proxy', 'https_proxy'):
        os.environ.pop(key, None)
    os.environ['SPLIT3_TRACE_FILE'] = str((args.output / 'trace.log').resolve())
    report = {'binary': str(args.binary.resolve()), 'sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'requests': 32, 'bytes_per_request': 4 * 1024 * 1024, 'log_enabled': args.log_enabled, 'probe_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    server = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    serve = client = None
    try:
        serve = start_target_serve('moli-full-cdp', args.binary.resolve(), 20,
                                   ('--http-cache-dir', str((args.output / 'cache').resolve())))
        client = await connect_routed_raw_cdp(serve.endpoint)

        async def command(method, params=None, session=None):
            result = await client.command(method, params, session_id=session, timeout=20)
            assert 'error' not in result.response, result.response
            return result.response['result']

        target = (await command('Target.createTarget', {'url': 'about:blank'}))['targetId']
        session = (await command('Target.attachToTarget', {'targetId': target, 'flatten': True}))['sessionId']
        for domain in ('Page', 'Runtime', 'Network'):
            await command(domain + '.enable', session=session)
        if args.log_enabled:
            await command('Log.enable', session=session)
        before = client.current_sequence
        url = f'http://127.0.0.1:{server.server_port}/'
        await command('Page.navigate', {'url': url}, session)
        await client.wait_for_event('Page.loadEventFired', after_sequence=before, session_id=session, timeout=20)
        await command('HeapProfiler.moliDiagnostics', session=session)
        before = client.current_sequence
        started = time.perf_counter()
        result = await command('Runtime.evaluate', {
            'expression': "Promise.all(Array.from({length:32},(_,i)=>fetch('/body/'+i).then(r=>r.arrayBuffer()).then(b=>[b.byteLength,new Uint8Array(b)[0],new Uint8Array(b)[b.byteLength-1]])))",
            'awaitPromise': True, 'returnByValue': True,
        }, session)
        report['elapsed_ms'] = (time.perf_counter() - started) * 1000
        assert 'exceptionDetails' not in result, result
        assert result['result']['value'] == [[4194304,120,120]] * 32, result
        await command('HeapProfiler.moliDiagnostics', session=session)
        messages = [m.payload for m in client.messages_since(before) if m.payload.get('sessionId') == session]
        starts = [m['params'] for m in messages if m.get('method') == 'Network.requestWillBeSent' and '/body/' in m['params']['request']['url']]
        ids = {m['requestId'] for m in starts}
        terminals = [m for m in messages if m.get('method') in ('Network.loadingFinished','Network.loadingFailed') and m['params']['requestId'] in ids]
        report['network_starts'] = len(starts)
        report['network_terminals'] = len(terminals)
        report['data_events'] = sum(m.get('method') == 'Network.dataReceived' and m['params']['requestId'] in ids for m in messages)
        assert len(starts) == len(ids) == len(terminals) == 32, report
        assert all(m['method'] == 'Network.loadingFinished' for m in terminals), terminals
        for request_id in ids:
            facts = [m for m in messages if m.get('params', {}).get('requestId') == request_id]
            heads = [i for i, m in enumerate(facts) if m.get('method') == 'Network.responseReceived']
            ends = [i for i, m in enumerate(facts) if m.get('method') == 'Network.loadingFinished']
            data = [(i, m['params']['dataLength']) for i, m in enumerate(facts)
                    if m.get('method') == 'Network.dataReceived']
            assert len(heads) == len(ends) == 1, (request_id, heads, ends)
            assert sum(size for _, size in data) == report['bytes_per_request'], request_id
            assert all(heads[0] < i < ends[0] for i, _ in data), request_id
        report['wire_bytes'] = len(ids) * report['bytes_per_request']
        secondary = (await command('Target.attachToTarget', {'targetId': target, 'flatten': True}))['sessionId']

        def errors_since(sequence):
            return [(m.payload.get('sessionId'), m.payload['params']['entry']['url'])
                    for m in client.messages_since(sequence)
                    if m.payload.get('method') == 'Log.entryAdded'
                    and '/missing/' in m.payload['params']['entry'].get('url', '')]

        async def missing(name):
            result = await command('Runtime.evaluate', {
                'expression': "fetch('/missing/" + name + "').then(r=>r.text())",
                'awaitPromise': True, 'returnByValue': True,
            }, session)
            assert result['result']['value'] == 'missing', result

        before_error = client.current_sequence
        await missing('first')
        assert errors_since(before_error) == ([(session, url + 'missing/first')] if args.log_enabled else []), errors_since(before_error)
        before_replay = client.current_sequence
        await command('Log.enable', session=secondary)
        assert errors_since(before_replay) == [(secondary, url + 'missing/first')], errors_since(before_replay)
        await command('Log.clear', session=session)
        before_enable = client.current_sequence
        await command('Log.enable', session=session)
        assert errors_since(before_enable) == [], errors_since(before_enable)
        before_second = client.current_sequence
        await missing('second')
        assert sorted(errors_since(before_second)) == sorted([(s, url + 'missing/second') for s in (session, secondary)]), errors_since(before_second)
        await command('Log.disable', session=secondary)
        before_third = client.current_sequence
        await missing('third')
        assert errors_since(before_third) == [(session, url + 'missing/third')], errors_since(before_third)
        before_reenable = client.current_sequence
        await command('Log.enable', session=secondary)
        assert errors_since(before_reenable) == [(secondary, url + 'missing/' + suffix) for suffix in ('second','third')], errors_since(before_reenable)
        report['log_checks'] = ['live subscribers', 'late enable', 'target-shared clear', 'two-session live fanout', 'disable', 'reenable replay']
        report['ok'] = True
    except Exception as error:
        report.update(ok=False, error=repr(error))
    finally:
        if client is not None:
            (args.output / 'wire.json').write_text(json.dumps([m.payload for m in client.messages_since(0)]))
            await client.close()
        if serve is not None:
            stopped = stop_target_serve(serve)
            (args.output / 'server.log').write_text(str(stopped))
        server.shutdown()
        server.server_close()
        thread.join(5)
        report.update(physical_requests=Fixture.requests, server_failures=Fixture.failures)
        if Fixture.failures or thread.is_alive():
            report.update(ok=False, fixture_error='request failure or server thread did not stop')
        (args.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report), flush=True)
    return report['ok']


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--log-enabled', action='store_true')
args = parser.parse_args()
raise SystemExit(0 if asyncio.run(run(args)) else 1)
