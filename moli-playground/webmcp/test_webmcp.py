"""Offline CLI integration checks against a release Moli and native WebMCP."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import subprocess
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

from webmcp_client import CdpPage, discovery, moli_server
from websockets.asyncio.client import connect

DIRECTORY = Path(__file__).resolve().parent
CLI = DIRECTORY / "webmcp.py"
PAGE = """<!doctype html><body>
<form toolname="first_form" tooldescription="Return the first form's message">
  <input name="message" required><button>Send first</button>
</form>
<form toolname="second_form" tooldescription="Return the second form's message">
  <input name="message" required><button>Send second</button>
</form>
<iframe src="/child"></iframe>
<script>
let count = 0;
let extraController;
const context = document.modelContext;
context.registerTool({name:'echo',description:'Echo with a session counter',
  inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text']},
  execute:({text})=>({text,count:++count})});
context.registerTool({name:'waiting',description:'Wait until cancelled',
  execute:(_,options)=>new Promise(resolve=>options.signal.addEventListener('abort',
    ()=>resolve('late result')))});
context.registerTool({name:'fail',description:'Return an execution error',
  execute:()=>{throw new Error('fixture failure')}});
context.registerTool({name:'add_extra',description:'Register an extra tool',
  execute:async()=>{
    extraController=new AbortController();
    await context.registerTool({name:'extra',description:'Dynamically registered tool',
      execute:()=>({extra:true})},{signal:extraController.signal});
    return 'added';
  }});
context.registerTool({name:'drop_extra',description:'Remove the extra tool',
  execute:()=>{extraController.abort();return 'removed'}});
for (const form of document.forms) form.addEventListener('submit',event=>{
  event.preventDefault();
  if (event.agentInvoked) event.respondWith({form:form.getAttribute('toolname'),
    message:form.elements.message.value});
});
</script>
"""
CHILD = """<!doctype html><script>
document.modelContext.registerTool({name:'echo',description:'Echo from a child frame',
  execute:({text})=>({text,child:true})});
</script>"""


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        content = (CHILD if self.path == "/child" else PAGE).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def log_message(self, _format, *_args) -> None:
        pass


def records(output: str) -> list:
    decoder = json.JSONDecoder()
    values = []
    while output.strip():
        output = output.lstrip()
        value, length = decoder.raw_decode(output)
        values.append(value)
        output = output[length:]
    return values


class WebMcpCliTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        # Match Cargo's local-test environment: fixtures must bypass host proxies.
        proxies = patch.dict(
            os.environ,
            {
                key: ""
                for key in (
                    "all_proxy",
                    "http_proxy",
                    "https_proxy",
                    "ALL_PROXY",
                    "HTTP_PROXY",
                    "HTTPS_PROXY",
                )
            },
        )
        proxies.start()
        cls.addClassCleanup(proxies.stop)
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.url = f"http://127.0.0.1:{cls.server.server_port}/"

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=3)

    def cli(
        self, *options: str, input_text: str | None = None, url: str | None = None
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(CLI),
                url or self.url,
                *options,
                "--timeout",
                "10",
                "--wait-for-tools",
                "0.2",
            ],
            input=input_text,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def test_catalog_and_one_shot_calls(self) -> None:
        listed = self.cli("list")
        self.assertEqual(listed.returncode, 0, listed.stderr)
        echoes = [tool for tool in json.loads(listed.stdout) if tool["name"] == "echo"]
        self.assertEqual(len(echoes), 2)
        self.assertNotEqual(echoes[0]["frameId"], echoes[1]["frameId"])
        self.assertIn("target/release/moli", listed.stderr)
        called = self.cli(
            "call",
            "echo",
            "--frame-id",
            "main",
            "--input",
            "-",
            input_text='{"text":"from stdin"}',
        )
        self.assertEqual(called.returncode, 0, called.stderr)
        self.assertEqual(
            json.loads(called.stdout)["output"], {"text": "from stdin", "count": 1}
        )
        failed = self.cli("call", "fail")
        self.assertEqual(failed.returncode, 1, failed.stderr)
        self.assertEqual(json.loads(failed.stdout)["status"], "Error")
        self.assertIn(
            "fixture failure", json.loads(failed.stdout)["exception"]["description"]
        )
        invalid = self.cli("call", "echo", "--input", '{"text":NaN}')
        self.assertEqual(invalid.returncode, 1, invalid.stderr)
        self.assertIn("Invalid tool input", invalid.stderr)
        self.assertNotIn("Moli:", invalid.stderr)

    def test_session_state_dynamic_tools_cancel_and_exact_form_confirmation(
        self,
    ) -> None:
        result = self.cli(
            "shell",
            input_text="""call echo {"text":"ambiguous"}
