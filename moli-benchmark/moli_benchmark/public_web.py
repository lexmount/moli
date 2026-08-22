from __future__ import annotations

import html.parser
import re
from collections.abc import Callable, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path
from threading import BoundedSemaphore
from typing import Any, Generic, TypeVar

from .artifacts import write_json, write_text
from .config import REPO_ROOT
from .process import ProcessResult
from .stats import summarize
from .synthetic_compare import (
    target_enables_all_resource_fetch,
    target_is_cdp,
    target_metadata,
)


POST_DCL_SETTLE_MILLISECONDS = 50
PUBLIC_WEB_SNAPSHOT_CONTRACT = (
    "at least "
    f"{POST_DCL_SETTLE_MILLISECONDS} ms of page event-loop time after "
    "the adapter observes DOMContentLoaded"
)
POST_DCL_WAIT_SCRIPT = (
    "globalThis.__moliBenchmarkPostDclReady === true || "
    "(setTimeout(() => { globalThis.__moliBenchmarkPostDclReady = true; }, "
    f"{POST_DCL_SETTLE_MILLISECONDS}), false)"
)

_MAIN_DOCUMENT_STATUS_DIAGNOSTIC = re.compile(
    rb"(?:lifecycle\s+target\s+document|http\s+request)\s+"
    rb"`[^`\r\n]+`\s+returned\s+([45]\d{2})\b",
    re.IGNORECASE,
)
_LIGHTPANDA_FETCH_TIMEOUT_DIAGNOSTIC = re.compile(
    rb'\$msg="fetch error"[^\r\n]*\berr=timeout\b',
    re.IGNORECASE,
)
_SAFE_ARTIFACT_TOKEN = re.compile(r"[^A-Za-z0-9._-]+")
_TITLE_RE = re.compile(r"<title[^>]*>(.*?)</title>", re.IGNORECASE | re.DOTALL)
_TOP_SITES_BLOCKED_TITLE_403 = re.compile(r"^\s*(?:http\s*)?403\b")
_TOP_SITES_HTTP_ERROR_TITLE_STATUS = re.compile(
    r"^\s*(?:"
    r"error\s*:\s*([45]\d{2})\b"
    r"|(?:http\s*)?([45]\d{2})\s+(?:"
    r"bad\s+gateway|gateway\s+timeout|internal\s+server\s+error|"
    r"service\s+unavailable|forbidden|not\s+found|unauthorized|"
    r"too\s+many\s+requests|request\s+timeout|error"
    r")\b"
    r")",
    re.IGNORECASE,
)
_WILD_WEB_HTTP_ERROR_TITLE_STATUS = re.compile(
    r"^\s*(?:"
    r"error\s*:\s*([45]\d{2})\b"
    r"|(?:http\s*)?([45]\d{2})\s+(?:"
    r"bad\s+gateway|gateway\s+timeout|internal\s+server\s+error|"
    r"service\s+unavailable|forbidden|not\s+found|unauthorized|error"
    r")\b"
    r")",
    re.IGNORECASE,
)

