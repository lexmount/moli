"""Ephemeral, explicitly trusted certificates for the local WPT listeners.

The fixture authority is private to one server instance. Clients receive its
public certificate; no system trust store or certificate verification setting
is changed. OpenSSL is also the certificate generator supported by wpt serve.
"""

from __future__ import annotations

import base64
import hashlib
import shutil
import ssl
import subprocess
import tempfile
import threading
import uuid
from pathlib import Path


class FixtureCertificateAuthority:
    def __init__(self, primary_hostname: str, external_host: str | None = None) -> None:
        executable = shutil.which("openssl")
        if executable is None:
            raise RuntimeError("WPT HTTPS fixtures require the openssl executable")
        self._openssl = executable
        self._directory = tempfile.TemporaryDirectory(prefix="moli-wpt-tls-")
        self.directory = Path(self._directory.name)
        self.ca_certificate = self.directory / "ca.pem"
        self._ca_key = self.directory / "ca.key"
        self._server_key = self.directory / "server.key"
        self._contexts: dict[str, ssl.SSLContext] = {}
        self._lock = threading.Lock()
        self._closed = False
        self._addresses = {"127.0.0.1", "::1"}
        if external_host is not None:
            self._addresses.add(external_host)
        try:
            self._run(
                "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                "-nodes", "-days", "2", "-subj", "/CN=Ephemeral local WPT CA",
                "-addext", "basicConstraints=critical,CA:TRUE",
                "-addext", "keyUsage=critical,keyCertSign,cRLSign",
                "-keyout", str(self._ca_key), "-out", str(self.ca_certificate),
            )
            self._run(
                "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
                "-out", str(self._server_key),
            )
            public_key = self._run("pkey", "-in", str(self._server_key), "-pubout", "-outform", "DER")
            self.server_spki_sha256 = base64.b64encode(hashlib.sha256(public_key).digest()).decode("ascii")
            self.context = self._context_for_hostname(primary_hostname)
            self.context.set_servername_callback(self._select_server_name)
        except BaseException:
            self.close()
            raise

    def _run(self, *arguments: str) -> bytes:
        result = subprocess.run(
            [self._openssl, *arguments], capture_output=True, check=False,
        )
        if result.returncode:
            raise RuntimeError(
                "WPT certificate generation failed: "
                + result.stderr.decode("utf-8", errors="replace")
            )
        return result.stdout

    def _context_for_hostname(self, hostname: str) -> ssl.SSLContext:
        with self._lock:
            if self._closed:
                return self.context
            if hostname in self._contexts:
                return self._contexts[hostname]
            stem = hashlib.sha256(hostname.encode("ascii")).hexdigest()
            request = self.directory / f"{stem}.csr"
            certificate = self.directory / f"{stem}.pem"
            extensions = self.directory / f"{stem}.ext"
            names = [f"DNS:{hostname}", "DNS:localhost"]
            names.extend(f"IP:{address}" for address in sorted(self._addresses))
            extensions.write_text(
                "basicConstraints=critical,CA:FALSE\n"
                "keyUsage=critical,digitalSignature\n"
                "extendedKeyUsage=serverAuth\n"
                + "subjectAltName=" + ",".join(names) + "\n",
                encoding="ascii",
            )
            self._run("req", "-new", "-key", str(self._server_key), "-subj", "/CN=Local WPT", "-out", str(request))
            self._run(
                "x509", "-req", "-in", str(request), "-CA", str(self.ca_certificate),
                "-CAkey", str(self._ca_key), "-set_serial", str(uuid.uuid4().int),
                "-days", "2", "-extfile", str(extensions), "-out", str(certificate),
            )
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = ssl.TLSVersion.TLSv1_2
            context.set_alpn_protocols(["http/1.1"])
            context.load_cert_chain(certificate, self._server_key)
            self._contexts[hostname] = context
            return context

    def _select_server_name(
        self, connection: ssl.SSLSocket, hostname: str | None, _context: ssl.SSLContext,
    ) -> None:
        if hostname is None:
            return
        hostname = hostname.encode("idna").decode("ascii").lower()
        # Issue only for the local fixture namespace. An unrelated name keeps
        # the default certificate and fails normal client hostname validation.
        if hostname != "localhost" and not hostname.endswith(".localhost"):
            return
        if any(character not in "abcdefghijklmnopqrstuvwxyz0123456789.-_" for character in hostname):
            return
        connection.context = self._context_for_hostname(hostname)

    def close(self) -> None:
        with self._lock:
            self._closed = True
            self._directory.cleanup()
