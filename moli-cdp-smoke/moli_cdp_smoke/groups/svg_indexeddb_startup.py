from __future__ import annotations

import json
from typing import Any

from ..assertions import assert_equal, record_contract
from ..config import REPO_ROOT


async def run_svg_indexeddb_startup_group(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        lifecycle_fixture = REPO_ROOT / "moli-renderer-v8/tests/fixtures/indexeddb-connection-lifecycle"
        observed = await page.evaluate(lifecycle_fixture.with_suffix(".js").read_text())
        expected = json.loads(lifecycle_fixture.with_suffix(".json").read_text())
        assert_equal(observed, expected, "connection queue admission and upgrade terminal lifecycle")
        record_contract(
            results, "indexeddb_connection_queue_lifecycle",
            contract=("Database upgrade association clears before terminal events, while the open "
                      "request retains its transaction during dispatch. All open/delete calls share "
                      "FIFO admission, versions resolve at the head, and versionchange microtasks "
                      "finish before checking whether the request is blocked."),
            source="Chromium IndexedDB connection coordinator and upgrade lifecycle WPT",
            commands=["Runtime.evaluate"], observed=observed,
        )
        # A fresh browser context keeps the worker's database namespace empty.
        # Worker task wakes use a different route from Window Page tasks, but
        # must exercise the same admission/lifecycle implementation.
        worker_context = await browser.new_context()
        try:
            worker_page = await worker_context.new_page()
            await worker_page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
            observed = await worker_page.evaluate("""source => new Promise((resolve, reject) => {
              const program = source + '.then(value => postMessage({value}), error => postMessage({error: String(error)}));';
              const url = URL.createObjectURL(new Blob([program], {type: 'application/javascript'}));
              const worker = new Worker(url);
              const cleanup = () => { worker.terminate(); URL.revokeObjectURL(url); };
              worker.onmessage = event => {
                cleanup();
                if (event.data.error) reject(new Error(event.data.error));
                else resolve(event.data.value);
              };
              worker.onerror = event => { cleanup(); reject(new Error(event.message)); };
            })""", lifecycle_fixture.with_suffix(".js").read_text().strip())
            assert_equal(observed, expected, "worker connection queue and upgrade lifecycle")
            record_contract(
                results, "indexeddb_worker_connection_queue_lifecycle",
                contract="Worker task wakes preserve the same connection admission, terminal ordering, and FIFO recovery as Window tasks.",
                source="Shared Chromium-calibrated IndexedDB connection lifecycle fixture",
                commands=["Runtime.evaluate"], observed=observed,
            )
        finally:
            await worker_context.close()
        upgrade_script = (REPO_ROOT / "moli-renderer-v8/tests/fixtures/indexeddb-upgrade.js").read_text()
        observed = await page.evaluate(upgrade_script)
        assert_equal(observed, {
            "trace": ["upgrade:done", "seed", "read:original", "request-microtask",
                      "migration-write", "microtask-write", "complete", "complete-microtask", "open-success"],
            "transactionCleared": True,
            "records": [{"id": 1, "text": "original"}, {"id": 2, "text": "microtask write"}],
            "microtaskValue": "value",
            "abortTrace": ["request-success", "abort", "open-error:AbortError"],
            "rollback": {"oldVersion": 0, "stores": []},
            "closedResult": "AbortError",
        }, "upgrade request callbacks/microtasks drain before commit; abort rolls back")
        record_contract(
            results, "indexeddb_upgrade_request_drain",
            contract=("Upgrade request callbacks and their microtasks can migrate schema and data; "
                      "the open result follows transaction completion, while abort rolls back and closes the connection."),
            source="Chromium 145.0.7632.116 executable calibration",
            commands=["Runtime.evaluate"], observed=observed,
        )

        observed = await page.evaluate("""() => new Promise((resolve, reject) => {
          const order = [];
          const open = indexedDB.open('startup-delete-order', 1);
          const remove = label => {
            const request = indexedDB.deleteDatabase(open.result.name);
            request.onerror = () => reject(request.error);
            request.onsuccess = () => {
              order.push(label);
              if (order.length === 2) resolve(order);
            };
          };
          open.onerror = () => reject(open.error);
          open.onupgradeneeded = () => remove('delete1');
          open.onsuccess = () => {
            remove('delete2');
            open.result.close();
          };
        })""")
        assert_equal(observed, ["delete1", "delete2"], "blocked deletes preserve arrival order")
        record_contract(
            results, "indexeddb_upgrade_delete_order",
            contract=("A delete queued during upgrade remains ahead of a delete requested by "
                      "open.success, even if both become runnable after the connection closes."),
            source="Chromium executable calibration; renderer initial-upgrade deletion regression",
            commands=["Runtime.evaluate"], observed=observed,
        )

        # Error strings deliberately exist in the DOM before startup. Neither
        # their presence nor a large text dump establishes application failure
        # or success: inspect the selected panel and await database completion.
        await page.set_content("""<!doctype html>
          <style>.off { display:none }</style>
          <section id="no-indexeddb" class="off">Your browser does not support IndexedDB</section>
          <section id="not-supported" class="off">Your browser does not support SVG or Base64</section>
          <section id="internal-error" class="off">The app could not be started</section>
          <main id="app" class="off"></main>""")
        observed = await page.evaluate("""async () => {
          const trace = [];
          const requestResult = request => new Promise((resolve, reject) => {
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
          });
          const transactionDone = tx => new Promise((resolve, reject) => {
            tx.oncomplete = resolve;
            tx.onabort = () => reject(tx.error || new Error('transaction aborted'));
          });
          const failPanel = id => { document.getElementById(id).classList.remove('off'); };
          const supportsSVG = !!document.createElementNS &&
            !!document.createElementNS('http://www.w3.org/2000/svg', 'svg').createSVGRect;
          if (!supportsSVG || !window.atob) { failPanel('not-supported'); return {trace}; }
          if (!('indexedDB' in window)) { failPanel('no-indexeddb'); return {trace}; }
          let db;
          try {
            const name = 'svg-indexeddb-startup';
            let open = indexedDB.open(name, 1);
            open.onupgradeneeded = event => {
              trace.push(`seed:${event.oldVersion}->${event.newVersion}`);
              const store = event.currentTarget.result.createObjectStore('legacy', {keyPath:'id'});
              store.createIndex('id', 'id', {unique:true});
              store.put({id:'drawing', content:'saved geometry'});
            };
            db = await requestResult(open);
            db.close();
            open = indexedDB.open(name, 4);
            open.onupgradeneeded = event => {
              trace.push(`upgrade:${event.oldVersion}->${event.newVersion}`);
              const database = event.currentTarget.result;
              const tx = event.currentTarget.transaction;
              const read = tx.objectStore('legacy').getAll();
              read.onsuccess = () => {
                trace.push('migration-read');
                const store = database.createObjectStore('drawings', {keyPath:'uuid'});
                store.createIndex('uuid', 'uuid', {unique:true});
                for (const record of read.result) store.put({uuid:record.id, content:record.content});
                database.deleteObjectStore('legacy');
              };
            };
            db = await requestResult(open);
            trace.push('opened');
            let tx = db.transaction('drawings', 'readwrite');
            let done = transactionDone(tx);
            tx.objectStore('drawings').put({uuid:'second', content:'new geometry'});
            await done;
            trace.push('written');
            db.close();
            db = await requestResult(indexedDB.open(name, 4));
            tx = db.transaction('drawings', 'readonly');
            done = transactionDone(tx);
            const value = await requestResult(tx.objectStore('drawings').index('uuid').get('drawing'));
            await done;
            const version = db.version;
            const stores = Array.from(db.objectStoreNames);
            db.close();
            trace.push('reopened-read');
            document.getElementById('app').classList.remove('off');
            document.getElementById('app').textContent = value.content;
            return {trace, version, stores, value};
          } catch (error) {
            db?.close();
            failPanel('internal-error');
            return {trace, error:`${error.name}: ${error.message}`};
          }
        }""")
        assert_equal(observed, {
            "trace": ["seed:0->1", "upgrade:1->4", "migration-read", "opened", "written", "reopened-read"],
            "version": 4, "stores": ["drawings"],
            "value": {"uuid": "drawing", "content": "saved geometry"},
        }, "SVG-gated IndexedDB startup, asynchronous upgrade, commit and reopen")
        panels = await page.evaluate("""() => ({
          hiddenErrors: ['no-indexeddb','not-supported','internal-error'].every(id =>
            getComputedStyle(document.getElementById(id)).display === 'none'),
          ready: !document.getElementById('app').classList.contains('off'),
          text: document.getElementById('app').textContent,
        })""")
        assert_equal(panels, {"hiddenErrors": True, "ready": True, "text": "saved geometry"},
                     "application readiness, not hidden error template text")
        record_contract(
            results, "svg_indexeddb_application_startup",
            contract=("SVG/Base64 feature admission and real IndexedDB upgrade/commit/reopen complete; "
                      "preexisting hidden error templates are not mistaken for a failed application."),
            source="Reduced sketchometry/JSXGraph startup; Chromium executable calibration",
            commands=["Runtime.evaluate", "Page.setDocumentContent"],
            observed={"database": observed, "panels": panels},
        )
    finally:
        await context.close()