_TOP_SITES_NETWORK_ERROR_MARKERS = (
    "privacy error",
    "your connection is not private",
    "net::err_cert",
    "this site can't be reached",
    "this site can’t be reached",
    "err_name_not_resolved",
    "err_connection",
    "err_timed_out",
    "dns_probe_finished",
    "could not resolve",
    "could not resolve host",
    "could not resolve hostname",
    "failed to resolve",
    "failed to lookup address information",
    "could not connect to server",
    "name resolution",
    "connection refused",
    "connection reset",
    "connection timed out",
    "no route to host",
    "network is unreachable",
    "ssl handshake",
    "ssl error",
    "ssl connect error",
    "tls handshake",
    "peerfailedverification",
    "peer failed verification",
    "timed out connecting",
    "timeout was reached",
    "request timeout",
    "i/o error",
    "broken pipe",
    "operation timed out",
    "failed to connect",
    "dns lookup",
    "curl request failed",
    "recv failure",
)
_WILD_WEB_NETWORK_ERROR_MARKERS = (
    "peerfailedverification",
    "peer failed verification",
    "privacy error",
    "your connection is not private",
    "this site can't be reached",
    "this site can’t be reached",
    "net::err_",
    "could not resolve",
    "failed to resolve",
    "failed to lookup address information",
    "connection refused",
    "curl request failed",
    "couldntresolvehost",
    "http2stream",
)
_NAVIGATION_NETWORK_FAILURE_MARKERS = (
    "couldntresolvehost",
    "http2stream",
    "reason: http2",
)
_CLI_DEADLINE_COMMAND_MARKERS = (
    "fetch_document_allow_http_error_with_wait_until",
    "fetch document allow-http-error",
    "fetch allow-http-error wait_until",
    "fetch wait_until",
)
_CLI_DEADLINE_TIMEOUT_MARKERS = ("timed out after",)
_CLI_FETCH_READINESS_TIMEOUT_MARKER = "fetch readiness timed out after"
_ELAPSED_FAILURE_TIMEOUT_GRACE_SECONDS = 1.0
_CAPTCHA_MARKERS = (
    "captcha",
    "安全验证",
    "验证你是真人",
    "人机验证",
    "human verification",
    "verification",
    "verify you are human",
    "verify that you're not a robot",
    "verify that you are not a robot",
    "are you a robot",
    "not a robot",
    "bot or not",
    "checking your browser",
    "perform security check",
    "security verification",
    "performing security verification",
    "请完成验证",
    "完成验证后",
    "当前环境异常",
    "select the 2 matching tiles",
    "powered and protected by privacy",
)
_LOGIN_MARKERS = ("sign in", "log in", "login", "登录", "注册", "账号")
_LOGIN_FORM_MARKERS = (
    "login to your account",
    "email/username",
    "your password is a required field",
    "forgot password",
    "create a free account",
)
_LOGIN_CONTEXT_MARKERS = (
    "password",
    "验证码",
    "短信验证码",
    "语音验证码",
    "手机号",
    "手机验证",
    "邮箱",
    "email",
)
_JS_CHALLENGE_MARKERS = (
    "__cf_chl_",
    "c2wf946j0/probe",
    "cf-challenge",
    "__tencent_chaos_vm",
    "__eo_jschallenge_vm",
    "teojschallengesdk.js",
    "eojschallengesdk",
    "window.solvechallenge(",
    "jsl_clearance",
    "acw_sc__v2",
    "window._phantom",
    "_$jsvmprt",
    "awswaf",
    "aliyun_waf",
    "aliyun waf",
    "_waf_",
    "just a moment...",
    "vercel security checkpoint",
    "we're verifying your browser",
    "we’re verifying your browser",
    "enable javascript and cookies to continue",
    "javascript is needed to access this site",
)
_BLOCKED_BODY_MARKERS = (
    "unable to give you access to our site",
    "access denied",
    "access to this page has been denied",
    "sorry, you have been blocked",
    "security issue was automatically identified",
    "access restricted",
    "exception: forbidden",
    "waf拦截",
    "被waf拦截",
    "security detection powered by safeline waf",
)
_NOT_FOUND_MARKERS = (
    "404 not found",
    "file not found",
    "page not found",
    "this page cannot be found",
    "page you requested is missing",
    "page you requested could not be found",
    "页面不存在",
    "视频不见了",
)
_FORBIDDEN_ERROR_MARKERS = ("403 forbidden", "returned 403", "http 403")
_NOT_FOUND_ERROR_MARKERS = ("returned 404", "http 404")


def reports_lightpanda_fetch_timeout(stderr: bytes) -> bool:
    """Return whether Lightpanda reported its terminal fetch-timeout record."""

    return _LIGHTPANDA_FETCH_TIMEOUT_DIAGNOSTIC.search(stderr) is not None


def reports_cli_fetch_timeout(stderr: bytes, returncode: int | None) -> bool:
    """Recognize stable typed and legacy Moli fetch-deadline diagnostics."""

    if returncode in (None, 0):
        return False
    diagnostic = stderr.decode("utf-8", errors="replace").lower()
    if _CLI_FETCH_READINESS_TIMEOUT_MARKER in diagnostic:
        return True
    return any(marker in diagnostic for marker in _CLI_DEADLINE_COMMAND_MARKERS) and any(
        marker in diagnostic for marker in _CLI_DEADLINE_TIMEOUT_MARKERS
    )


def resolve_main_document_status(
    protocol_status: int | None,
    stderr: bytes,
    returncode: int | None,
) -> tuple[int | None, str | None]:
    """Return the main-document status and the evidence that supplied it.

    CDP is the preferred source. Moli's fetch CLI also includes the terminal
    main-document status in its non-zero-exit diagnostic, so preserve that
    structured evidence instead of reducing it to a generic process error.
    The diagnostic matcher is deliberately narrow to avoid treating a logged
    subresource failure as the main navigation status.
    """

    if protocol_status is not None:
        return protocol_status, "protocol"
    if returncode in {None, 0}:
        return None, None
    matches = _MAIN_DOCUMENT_STATUS_DIAGNOSTIC.findall(stderr)
    if not matches:
        return None, None
    return int(matches[-1]), "cli-diagnostic"


def utc_timestamp() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def rotated_target_order(
    targets: tuple[str, ...],
    schedule_index: int,
) -> tuple[str, ...]:
    if not targets:
        return ()
    offset = schedule_index % len(targets)
    return (*targets[offset:], *targets[:offset])


@dataclass(frozen=True)
class PublicWebSnapshotProfile:
    hidden_tags: frozenset[str]
    preserve_title_entities: bool = False
    sample_chars: int = 500


TOP_SITES_SNAPSHOT_PROFILE = PublicWebSnapshotProfile(
    hidden_tags=frozenset(
        {"head", "script", "style", "noscript", "svg", "template"}
    ),
    preserve_title_entities=True,
)
WILD_WEB_SNAPSHOT_PROFILE = PublicWebSnapshotProfile(
    hidden_tags=frozenset({"script", "style", "noscript"}),
)


