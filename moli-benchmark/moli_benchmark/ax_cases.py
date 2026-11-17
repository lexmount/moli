"""Independent expectations for the AX integration benchmark.

URLs use {origin}; backend/AX identifiers are deliberately not expectations.
Each case is a separate real document, including a script-completion marker.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class Probe:
    id: str
    expected: dict[str, Any]


@dataclass(frozen=True)
class Step:
    name: str
    probes: tuple[Probe, ...]
    script: str = ""


@dataclass(frozen=True)
class AxCase:
    name: str
    body: str
    steps: tuple[Step, ...]
    role_counts: dict[str, int] | None = None

    def html(self) -> str:
        return (f'<!doctype html><meta charset="utf-8"><title>{self.name}</title>'
                f'<body data-benchmark-case="{self.name}">{self.body}'
                '<script>const ready = document.createElement("i"); ready.hidden = true; '
                'ready.setAttribute("data-benchmark-status","ok"); '
                'document.body.appendChild(ready);</script></body>')


def p(id: str, role: str, name: str, **properties: Any) -> Probe:
    return Probe(id, {"ignored": False, "role": role, "name": name, **properties})


def case(name: str, body: str, *probes: Probe, role_counts: dict[str, int] | None = None,
         setup: str = "") -> AxCase:
    return AxCase(name, body, (Step("initial", probes, setup),), role_counts)


AX_CASES = (
    case("presentation-focus", '<a id="a" href="/go" role="presentation">Go</a>'
         '<a id="b" href="/back" role="none" tabindex="-1">Back</a>'
         '<button id="c" role="presentation">Submit</button>',
         p("a", "link", "Go", url="{origin}/go"), p("b", "link", "Back", url="{origin}/back"),
         p("c", "button", "Submit")),
    case("explicit-role-overrides", '<a id="a" role="button" href="/wrong">Action</a>'
         '<img id="b" role="link" alt="Help" src="/image.svg">'
         '<h2 id="c" role="none" aria-label="Heading">Visual</h2>',
         p("a", "button", "Action", url=None), p("b", "link", "Help", url=None),
         p("c", "heading", "Heading", level=2)),
    case("document-link-roles", ''.join(
        f'<a id="p{i}" role="{role}" href="#note{i}">Reference {i}</a>'
        for i, role in enumerate(("doc-noteref", "doc-backlink", "doc-biblioref", "doc-glossref"))),
        *(p(f"p{i}", role, f"Reference {i}", url=f"{{origin}}/document-link-roles#note{i}")
          for i, role in enumerate(("doc-noteref", "doc-backlink", "doc-biblioref", "doc-glossref")))),
    case("url-whitespace", '<base href="/assets/"><a id="a" href="&nbsp;">NBSP</a>'
         '<a id="b" href=" &#x2003; ">EM space</a><a id="c" href="">Empty</a>'
         '<a id="d" href=" &#x9;&#xA; ">HTML spaces</a>',
         p("a", "link", "NBSP", url="{origin}/assets/%C2%A0"),
         p("b", "link", "EM space", url="{origin}/assets/%E2%80%83"),
         p("c", "link", "Empty", url="{origin}/assets/"),
         p("d", "link", "HTML spaces", url="{origin}/assets/")),
    case("svg-href-namespaces", '<svg xmlns="http://www.w3.org/2000/svg" '
         'xmlns:xlink="http://www.w3.org/1999/xlink">'
         '<a id="a" xlink:href="/real" aria-label="Legacy"><text>Legacy</text></a>'
         '<a id="b" href="/primary" xlink:href="/real" aria-label="Primary"><text>Primary</text></a>'
         '<a id="c" href="" xlink:href="/real" aria-label="Empty"><text>Empty</text></a>'
         '<a id="d" role="link" aria-label="Foreign"><text>Foreign</text></a></svg>'
         '<script>for(const id of ["a","b","c","d"]) '
         'document.getElementById(id).setAttributeNS("urn:example","href","/wrong");</script>',
         p("a", "link", "Legacy", url="{origin}/real"),
         p("b", "link", "Primary", url="{origin}/primary"),
         p("c", "link", "Empty", url="{origin}/svg-href-namespaces"),
         p("d", "link", "Foreign", url=None),
         setup='if (!["a","b","c","d"].every(id => document.getElementById(id).'
               'getAttributeNS("urn:example","href") === "/wrong")) throw Error("namespace setup failed");'),
    case("mathml-link-capability", '<math><a id="a" role="link" href="/wrong" '
         'aria-label="Math reference">M</a></math><a id="b" href="/real">HTML reference</a>',
         p("a", "link", "Math reference", url=None),
         p("b", "link", "HTML reference", url="{origin}/real")),
    case("name-precedence", '<span id="first">First</span><span id="last">Last</span>'
         '<label for="a">Native label</label><input id="a" aria-label="ARIA label" '
         'aria-labelledby="last first" title="Tooltip">'
         '<label for="b">Native label</label><input id="b" aria-label="ARIA label">'
         '<label for="c">Native label</label><input id="c">',
         p("a", "textbox", "Last First", description="Tooltip"), p("b", "textbox", "ARIA label"),
         p("c", "textbox", "Native label")),
    case("hidden-label-references", '<span id="label" hidden>Hidden label</span>'
         '<span id="description" style="display:none">Hidden description</span>'
         '<button id="a" aria-labelledby="label" aria-describedby="description">Visual</button>'
         '<button id="b" title="Tooltip">Contents</button>',
         p("a", "button", "Hidden label", description="Hidden description"),
         p("b", "button", "Contents", description="Tooltip")),
    case("nested-name-content", '<button id="a">Save<span aria-label="document"></span>now</button>'
         '<button id="b">foo<span>bar</span>baz</button>'
         '<button id="c">A&nbsp;B<span hidden>Wrong</span></button>',
         p("a", "button", "Save document now"), p("b", "button", "foobarbaz"),
         p("c", "button", "A\u00a0B")),
    case("image-role-urls", '<img id="a" alt="Photo" src="/image.svg">'
         '<img id="b" role="button" alt="Picture action" src="/image.svg">'
         '<input id="c" type="image" alt="Submit" src="/image.svg">',
         p("a", "image", "Photo", url="{origin}/image.svg"),
         p("b", "button", "Picture action", url=None),
         p("c", "button", "Submit", url="{origin}/image.svg")),
    case("text-field-state", '<input id="a" aria-label="Name" value="Alice" required>'
         '<textarea id="b" aria-label="Notes" readonly>Line one</textarea>'
         '<input id="c" type="search" aria-label="Search" value="needle">',
         p("a", "textbox", "Name", value="Alice", required=True),
         p("b", "textbox", "Notes", value="Line one", readonly=True),
         p("c", "searchbox", "Search", value="needle")),
    case("choice-control-state", '<input id="a" type="checkbox" aria-label="Enabled" checked>'
         '<div id="b" role="checkbox" aria-label="Mixed" aria-checked="mixed" tabindex="0"></div>'
         '<input id="c" type="radio" aria-label="Choice" checked>',
         p("a", "checkbox", "Enabled", checked="true"),
         p("b", "checkbox", "Mixed", checked="mixed"), p("c", "radio", "Choice", checked="true")),
    case("range-control-state", '<input id="a" type="range" aria-label="Volume" min="10" max="90" value="42">'
         '<progress id="b" aria-label="Progress" value="3" max="10"></progress>'
         '<div id="c" role="slider" aria-label="Temperature" aria-valuemin="-10" '
         'aria-valuemax="50" aria-valuenow="20" aria-valuetext="Warm" tabindex="0"></div>',
         p("a", "slider", "Volume", value=42, valuemin=10, valuemax=90),
         p("b", "progressbar", "Progress", value=3, valuemin=0, valuemax=10),
         # Chromium's CDP valuetext exports AX kValue, rather than the separate
         # ARIA value-text attribute. Keep the observed protocol boundary here.
         p("c", "slider", "Temperature", value=20, valuemin=-10, valuemax=50, valuetext=""),
         setup='if (document.getElementById("c").getAttribute("aria-valuetext") !== "Warm") '
               'throw Error("range setup failed");'),
    case("disabled-fieldset", '<fieldset disabled><legend><button id="a" '
         'role="presentation">Legend action</button></legend><button id="b">Disabled action</button>'
         '</fieldset><button id="c" aria-disabled="true">ARIA disabled</button>',
         p("a", "button", "Legend action", disabled=False),
         p("b", "button", "Disabled action", disabled=True),
         p("c", "button", "ARIA disabled", disabled=True)),
    case("visibility-subtrees", '<button id="a">Visible</button>'
         '<div hidden><button id="b">Hidden</button></div>'
         '<div aria-hidden="true"><button id="c">ARIA hidden</button></div>'
         '<div inert><button id="d">Inert</button></div>'
         '<div style="visibility:hidden"><button id="e" style="visibility:visible">Restored</button></div>',
         p("a", "button", "Visible"), Probe("b", {"ignored": True}),
         Probe("c", {"ignored": True}), Probe("d", {"ignored": True}),
         p("e", "button", "Restored")),
    case("contents-and-containment", '<div style="display:contents"><button id="a">Contents</button></div>'
         '<span style="content-visibility:hidden"><button id="b">Inline content</button></span>'
         '<div style="content-visibility:hidden"><button id="c">Skipped</button></div>',
         p("a", "button", "Contents"), p("b", "button", "Inline content"),
         Probe("c", {"ignored": True})),
    case("shadow-slot-names", '<div id="a" role="button"><span slot="action">Slotted</span></div>'
         '<div id="b" role="button"><span slot="action">Closed slot</span></div>'
         '<script>for (const [id,mode] of [["a","open"],["b","closed"]]) '
         'document.getElementById(id).attachShadow({mode}).innerHTML = '
         '\'<span aria-label="Icon"></span><slot name="action"></slot>\';</script>',
         p("a", "button", "Icon Slotted"), p("b", "button", "Icon Closed slot")),
    AxCase("dynamic-role-and-name", '<a id="a" role="presentation" href="/old">Original</a>', (
        Step("initial", (p("a", "link", "Original", url="{origin}/old"),)),
        Step("button", (p("a", "button", "Updated", url=None),),
             'const a = document.getElementById("a"); a.setAttribute("role","button"); '
             'a.setAttribute("aria-label","Updated"); a.href="/new";'),
        Step("link", (p("a", "link", "Updated", url="{origin}/new"),),
             'document.getElementById("a").setAttribute("role","none");'),
    )),
    case("table-structure", '<table aria-label="Results"><thead><tr>'
         '<th id="a">Item</th><th>Count</th></tr></thead><tbody>' + ''.join(
             f'<tr><th scope="row">Item {i}</th><td>{i}</td></tr>' for i in range(300)) +
         '<tr><th id="b" scope="row">Last item</th><td id="c">300</td></tr></tbody></table>',
         p("a", "columnheader", "Item"), p("b", "rowheader", "Last item"), p("c", "cell", "300"),
         role_counts={"table": 1, "rowheader": 301, "columnheader": 2, "cell": 301}),
    case("navigation-1000", '<nav aria-label="Catalog">' + ''.join(
        f'<a id="item{i}" href="/item/{i}" role="presentation">Item {i}</a>' for i in range(1000)) + '</nav>',
        p("item0", "link", "Item 0", url="{origin}/item/0"),
        p("item999", "link", "Item 999", url="{origin}/item/999"), role_counts={"link": 1000}),
)
