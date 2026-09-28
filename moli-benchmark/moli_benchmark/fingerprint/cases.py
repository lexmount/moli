from __future__ import annotations

from dataclasses import asdict, dataclass


@dataclass(frozen=True)
class Case:
    id: str
    url: str
    wait_seconds: int
    kind: str
    action: str | None = None
    action_wait_seconds: int = 0


# Keep the original twelve-site workload and fixed observation windows. Changing
# a URL, action, window or projector changes the workload hash in every report.
CASES = (
    Case("fingerprint-pro", "https://demo.fingerprint.com/playground", 12, "commercial-fingerprint"),
    Case("browserscan-bot", "https://www.browserscan.net/bot-detection", 10, "bot-signals"),
    Case("incolumitas-bot", "https://bot.incolumitas.com/", 16, "bot-and-behavior"),
    Case("incolumitas-proxy", "https://bot.incolumitas.com/proxy_detect.html", 15, "network-consistency"),
    Case("device-browser-static", "https://deviceandbrowserinfo.com/are_you_a_bot", 10, "bot-signals"),
    Case("device-browser-behavior", "https://deviceandbrowserinfo.com/are_you_a_bot_interactions", 20,
         "behavior", "demo_login", 20),
    Case("sannysoft", "https://bot.sannysoft.com/", 8, "legacy-headless"),
    Case("creepjs", "https://abrahamjuliot.github.io/creepjs/", 20, "fingerprint-consistency"),
    Case("pixelscan", "https://pixelscan.net/", 15, "fingerprint-consistency", "start_scan", 15),
    Case("browserleaks-js", "https://browserleaks.com/javascript", 8, "javascript-fingerprint"),
    Case("browserleaks-webgl", "https://browserleaks.com/webgl", 8, "graphics-fingerprint"),
    Case("browserscan-tls", "https://www.browserscan.net/tls", 10, "network-fingerprint"),
)


def select_cases(ids: list[str] | None = None) -> tuple[Case, ...]:
    known = {case.id for case in CASES}
    if ids is not None and (not ids or len(ids) != len(set(ids)) or set(ids) - known):
        raise ValueError("case selection must contain unique, known site IDs")
    return tuple(case for case in CASES if ids is None or case.id in ids)


def manifest(cases: tuple[Case, ...]) -> list[dict]:
    return [asdict(case) for case in cases]