@dataclass(frozen=True)
class PublicWebArtifactPolicy:
    output_sample_bytes: int
    failure_stdout_limit_bytes: int
    failure_stderr_limit_bytes: int
    write_empty_failure_stdout: bool


TOP_SITES_ARTIFACT_POLICY = PublicWebArtifactPolicy(
    output_sample_bytes=512,
    failure_stdout_limit_bytes=32 * 1024,
    failure_stderr_limit_bytes=16 * 1024,
    write_empty_failure_stdout=False,
)
WILD_WEB_ARTIFACT_POLICY = PublicWebArtifactPolicy(
    output_sample_bytes=1024,
    failure_stdout_limit_bytes=128 * 1024,
    failure_stderr_limit_bytes=32 * 1024,
    write_empty_failure_stdout=True,
)


class _PublicWebTextExtractor(html.parser.HTMLParser):
    def __init__(self, hidden_tags: frozenset[str]) -> None:
        super().__init__(convert_charrefs=True)
        self._hidden_tags = hidden_tags
        self._hidden_depth = 0
        self._in_title = False
        self.title_parts: list[str] = []
        self.text_parts: list[str] = []

    def handle_starttag(
        self,
        tag: str,
        attrs: list[tuple[str, str | None]],
    ) -> None:
        del attrs
        normalized = tag.lower()
        if normalized in self._hidden_tags:
            self._hidden_depth += 1
        if normalized == "title":
            self._in_title = True

    def handle_endtag(self, tag: str) -> None:
        normalized = tag.lower()
        if normalized == "title":
            self._in_title = False
        if normalized in self._hidden_tags and self._hidden_depth:
            self._hidden_depth -= 1

    def handle_data(self, data: str) -> None:
        text = " ".join(data.split())
        if not text:
            return
        if self._in_title:
            self.title_parts.append(text)
        if self._hidden_depth == 0 and not self._in_title:
            self.text_parts.append(text)


def extract_public_web_snapshot(
    stdout: bytes,
    *,
    profile: PublicWebSnapshotProfile,
) -> dict[str, Any]:
    html_text = stdout.decode("utf-8", errors="replace")
    parser = _PublicWebTextExtractor(profile.hidden_tags)
    try:
        parser.feed(html_text)
    except html.parser.HTMLParseError:
        pass
    title = " ".join(parser.title_parts).strip()
    if profile.preserve_title_entities:
        match = _TITLE_RE.search(html_text)
        title = re.sub(r"\s+", " ", match.group(1)).strip() if match else ""
    visible_text = re.sub(r"\s+", " ", " ".join(parser.text_parts)).strip()
    return {
        "title": title,
        "text_length": len(visible_text),
        "text_sample": visible_text[: profile.sample_chars],
    }


