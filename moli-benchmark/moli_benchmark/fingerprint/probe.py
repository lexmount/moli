from pathlib import Path

ROOT = Path(__file__).parent


def dom_probe(fallback: str | None = None) -> str:
    source = (ROOT / "dom_results.js").read_text(encoding="utf-8")
    return source if fallback is None else f"arg => (\n{source}\n)(arg) ?? (\n{fallback}\n)(arg)"


def site_probe() -> str:
    return dom_probe((ROOT / "site_results.js").read_text(encoding="utf-8"))
