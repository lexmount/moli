# Reverse image search

Find subject labels, source pages, or similar images from a picture. Keyword
image results such as `tbm=isch` do not establish a reverse-image match.

## Prepare the Image

- Preserve the relevant subject when cropping surrounding UI or enlarging a
  tiny image for an upload widget.
- URL methods need public HTTP(S) image bytes with an `image/*` content type.
  Check redirects with `curl -fsSI -L "$IMAGE_URL"`; download if HEAD is
  inconclusive. Engine servers cannot fetch localhost or private-only URLs.
- Upload methods accept local bytes. Diagnostic fixtures and method coverage
  are in [imagesearch-engines.md](imagesearch-engines.md); use the user's image
  for the actual search.

## Public URL

Launch compatible URL methods from the engine table concurrently under the
[parallel execution rules](../SKILL.md#parallel-execution). Encode the image URL
once as the `ENC` query value; each command is one job:

```bash
moli fetch --timeout 10000 --wait-until done --dump markdown "$SEARCH_URL"
```

An outer `timeout 15s` allows startup and cleanup around the 10-second request
timeout. If only an empty image and “No matching results” appear, check image
retrieval before treating it as an index miss; Wikimedia hotlinks failed in
the recorded tests.

## Upload and API

Use the relevant [CDP/API recipe](cdp-driver.md) for file inputs, multipart,
raw bytes, or base64 requests. That guide includes server startup and cleanup;
[moli-cdp-server](../../moli-cdp-server/SKILL.md) covers other CDP clients.

Run independent engine flows in parallel. In a shared CDP context, create
targets serially as the driver requires, then run each flow in its own target.
Keep navigation, upload, and result retrieval ordered within each flow.

Assign a file and wait for its automatic upload. An extra search click can
cancel Baidu's request; wait for hydrated result cards as shown in the recipe.
Follow the resulting URL or inspect API data to determine whether matches exist.

Yandex's Sites section supplies source pages; its regional endpoints share an
index, so agreement is not independent corroboration. IQDB's weak Possible
match scores do not identify a real-world photograph. Marketing demos and
homepage topic mosaics are not reverse-image results.