def _looks_like_binary_content(body: bytes) -> bool:
    sample = body[:4096]
    if sample.startswith(b"%PDF-") or b"\x00" in sample:
        return True
    decoded = sample.decode("utf-8", errors="replace")
    return decoded.count("\ufffd") > max(8, len(decoded) // 20)


def _looks_like_navigation_network_failure(text: str) -> bool:
    return (
        "navigate failed" in text or "navigation failed" in text
    ) and any(marker in text for marker in _NAVIGATION_NETWORK_FAILURE_MARKERS)


def _looks_like_login_wall(
    title: str,
    sample: str,
    text_length: int,
) -> bool:
    page_text = f"{title}\n{sample}"
    if sum(marker in page_text for marker in _LOGIN_FORM_MARKERS) >= 2:
        return True
    if text_length >= 800:
        return False
    login_hits = {marker for marker in _LOGIN_MARKERS if marker in page_text}
    if not login_hits:
        return False
    title_has_login = any(marker in title for marker in _LOGIN_MARKERS)
    if title_has_login and len(login_hits) >= 2:
        return True
    return len(login_hits) >= 2 and any(
        marker in page_text for marker in _LOGIN_CONTEXT_MARKERS
    )


def _top_sites_http_error_category(status: int) -> str:
    if status in {401, 403}:
        return "blocked-or-forbidden"
    if status == 404:
        return "not-found"
    return "http-error"


def _wild_web_http_error_category(status: int) -> str:
    if status in {401, 403}:
        return "blocked"
    if status == 404:
        return "not-found"
    return "http-error"


@dataclass(frozen=True)
class PublicWebClassifier:
    """Classify normalized public-web evidence with a suite policy.

    Transport/status handling and evidence precedence live here once.  The
    two policies retain their established category vocabulary because those
    values are part of each suite's artifact schema.
    """

    policy: str
    snapshot_profile: PublicWebSnapshotProfile
    ok_categories: frozenset[str]
    non_comparable_categories: frozenset[str]
    failure_kind_aliases: Mapping[str, str]
    use_category_as_failure_kind: bool

    def classification_ok(self, category: str) -> bool:
        return category in self.ok_categories

    def comparable(self, category: str) -> bool:
        return category not in self.non_comparable_categories

    def failure_kind(
        self,
        category: str,
        *,
        error: str | None = None,
        extraction_failed: bool = False,
    ) -> str | None:
        if error == "target binary unavailable":
            return "target-unavailable"
        alias = self.failure_kind_aliases.get(category)
        if alias is not None:
            return alias
        if self.use_category_as_failure_kind and not self.classification_ok(
            category
        ):
            return category
        if extraction_failed:
            return "extraction-failure"
        return None

    def snapshot(self, stdout: bytes) -> dict[str, Any]:
        return extract_public_web_snapshot(
            stdout,
            profile=self.snapshot_profile,
        )

    def classify_output(
        self,
        *,
        stdout: bytes,
        stderr: bytes,
        returncode: int | None,
        timed_out: bool,
        min_body_bytes: int = 1,
        snapshot: Mapping[str, Any] | None = None,
        response_status: int | None = None,
        main_document_body_capture: str | None = None,
    ) -> str:
        if timed_out:
            return "timeout"
        response_status, _ = resolve_main_document_status(
            response_status,
            stderr,
            returncode,
        )
        if response_status is not None and response_status >= 400:
            return self._http_error_category(response_status)
        if (
            self.policy == "top-sites"
            and main_document_body_capture == "response-headers-only"
        ):
            return "binary-response-headers"
        if reports_lightpanda_fetch_timeout(stderr):
            return "timeout"
        if reports_cli_fetch_timeout(stderr, returncode):
            return "timeout"
        if self.policy == "top-sites":
            return self._classify_top_sites_document(
                stdout=stdout,
                stderr=stderr,
                returncode=returncode,
                min_body_bytes=min_body_bytes,
                snapshot=snapshot,
            )
        if self.policy == "wild-web":
            return self._classify_wild_web_document(
                stdout=stdout,
                stderr=stderr,
                returncode=returncode,
                snapshot=snapshot,
            )
        raise ValueError(f"unknown public-web classifier policy: {self.policy}")

    def classify_result(
        self,
        result: PublicWebResult,
        *,
        min_body_bytes: int = 1,
    ) -> str:
        process = result.process
        return self.classify_output(
            stdout=process.stdout,
            stderr=process.stderr,
            returncode=process.returncode,
            timed_out=process.timed_out,
            min_body_bytes=min_body_bytes,
            snapshot=result.snapshot,
            response_status=result.response_status,
            main_document_body_capture=process.main_document_body_capture,
        )

    def classification_basis(
        self,
        result: PublicWebResult,
        category: str,
    ) -> str:
        return self.classification_basis_for_process(
            result.process,
            category,
            response_status=result.response_status,
            response_status_source=result.response_status_source,
        )

    def classification_basis_for_process(
        self,
        process: ProcessResult,
        category: str,
        *,
        response_status: int | None,
        response_status_source: str | None,
    ) -> str:
        if process.timed_out or category == "timeout":
            return "timeout"
        if response_status is not None and response_status >= 400:
            source = response_status_source or "unknown"
            return f"{source}-http-status"
        if category == "binary-response-headers":
            return "protocol-binary-response-headers"
        if process.returncode not in {None, 0}:
            return "process-exit"
        if category == "success-binary-content":
            return "binary-signature"
        visible_categories = (
            {"success-content", "app-shell-only", "empty-response"}
            if self.policy == "top-sites"
            else {"success", "empty"}
        )
        if category in visible_categories:
            return "visible-content"
        return "document-marker"

    def _http_error_category(self, status: int) -> str:
        if self.policy == "top-sites":
            return _top_sites_http_error_category(status)
        if self.policy == "wild-web":
            return _wild_web_http_error_category(status)
        raise ValueError(f"unknown public-web classifier policy: {self.policy}")

    def _classify_top_sites_document(
        self,
        *,
        stdout: bytes,
        stderr: bytes,
        returncode: int | None,
        min_body_bytes: int,
        snapshot: Mapping[str, Any] | None,
    ) -> str:
        combined_text = (stdout + b"\n" + stderr).decode(
            "utf-8", errors="replace"
        ).lower()
        if "operationtimedout" in combined_text and (
            "navigate failed" in combined_text
            or "navigation failed" in combined_text
        ):
            return "timeout"
        if _looks_like_navigation_network_failure(combined_text):
            return "network-error"
        if returncode != 0:
            if any(
                marker in combined_text
                for marker in _FORBIDDEN_ERROR_MARKERS
            ):
                return "blocked-or-forbidden"
            if any(
                marker in combined_text for marker in _NOT_FOUND_ERROR_MARKERS
            ):
                return "not-found"
            if any(
                marker in combined_text
                for marker in _TOP_SITES_NETWORK_ERROR_MARKERS
            ):
                return "network-error"
            return "process-error"

        body = stdout.strip()
        if not body:
            if any(
                marker in combined_text
                for marker in _TOP_SITES_NETWORK_ERROR_MARKERS
            ):
                return "network-error"
            return "empty-response"
        if len(body) >= min_body_bytes and _looks_like_binary_content(body):
            return "success-binary-content"

        page_snapshot = snapshot or self.snapshot(stdout)
        text_length = int(page_snapshot.get("text_length") or 0)
        title = str(page_snapshot.get("title") or "").lower()
        sample = str(page_snapshot.get("text_sample") or "").lower()
        page_text = f"{title}\n{sample}"
        if any(
            marker in page_text for marker in _TOP_SITES_NETWORK_ERROR_MARKERS
        ):
            return "network-error"
        inferred_error = self._inferred_top_sites_http_error(title, sample)
        if inferred_error is not None:
            return inferred_error
        if (
            "blocked" in title
            or "forbidden" in title
            or _TOP_SITES_BLOCKED_TITLE_403.search(title)
            or "access restricted" in title
        ):
            return "blocked-or-forbidden"
        if any(marker in page_text for marker in _NOT_FOUND_MARKERS):
            return "not-found"
        if any(marker in page_text for marker in _BLOCKED_BODY_MARKERS):
            return "blocked-or-forbidden"
        if any(marker in page_text for marker in _CAPTCHA_MARKERS):
            return "captcha-or-verification"
        if any(marker in combined_text for marker in _JS_CHALLENGE_MARKERS):
            return "js-challenge"
        if _looks_like_login_wall(title, sample, text_length):
            return "login-wall"
        if len(body) < min_body_bytes or text_length < min_body_bytes:
            return "app-shell-only"
        return "success-content"

    def _classify_wild_web_document(
        self,
        *,
        stdout: bytes,
        stderr: bytes,
        returncode: int | None,
        snapshot: Mapping[str, Any] | None,
    ) -> str:
        diagnostic_text = stderr.decode("utf-8", errors="replace").lower()
        if returncode != 0:
            if any(
                marker in diagnostic_text
                for marker in _WILD_WEB_NETWORK_ERROR_MARKERS
            ):
                return "network-error"
            return "error"

        page_snapshot = snapshot or self.snapshot(stdout)
        visible_text = " ".join(
            str(page_snapshot.get(key) or "")
            for key in ("title", "text_sample")
        ).lower()
        text = f"{visible_text}\n{diagnostic_text}"
        if any(marker in text for marker in _WILD_WEB_NETWORK_ERROR_MARKERS):
            return "network-error"
        title = str(page_snapshot.get("title") or "")
        status_match = _WILD_WEB_HTTP_ERROR_TITLE_STATUS.match(title)
        if status_match is not None:
            status = int(status_match.group(1) or status_match.group(2))
            return _wild_web_http_error_category(status)
        sample = str(page_snapshot.get("text_sample") or "")
        if sample.lstrip().startswith(('{"error":{', '{"errors":[')):
            return "http-error"
        if "页面不存在" in text or "视频不见了" in text:
            return "not-found"
        if "security detection powered by safeline waf" in text:
            return "blocked"
        if (
            "captcha" in text
            or "verify" in text
            or "challenge" in text
            or "安全验证" in text
            or "完成验证后" in text
            or "当前环境异常" in text
            or "select the 2 matching tiles" in text
            or "powered and protected by privacy" in text
        ):
            return "challenge"
        if "login" in text or "登录" in text:
            return "login"
        if (
            "blocked" in text
            or "forbidden" in text
            or "403" in text
            or "access restricted" in text
        ):
            return "blocked"
        return "success" if stdout.strip() else "empty"

    @staticmethod
    def _inferred_top_sites_http_error(
        title: str,
        sample: str,
    ) -> str | None:
        match = _TOP_SITES_HTTP_ERROR_TITLE_STATUS.match(title)
        if match is not None:
            status_text = match.group(1) or match.group(2)
            if status_text is not None:
                return _top_sites_http_error_category(int(status_text))
        if sample.lstrip().startswith(('{"error":{', '{"errors":[')):
            return "http-error"
        return None


TOP_SITES_CLASSIFIER = PublicWebClassifier(
    policy="top-sites",
    snapshot_profile=TOP_SITES_SNAPSHOT_PROFILE,
    ok_categories=frozenset({"success-content", "success-binary-content"}),
    non_comparable_categories=frozenset({"binary-response-headers"}),
    failure_kind_aliases={},
    use_category_as_failure_kind=True,
)
WILD_WEB_CLASSIFIER = PublicWebClassifier(
    policy="wild-web",
    snapshot_profile=WILD_WEB_SNAPSHOT_PROFILE,
    ok_categories=frozenset({"success", "login", "challenge"}),
    non_comparable_categories=frozenset(),
    failure_kind_aliases={
        "timeout": "timeout",
        "blocked": "blocked",
        "challenge": "challenge",
        "login": "login",
        "empty": "empty-response",
        "error": "process-error",
        "network-error": "network-error",
        "not-found": "not-found",
        "http-error": "http-error",
    },
    use_category_as_failure_kind=False,
)


def elapsed_failure_reached_timeout(
    elapsed_ms: float | None,
    timeout_seconds: float,
) -> bool:
    if elapsed_ms is None or timeout_seconds <= 0:
        return False
    grace_seconds = min(
        _ELAPSED_FAILURE_TIMEOUT_GRACE_SECONDS,
        timeout_seconds * 0.05,
    )
    timeout_floor_ms = max(
        0.0,
        (timeout_seconds - grace_seconds) * 1000.0,
    )
    return elapsed_ms >= timeout_floor_ms


@dataclass(frozen=True)
class PublicWebAttempt:
    """One target's execution of one scheduled public-web case."""

    target: str
    metadata: Mapping[str, str]
    target_info: Mapping[str, Any]
    run: int
    case_fields: Mapping[str, Any]
    url: str
    schedule_index: int
    target_order_index: int
    artifact_stem: str
    started_at: str

    @classmethod
    def start(
        cls,
        *,
        target: str,
        metadata: Mapping[str, str],
        target_info: Mapping[str, Any],
        run: int,
        case_fields: Mapping[str, Any],
        url: str,
        schedule_index: int,
        target_order_index: int,
        artifact_stem: str,
    ) -> PublicWebAttempt:
        return cls(
            target=target,
            metadata=dict(metadata),
            target_info=dict(target_info),
            run=run,
            case_fields=dict(case_fields),
            url=url,
            schedule_index=schedule_index,
            target_order_index=target_order_index,
            artifact_stem=artifact_stem,
            started_at=utc_timestamp(),
        )

    def binary_path(self) -> Path | None:
        path = self.target_info.get("path")
        if not self.target_info.get("available") or not path:
            return None
        return Path(str(path))

    def base_row(self, *, finished_at: str | None = None) -> dict[str, Any]:
        return {
            "target": self.target,
            **dict(self.metadata),
            "run": self.run,
            **dict(self.case_fields),
            "url": self.url,
            "schedule_index": self.schedule_index,
            "target_order_index": self.target_order_index,
            "started_at": self.started_at,
            "finished_at": finished_at or utc_timestamp(),
        }


@dataclass(frozen=True)
class PublicWebResult:
    """Normalized process and response evidence for a public-web attempt."""

    attempt: PublicWebAttempt
    process: ProcessResult
    response_status: int | None
    response_status_source: str | None
    snapshot: dict[str, Any]
    finished_at: str

    @classmethod
    def capture(
        cls,
        attempt: PublicWebAttempt,
        process: ProcessResult,
        *,
        classifier: PublicWebClassifier,
    ) -> PublicWebResult:
        response_status, response_status_source = resolve_main_document_status(
            process.response_status,
            process.stderr,
            process.returncode,
        )
        return cls(
            attempt=attempt,
            process=process,
            response_status=response_status,
            response_status_source=response_status_source,
            snapshot=classifier.snapshot(process.stdout),
            finished_at=utc_timestamp(),
        )

    def base_row(self) -> dict[str, Any]:
        return self.attempt.base_row(finished_at=self.finished_at)

    def evidence_fields(
        self,
        *,
        policy: PublicWebArtifactPolicy,
    ) -> dict[str, Any]:
        process = self.process
        sample_bytes = policy.output_sample_bytes
        return {
            "final_url": process.final_url,
            "elapsed_ms": process.elapsed_ms,
            "returncode": process.returncode,
            "timed_out": process.timed_out,
            "response_status": self.response_status,
            "response_status_source": self.response_status_source,
            "response_mime_type": process.response_mime_type,
            "main_document_body_capture": process.main_document_body_capture,
            "stdout_bytes": len(process.stdout),
            "stderr_bytes": len(process.stderr),
            "stdout_sha256": sha256(process.stdout).hexdigest(),
            "stderr_sha256": sha256(process.stderr).hexdigest(),
            "title": self.snapshot["title"],
            "text_length": self.snapshot["text_length"],
            "text_sample": self.snapshot["text_sample"],
            "stdout_head": process.stdout[:sample_bytes].decode(
                "utf-8", errors="replace"
            ),
            "stdout_tail": process.stdout[-sample_bytes:].decode(
                "utf-8", errors="replace"
            ),
            "stderr_tail": process.stderr[-sample_bytes:].decode(
                "utf-8", errors="replace"
            ),
            "peak_pss_bytes": process.resources.get("peak_pss_bytes"),
            "peak_rss_bytes": process.resources.get("peak_rss_bytes"),
        }


def unavailable_public_web_row(
    attempt: PublicWebAttempt,
    *,
    category: str,
    extra_fields: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        **attempt.base_row(),
        "category": category,
        "ok": False,
        "failure_kind": "target-unavailable",
        "error": "target binary unavailable",
        **dict(extra_fields or {}),
    }


def build_public_web_fetch_command(
    target: str,
    binary: Path,
    url: str,
    timeout_seconds: float,
    *,
    suite_name: str,
    omit_moli_page_wait: bool = False,
) -> list[str]:
    if target_is_cdp(target):
        raise RuntimeError(f"{target} is a CDP target; use the cdp-session suite")
    if target == "chrome":
        raise RuntimeError(f"chrome {suite_name} uses the CDP DCL runner")
    timeout_ms = str(int(timeout_seconds * 1000))
    if target in {"moli", "moli-full"}:
        compatibility_args = (
            ["--layout", "--resource"]
            if target_enables_all_resource_fetch(target)
            else []
        )
        page_wait_args = (
            []
            if omit_moli_page_wait
            else [
                "--wait-until",
                "domcontentloaded",
                "--wait-script",
                POST_DCL_WAIT_SCRIPT,
            ]
        )
        return [
            str(binary),
            "fetch",
            *compatibility_args,
            "--dump",
            "html",
            *page_wait_args,
            "--timeout",
            timeout_ms,
            "--http-timeout",
            timeout_ms,
            url,
        ]
    if target == "lightpanda":
        return [
            str(binary),
            "fetch",
            "--dump",
            "html",
            "--wait-until",
            "domcontentloaded",
            "--wait-script",
            POST_DCL_WAIT_SCRIPT,
            "--wait-ms",
            timeout_ms,
            "--http-timeout",
            timeout_ms,
            "--terminate-ms",
            timeout_ms,
            url,
        ]
    if target == "obscura":
        return [
            str(binary),
            "fetch",
            "--dump",
            "html",
            "--wait-until",
            "load",
            "--wait",
            "0",
            "--timeout",
            str(max(1, int(timeout_seconds))),
            url,
        ]
    raise RuntimeError(f"unknown target: {target}")


def public_web_target_metadata(target: str) -> dict[str, str]:
    metadata = dict(target_metadata(target))
    if target == "chrome" or target_is_cdp(target):
        metadata["driver"] = "cdp-dcl"
        prefix = "moli full" if target == "moli-full-cdp" else metadata["engine"]
        metadata["label"] = f"{prefix} / cdp-dcl"
    return metadata


def run_public_web_target(
    *,
    target: str,
    binary: Path,
    url: str,
    timeout_seconds: float,
    proc_env: dict[str, str],
    command_builder: Callable[[str, Path, str, float], list[str]],
    chrome_runner: Callable[..., ProcessResult],
    served_cdp_runner: Callable[..., ProcessResult],
    process_runner: Callable[..., ProcessResult],
) -> ProcessResult:
    if target == "chrome":
        return chrome_runner(
            binary,
            url,
            cwd=REPO_ROOT,
            timeout_seconds=timeout_seconds,
            env=proc_env,
        )
    if target_is_cdp(target):
        return served_cdp_runner(
            target,
            binary,
            url,
            cwd=REPO_ROOT,
            timeout_seconds=timeout_seconds,
            env=proc_env,
        )
    return process_runner(
        command_builder(target, binary, url, timeout_seconds),
        cwd=REPO_ROOT,
        timeout_seconds=timeout_seconds + 2,
        env=proc_env,
    )


CaseT = TypeVar("CaseT")
ExecutionT = TypeVar("ExecutionT")


@dataclass(frozen=True)
class PublicWebScheduledCase(Generic[CaseT]):
    schedule_index: int
    case_index: int
    run: int
    case: CaseT

    @property
    def target_rotation_index(self) -> int:
        return self.case_index + self.run - 1


def schedule_public_web_cases(
    cases: Sequence[CaseT],
    *,
    runs: int,
) -> list[PublicWebScheduledCase[CaseT]]:
    return [
        PublicWebScheduledCase(
            schedule_index=schedule_index,
            case_index=case_index,
            run=run,
            case=case,
        )
        for schedule_index, (run, case_index, case) in enumerate(
            (run, case_index, case)
            for run in range(1, runs + 1)
            for case_index, case in enumerate(cases)
        )
    ]


class PublicWebScheduler(Generic[CaseT, ExecutionT]):
    """Run target attempts sequentially per case and cases concurrently."""

    def __init__(
        self,
        targets: tuple[str, ...],
        *,
        parallelism: int = 1,
        target_parallelism: Mapping[str, int] | None = None,
    ) -> None:
        if parallelism <= 0:
            raise ValueError("public-web parallelism must be positive")
        limits = dict(target_parallelism or {})
        if any(limit <= 0 for limit in limits.values()):
            raise ValueError("public-web target parallelism must be positive")
        self._targets = targets
        self._parallelism = parallelism
        self._target_slots = {
            target: BoundedSemaphore(limit) for target, limit in limits.items()
        }

    def run(
        self,
        scheduled_cases: Sequence[PublicWebScheduledCase[CaseT]],
        execute: Callable[[PublicWebScheduledCase[CaseT], str, int], ExecutionT],
    ) -> list[ExecutionT]:
        def execute_group(
            scheduled: PublicWebScheduledCase[CaseT],
        ) -> list[ExecutionT]:
            results: list[ExecutionT] = []
            target_order = rotated_target_order(
                self._targets,
                scheduled.target_rotation_index,
            )
            for target_order_index, target in enumerate(target_order, start=1):
                slots = self._target_slots.get(target)
                if slots is None:
                    results.append(execute(scheduled, target, target_order_index))
                    continue
                with slots:
                    results.append(execute(scheduled, target, target_order_index))
            return results

        if self._parallelism == 1:
            return [
                result
                for scheduled in scheduled_cases
                for result in execute_group(scheduled)
            ]

        results_by_index: dict[int, list[ExecutionT]] = {}
        with ThreadPoolExecutor(max_workers=self._parallelism) as executor:
            future_to_case = {
                executor.submit(execute_group, scheduled): scheduled
                for scheduled in scheduled_cases
            }
            for future in as_completed(future_to_case):
                scheduled = future_to_case[future]
                results_by_index[scheduled.schedule_index] = future.result()
        return [
            result
            for scheduled in scheduled_cases
            for result in results_by_index[scheduled.schedule_index]
        ]


def safe_artifact_filename_token(value: str, *, max_length: int = 80) -> str:
    token = _SAFE_ARTIFACT_TOKEN.sub("_", value)
    while ".." in token:
        token = token.replace("..", "_")
    token = token.strip("._-") or "site"
    if token == value and len(token) <= max_length:
        return token
    digest = sha256(value.encode("utf-8", errors="surrogatepass")).hexdigest()[:12]
    token = token[:max_length].rstrip("._-") or "site"
    return f"{token}-{digest}"


def write_public_web_failure_artifacts(
    *,
    suite_dir: Path,
    attempt: PublicWebAttempt,
    row: Mapping[str, Any],
    stdout: bytes,
    stderr: bytes,
    policy: PublicWebArtifactPolicy,
) -> str:
    failures_dir = suite_dir / "failures"
    json_path = failures_dir / f"{attempt.artifact_stem}.json"
    stdout_path = failures_dir / f"{attempt.artifact_stem}.stdout.html"
    stderr_path = failures_dir / f"{attempt.artifact_stem}.stderr.txt"
    write_json(json_path, dict(row))
    write_text(
        stderr_path,
        stderr[-policy.failure_stderr_limit_bytes :].decode(
            "utf-8", errors="replace"
        ),
    )
    if policy.write_empty_failure_stdout or stdout.strip():
        write_text(
            stdout_path,
            stdout[-policy.failure_stdout_limit_bytes :].decode(
                "utf-8", errors="replace"
            ),
        )
    return str(json_path.relative_to(suite_dir))


def write_public_web_replay_artifact(
    *,
    suite_dir: Path,
    attempt: PublicWebAttempt,
    stdout: bytes,
) -> str:
    replay_path = suite_dir / "replay" / f"{attempt.artifact_stem}.html"
    write_text(replay_path, stdout.decode("utf-8", errors="replace"))
    return str(replay_path.relative_to(suite_dir))


def successful_public_web_attempt_cohort(
    rows: list[dict[str, Any]],
    targets: tuple[str, ...],
    *,
    attempt_key_fields: tuple[str, ...],
    unique_key_fields: tuple[str, ...],
    unique_count_name: str,
    metric_fields: tuple[str, ...],
    require_comparable: bool = False,
) -> dict[str, Any]:
    by_attempt: dict[tuple[Any, ...], dict[str, dict[str, Any]]] = {}
    for row in rows:
        key = tuple(row[field] for field in attempt_key_fields)
        by_attempt.setdefault(key, {})[str(row["target"])] = row
    selected_keys = [
        key
        for key, target_rows in by_attempt.items()
        if all(
            target in target_rows
            and (
                not require_comparable
                or target_rows[target].get("comparable", True)
            )
            and target_rows[target].get("ok")
            for target in targets
        )
    ]
    unique_indexes = tuple(
        attempt_key_fields.index(field) for field in unique_key_fields
    )
    cohort: dict[str, Any] = {
        "attempts": len(selected_keys),
        unique_count_name: len(
            {
                tuple(key[index] for index in unique_indexes)
                for key in selected_keys
            }
        ),
        "targets": {},
    }
    for target in targets:
        target_rows = [by_attempt[key][target] for key in selected_keys]
        cohort["targets"][target] = {
            field: summarize(
                row[field]
                for row in target_rows
                if row.get(field) is not None
            )
            for field in metric_fields
        }
    return cohort


def count_public_web_row_values(
    rows: Sequence[Mapping[str, Any]],
) -> dict[str, dict[str, int]]:
    counts: dict[str, dict[str, int]] = {
        "categories": {},
        "failure_kinds": {},
        "classification_bases": {},
        "response_status_sources": {},
    }
    field_names = {
        "category": "categories",
        "failure_kind": "failure_kinds",
        "classification_basis": "classification_bases",
        "response_status_source": "response_status_sources",
    }
    for row in rows:
        for field, count_name in field_names.items():
            value = row.get(field)
            if field == "classification_basis" and not value:
                value = "unknown"
            if not isinstance(value, str) or not value:
                continue
            field_counts = counts[count_name]
            field_counts[value] = field_counts.get(value, 0) + 1
    return counts