frame main
call echo {"text":"first"}
wait 1
call echo {"text":"second"}
wait 2
call add_extra {}
wait 3
schema extra
call extra {}
wait 4
call drop_extra {}
wait 5
tools
call waiting {}
cancel 6
call first_form {"message":"one"}
call second_form {"message":"two"}
confirm 7
wait 7
confirm 8
wait 8
quit
""",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("exists in multiple frames", result.stderr)
        values = records(result.stdout)
        responses = {
            value["invocationId"]: value
            for value in values
            if isinstance(value, dict)
            and value.get("status") in {"Completed", "Canceled"}
        }
        self.assertEqual(responses["1"]["output"], {"text": "first", "count": 1})
        self.assertEqual(responses["2"]["output"], {"text": "second", "count": 2})
        self.assertTrue(
            any(
                isinstance(value, dict) and value.get("name") == "extra"
                for value in values
            )
        )
        catalogs = [value for value in values if isinstance(value, list)]
        self.assertNotIn("extra", {tool["name"] for tool in catalogs[-1]})
        self.assertEqual(responses["6"]["status"], "Canceled")
        self.assertEqual(
            responses["7"]["output"], {"form": "first_form", "message": "one"}
        )
        self.assertEqual(
            responses["8"]["output"], {"form": "second_form", "message": "two"}
        )

    def test_existing_page_is_preserved_and_child_tool_is_callable(self) -> None:
        async def scenario(endpoint: str) -> None:
            version = discovery(endpoint)
            async with connect(
                version["webSocketDebuggerUrl"], proxy=None
            ) as websocket:
                page = CdpPage(websocket, 10)
                try:
                    await page.open(self.url)
                    await page.until(
                        lambda: (
                            len(
                                [
                                    tool
                                    for tool in page.all_tools()
                                    if tool["name"] == "echo"
                                ]
                            )
                            == 2
                        ),
                        "child tool",
                    )
                    child = next(
                        tool
                        for tool in page.all_tools()
                        if tool["frameId"] != page.frame_id
                    )
                    attached = await asyncio.to_thread(
                        self.cli,
                        "call",
                        "echo",
                        "--input",
                        '{"text":"child"}',
                        "--frame-id",
                        child["frameId"],
                        "--cdp-endpoint",
                        endpoint,
                        "--target-id",
                        page.target_id,
                        url="-",
                    )
                    self.assertEqual(attached.returncode, 0, attached.stderr)
                    self.assertEqual(
                        json.loads(attached.stdout)["output"],
                        {"text": "child", "child": True},
                    )
                    pending_form = await asyncio.to_thread(
                        self.cli,
                        "shell",
                        "--cdp-endpoint",
                        endpoint,
                        "--target-id",
                        page.target_id,
                        url="-",
                        input_text='call first_form {"message":"leave open"}\nquit\n',
                    )
                    self.assertEqual(pending_form.returncode, 0, pending_form.stderr)
                    self.assertFalse(
                        await page.evaluate(
                            "document.forms[0].matches(':tool-form-active')"
                        )
                    )
                    targets = await page.command("Target.getTargets", browser=True)
                    self.assertIn(
                        page.target_id,
                        {target["targetId"] for target in targets["targetInfos"]},
                    )
                    self.assertEqual(await page.evaluate("location.href"), self.url)
                finally:
                    await page.close()

        args = argparse.Namespace(
            cdp_endpoint=None,
            moli_bin=None,
            timeout=10,
            startup_timeout=15,
            http_proxy=None,
        )
        with moli_server(args) as endpoint:
            asyncio.run(scenario(endpoint))


if __name__ == "__main__":
    unittest.main()
