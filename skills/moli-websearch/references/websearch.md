# Keyword search

Choose a URL from [websearch-engines.md](websearch-engines.md). Replace `Q`
with the query encoded once using `encodeURIComponent(query)` or
`urllib.parse.quote(query, safe="")`.

## Fetch and Readiness

```bash
moli fetch --timeout 25000 --wait-until done --dump markdown "$SEARCH_URL"
```

Keep stderr and exit status separate from result text. An outer `timeout 30s`
can bound the process. For failed or empty output, try
`--wait-until domcontentloaded`; for page shells, inspect `--dump html`. Toutiao's
`snssdk143` shell can need the same readiness change. Persistent requests can
prevent `networkidle` from completing.

## Bing Extraction

Extract result elements with `textContent`; `innerText` depends on layout.
`--eval` and `--eval-file` are alternatives to `--dump`:

```bash
moli fetch --timeout 25000 --wait-until done --eval '
Array.from(document.querySelectorAll("li.b_algo")).map((row) => {
  const link = row.querySelector("h2 a[href]");
  return {
    title: link?.textContent.trim() ?? "",
    url: link?.href ?? "",
    snippet: row.querySelector(".b_caption p")?.textContent.trim() ?? "",
    text: row.textContent.trim()
  };
}).filter((row) => row.url)
' "$SEARCH_URL"
```

The expression can also be saved for `--eval-file`. If it returns an empty
array, compare with Markdown or HTML before concluding there are no results.

## Capture

For a requested Bing screenshot, this tested recipe allows time for rendering:

```bash
moli fetch --timeout 60000 --delay-ms 30000 --layout --resource \
  --dump screenshot_full "$SEARCH_URL" > bing-results.png
```

Adjust the delay to page readiness and allow an outer timeout longer than
60 seconds. `--wait-selector li.b_algo` alone did not make the recorded capture
ready. Verify a nonempty PNG; use extracted rows to assess search relevance.
