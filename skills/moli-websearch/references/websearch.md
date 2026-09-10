# Keyword search

Build a batch of independent query/engine jobs from
[websearch-engines.md](websearch-engines.md) using the
[parallel execution rules](../SKILL.md#parallel-execution). Replace `Q` with
the query encoded once using `encodeURIComponent(query)` or
`urllib.parse.quote(query, safe="")`. Split broad requests into subquestions;
search them concurrently across suitable engines.

## Fetch and Readiness

This is one job; dispatch the batch before awaiting individual results:

```bash
moli fetch --timeout 10000 --wait-until done --dump markdown "$SEARCH_URL"
```

Keep stdout, stderr, and exit status separate for each job. An outer
`timeout 15s` can bound the process, allowing startup and cleanup around the
10-second request timeout. Continue other jobs when one fails. For readiness
failures or empty output, try `--wait-until domcontentloaded`; for page shells,
inspect `--dump html`. On a CAPTCHA or access block, queue another engine with
Moli. Toutiao's `snssdk143` shell can need the same readiness change. Persistent
requests can prevent `networkidle` from completing.

## Bing Extraction

Extract result elements with `textContent`; `innerText` depends on layout.
`--eval` and `--eval-file` are alternatives to `--dump`:

```bash
moli fetch --timeout 10000 --wait-until done --eval '
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

For a requested Bing screenshot, this tested recipe uses a longer timeout
for rendering:

```bash
moli fetch --timeout 60000 --delay-ms 30000 --layout --resource \
  --dump screenshot_full "$SEARCH_URL" > bing-results.png
```

Adjust the delay to page readiness and allow an outer timeout longer than
60 seconds. `--wait-selector li.b_algo` alone did not make the recorded capture
ready. Verify a nonempty PNG; use extracted rows to assess search relevance.
