from __future__ import annotations

import socket
import unittest
from contextlib import ExitStack, closing
from http.server import BaseHTTPRequestHandler

from moli_benchmark.wpt_cross.server import (
    _FixtureThreadingHTTPServer,
    _Ipv6ThreadingHTTPServer,
)


class FixtureConnectionBacklogTests(unittest.TestCase):
    def assert_worker_burst_can_connect(
        self,
        server_class: type[_FixtureThreadingHTTPServer],
        address: tuple[str, int],
    ) -> None:
        # Hold the accept loop so all connections coexist in the listen queue.
        # This isolates queue capacity from handler speed and TCP retry timing.
        with server_class(address, BaseHTTPRequestHandler) as server:
            host, port = server.server_address[:2]
            with ExitStack() as connections:
                for _ in range(50):
                    connections.enter_context(closing(socket.create_connection(
                        (host, port), timeout=1,
                    )))

    def test_loopback_listener_accepts_a_full_worker_burst(self) -> None:
        self.assert_worker_burst_can_connect(
            _FixtureThreadingHTTPServer, ("127.0.0.1", 0),
        )

    @unittest.skipUnless(socket.has_ipv6, "IPv6 is unavailable")
    def test_ipv6_listener_accepts_a_full_worker_burst(self) -> None:
        self.assert_worker_burst_can_connect(
            _Ipv6ThreadingHTTPServer, ("::1", 0),
        )
