# Reverse image CDP and API recipes

Read the sections needed for the selected method.
[imagesearch-engines.md](imagesearch-engines.md) contains coverage and
observations; [imagesearch.md](imagesearch.md) covers image preparation and
interpretation.

## Contents

- [Start the Server](#start-the-server)
- [Driver Template](#driver-template)
- [Browser Uploads](#browser-uploads)
- [General Web APIs](#general-web-apis)
- [Stock and Editorial Images](#stock-and-editorial-images)
- [Shopping and Fashion](#shopping-and-fashion)
- [Art and Specialist Indexes](#art-and-specialist-indexes)
- [Trademarks and Designs](#trademarks-and-designs)
- [Other Upload and API Methods](#other-upload-and-api-methods)
- [Teardown](#teardown)

## Start the Server

```bash
MOLI_SEARCH_PORT=19890          # choose an unused port for this run
moli serve --host 127.0.0.1 --port "$MOLI_SEARCH_PORT" --layout --timeout 180 \
  > moli-websearch-server.log 2>&1 &
MOLI_SEARCH_PID=$!
curl -fsS \
  --retry 10 --retry-connrefused --retry-delay 1 --max-time 2 \
  "http://127.0.0.1:$MOLI_SEARCH_PORT/json/version"
```

Wait for discovery before connecting. `serve --timeout` uses seconds;
`fetch --timeout` uses milliseconds. Layout supplies upload-widget geometry.
Hidden file inputs can accept `setInputFiles`; camera controls may need a DOM
`element.click()` when locator clicks fail. Broad `page.route` interception
stalled the recorded client.

## Driver Template

Save as `reverse-image.cjs` where Node resolves `playwright-core`, with `curl`
on `PATH`. This example submits a local image to Sogou:

```bash
MOLI_CDP_URL="http://127.0.0.1:$MOLI_SEARCH_PORT" \
  node reverse-image.cjs /absolute/path/to/image.jpg
```

```js
const { chromium } = require("playwright-core");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs"), path = require("node:path");

const CDP = (process.env.MOLI_CDP_URL || "http://127.0.0.1:19890").replace(/\/$/, "");
const EVAL_MS = 8000, GOTO_MS = 35000;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function bounded(promise, ms, label) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(label)), ms);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function putNew() {
  const raw = execFileSync("curl", ["-fsS", "-X", "PUT", CDP + "/json/new"],
    { encoding: "utf8", timeout: 15000 });
  const info = JSON.parse(raw);
  if (!info.id) throw new Error("target creation returned no id");
  return info;
}
function jsonClose(id) {
  try {
    execFileSync("curl", ["-fsS", CDP + "/json/close/" + encodeURIComponent(id)], { timeout: 8000 });
  } catch (error) {
    process.exitCode = 1;
    console.error("target cleanup failed:", error.message);
  }
}

async function newPage(ctx) {
  const before = new Set(ctx.pages());
  const info = putNew();
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    const page = ctx.pages().find((p) => !before.has(p));
    if (page) return { page, id: info.id };
    await sleep(100);
  }
  jsonClose(info.id);
  throw new Error("new target did not attach");
}

async function evalPage(page, fn, arg, label = "eval") {
  try {
    const p = arg === undefined ? page.evaluate(fn) : page.evaluate(fn, arg);
    return await bounded(p, EVAL_MS, "eval-timeout:" + label);
  } catch (e) { return { _evalError: String(e.message || e) }; }
}

// Diagnostic text and links for upload pages; inspect result/API fields too.
const DUMP = () => {
  const body = (document.body?.textContent || "").replace(/\s+\n/g, "\n").trim();
  return {
    href: location.href, title: document.title,
    body: body.slice(0, 2500), bodyLen: body.length,
    links: [...document.querySelectorAll("a[href]")].slice(0, 80)
      .map((a) => ({ t: (a.textContent || "").replace(/\s+/g, " ").trim().slice(0, 80), h: a.href })),
    nFile: document.querySelectorAll("input[type=file]").length,
  };
};

// wait for the URL to LEAVE the landing page (auto-POST navigation)
async function waitLeave(page, stayRe, ms) {
  const t0 = Date.now(), trail = [];
  while (Date.now() - t0 < ms) {
    let u = ""; try { u = page.url(); } catch {}
    trail.push({ dt: Date.now() - t0, u });
    if (u && !stayRe.test(u)) return { moved: true, url: u, trail };
    await sleep(400);
  }
  return { moved: false, url: page.url(), trail: trail.slice(-8) };
}

// fallback when setInputFiles does not trigger the site's listener
async function setDT(page, filePath) {
  const b64 = fs.readFileSync(filePath).toString("base64");
  const name = path.basename(filePath);
  const mime = {
    ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png",
    ".webp": "image/webp", ".gif": "image/gif", ".avif": "image/avif",
    ".bmp": "image/bmp", ".tif": "image/tiff", ".tiff": "image/tiff",
  }[path.extname(name).toLowerCase()];
  if (!mime) throw new Error("unsupported image extension; convert to PNG or JPEG first");
  return evalPage(page, ({ b64, name, mime }) => {
    const bin = atob(b64), bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const file = new File([bytes], name, { type: mime });
    const out = [];
    for (const el of document.querySelectorAll("input[type=file]")) {
      const dt = new DataTransfer(); dt.items.add(file);
      el.files = dt.files;
      el.dispatchEvent(new Event("input",  { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      out.push({ name: el.name, n: el.files.length });
    }
    return out;
  }, { b64, name, mime }, "dt");
}

(async () => {
  if (!process.argv[2]) throw new Error("usage: node reverse-image.cjs <image-path>");
  const SEED = path.resolve(process.argv[2]);
  fs.accessSync(SEED, fs.constants.R_OK);
  const browser = await chromium.connectOverCDP(CDP);
  let id;
  try {
    const ctx = browser.contexts()[0];
    if (!ctx) throw new Error("CDP endpoint returned no default context");
    const target = await newPage(ctx);
    const page = target.page;
    id = target.id;
    page.on("request", (request) => {
      if (request.method() === "POST") console.error("upload request observed");
    });

    await bounded(
      page.goto("https://pic.sogou.com/ris", { waitUntil: "domcontentloaded", timeout: GOTO_MS }),
      GOTO_MS + 2000, "navigation-timeout"
    );
    await sleep(1500);

    const fileLoc = page.locator("input[type=file]");
    if (!(await fileLoc.count())) throw new Error("no file input; inspect the camera widget");
    await fileLoc.first().setInputFiles(SEED, { timeout: 8000 });

    // These widgets upload on change; wait without clicking a search button.
    const nav = await waitLeave(page, /pic\.sogou\.com\/ris\/?$/, 15000);
    const dump = await evalPage(page, DUMP);
    console.log(JSON.stringify({ nav, dump }, null, 2));
    if (!nav.moved || dump?._evalError) process.exitCode = 1;
  } finally {
    if (id) jsonClose(id);
    await bounded(browser.close(), 8000, "disconnect-timeout");
  }
})().catch((error) => { console.error("driver failed:", error); process.exit(1); });
```

Create targets serially so `context.pages()` comparison is unambiguous.
`_evalError` is a failure; a timeout stops waiting but does not cancel the
evaluation. Close stalled targets and restart only an owned server if needed.
The truncated dump is diagnostic; inspect result Markdown or API data for matches.

The remaining blocks are fragments. Read bytes with `fs.readFileSync` in Node;
pass base64 or byte arrays through `evalPage`, then construct page-side `Blob`
or `FormData`. Variables `bytes`, `blob`, `publicImageUrl`, and tokens refer to
the chosen image/session. Run cookie-dependent requests on the engine's origin,
check HTTP status, and return API fields to Node before navigation or CLI fetches.

## Browser Uploads

**Sogou RIS:** the driver starts at `https://pic.sogou.com/ris` and waits for
`ris.sogou.com/ris?flag=1&from=pic_result_list&query=…`.

**360** — land on `https://st.so.com/`, assign `#stUpload input[name=upload]`
(or the page's only file input), stay-regex `/^https:\/\/st\.so\.com\/?$/`,
expect `/r?src=st`.

**360 public URL:** `GET https://st.so.com/r?src=st&img_url=${encodeURIComponent(url)}`
or from `st.so.com` origin `FormData.append("img_url", url)` → `POST /r?src=st`.

**Baidu:** start at `https://graph.baidu.com/pcpage/index`, assign
`input.general-upload-file`, and wait to leave `/pcpage\/index/`. The camera
`.graph-d20-search-wrapper-camera` is optional. Clicking
`.graph-d20-search-btn` 「识图一下」 can cancel the automatic upload.
After navigation to `graph.baidu.com/s?...`, wait for hydrated cards:

```js
await page.waitForFunction(() => {
  const cd = window.cardData;
  if (Array.isArray(cd)) {
    return cd.some((card) => ["product", "simipic", "same"].includes(card?.cardName));
  }
  return !!(cd && (cd.product || cd.simipic || cd.same));
}, undefined, { timeout: 15000 });
```

Cards can be an array of `{cardName, tplData}` or an object keyed by card name.
An early `noresult` / 「功能优化中，敬请期待」 is a placeholder;
`errpage?pron=pcupload` reported an index miss in the recorded flow.

**SauceNAO:** open `https://saucenao.com/`, then submit a URL or file:

```js
await page.fill("#urlInput", SEED_URL);
await evalPage(page, () => { document.querySelector("form").submit(); });
// or file: await page.locator("#fileInput").setInputFiles(SEED);
// or XHR:
// const fd = new FormData(); fd.append("url", SEED_URL); fd.append("db", "999");
// await fetch("/search.php", { method: "POST", body: fd, credentials: "include" });
```

Expect `search.php` with title “Sauce Found?” and percentage rows.

**Bing native widget:** open the camera with a DOM click, then assign the file:

```js
await evalPage(page, () => document.querySelector("#sb_sbi").click());
await page.locator("#sb_fileinput").setInputFiles(SEED);
```

### Bing Visual Search

```js
const fd = new FormData();
fd.append("image", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const r = await fetch(
  "/images/kblob?iss=sbiupload&FORM=SBIIRP&sbisrc=ImgPicker",
  { method: "POST", body: fd, credentials: "include" }
);
const { redirectUrl } = await r.json(); // e.g. /search?q=Apollo%2011%20Moon%20Landing&bcid=…
// then goto https://www.bing.com + redirectUrl, or moli fetch --delay-ms 5000
```

### Yandex Local Files

Visit `https://yandex.com/images/` or `https://yandex.ru/images/`, then submit
a raw image Blob:

```js
const r = await fetch(
  "/images-apphost/image-download?cbird=111&images_avatars_size=preview&images_avatars_namespace=images-cbir",
  { method: "POST", body: new Blob([bytes], { type: "image/jpeg" }), credentials: "include" }
);
// multipart FormData with file/image/upfile → 400
const j = await r.json(); // cbir_id, url, …
```

Encode `j.url` (the preview under `avatars.mds.yandex.net/get-images-cbir/`)
and `j.cbir_id` into
`https://yandex.com/images/search?rpt=imageview&url=ENC&cbir_id=CBIR_ID`.
Fetch that URL as Markdown; the recorded CDP SPA had little result content.

### Reuse Results and Structured Fields

- **Baidu:** from upload result URL keep `sign=HEX`; later `goto https://graph.baidu.com/s?sign=HEX&tpl_from=pc&f=all`.
- **Sogou:** from result `query=` param (sogoucdn hash URL); reopen `https://ris.sogou.com/ris?query=` + that CDN URL.
- **Bing IK:** warm `https://www.bing.com/images`, scrape `skey` from page/scripts, then
  `GET /images/api/custom/knowledge?nbl=1&form=VSSERP&skey=…&imgurl=…&ajaxreq=1`.

## General Web APIs

### CopyChecker

```js
const b64 = Buffer.from(jpegBytes).toString("base64"); // NOT data:image/... — that 500s
const res = await fetch("https://copychecker.com/api/ai-reverse-image-search", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    Origin: "https://copychecker.com",
    Referer: "https://copychecker.com/ai-reverse-image-search",
  },
  body: JSON.stringify({ image: b64, deviceId: optionalDeviceId }),
});
// URL path: POST /api/ai-reverse-image-search-image-fatcher {imageUrl} → jpeg bytes → then same search with base64
// Rate: /api/ris-check-device-limit {deviceId} ; may 429
```

### CopySeeker

```js
// Prefer URL recipe (public image URL). Capture RSC/flight JSON from network — freemium UI may blank after SCANNING.
const land = "https://copyseeker.net/search?imageurl=" + encodeURIComponent(publicImageUrl);
await page.goto(land, { waitUntil: "domcontentloaded" });
// → redirects to /discovery#<discoveryId>
// Parse flight/RSC payloads for: bestGuessLabel, entities[], totalLinksFound, pages[]
// RapidAPI surface is keygated; use site recipe above.
```

### Pic Detective

```js
const fd = new FormData();
fd.append("file", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg"); // NOT "image"
const up = await fetch("https://picdetective.com/api/upload", { method: "POST", body: fd });
const { imageUrl } = await up.json();
// or: await fetch(..., { method:"POST", headers:{"Content-Type":"application/json"}, body: JSON.stringify({url: publicUrl}) })
const sr = await fetch(
  "https://picdetective.com/api/search?url=" + encodeURIComponent(imageUrl) + "&search_type=similar",
  { headers: { Accept: "application/json", "X-Provider-Preference": "auto" } }
);
// → visual_matches (prefer similar; exact may 500; may requiresCaptcha)
```

### RepostSleuth

```js
// URL (target_match_percent REQUIRED)
const u =
  "https://api.repostsleuth.com/api/image?url=" +
  encodeURIComponent(publicImageUrl) +
  "&target_match_percent=50";
const jr = await fetch(u).then((r) => r.json());
// → matches[] with subreddit titles (NASA → Buzz Aldrin / Apollo)

// Local file
const fd = new FormData();
fd.append("image", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const pr = await fetch("https://api.repostsleuth.com/api/image", { method: "POST", body: fd });
```

SPA UI: `https://repostsleuth.com/search` (login walls on some pages — prefer API).

### same.energy

```js
// bytes: a Uint8Array containing the selected JPEG image.
const up = await fetch(
  "https://imageapi.same.energy/upload?length=" + bytes.byteLength,
  { method: "PUT", headers: { "Content-Type": "image/jpeg" }, body: bytes }
);
const j = await up.json(); // { kind:"success", payload:{ id, sha1, width, height } }
const id = j.payload.id;
const nd = await fetch(
  "https://imageapi.same.energy/search?i=" + encodeURIComponent(id) + "&n=40",
  { headers: { Origin: "https://same.energy", Referer: "https://same.energy/" } }
).then((r) => r.text());
// NDJSON: progress then success with payload.images[]
// SPA often empty innerText / login chrome — prefer imageapi (raw PUT, no anon host)
// SPA may resize maxSize=512 before upload
```

### Decopy.ai

```js
// Product-Serial from localStorage web-client-device; Product-Code 067003
// Prefer curl/UI — page fetch may CORS-block custom Product-Code header
const fd = new FormData();
fd.append("image", blob, "seed.jpg");
await fetch("https://api.decopy.ai/api/decopy/file-manager/upload-image", {
  method: "POST",
  headers: { "Product-Code": "067003", "Product-Serial": deviceSerial },
  body: fd,
});
// POST /api/decopy/reverse-image/search-by-image { image_url | image } → job_id
// GET /api/decopy/reverse-image/get-job/{id} → CDN thumbs
// Unlogin quota 210301; open-link login-gated
```

## Stock and Editorial Images

### Depositphotos

```js
const land =
  "https://depositphotos.com/search/by-images.html?url=" +
  encodeURIComponent(publicImageUrl);
// Parse result alts/captions for stock CBIR evidence

// Local file (UI SBI path)
const fd = new FormData();
fd.append("dp_files", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
await fetch("https://uploader.depositphotos.com/upload/saveFiles", { method: "POST", body: fd });
```

### Getty and iStock

```js
// Same family on gettyimages.com and istockphoto.com (cookies from land).
const origin = "https://www.gettyimages.com"; // or istockphoto.com
const meta = await fetch(
  origin + "/components/search-bar/api/upload-visual-search/upload-data?assettype=image&family=creative",
  { credentials: "include" }
).then((r) => r.json());
// → { uploadUrl, searchUrl }
await fetch(meta.uploadUrl, {
  method: "PUT",
  headers: { "Content-Type": "image/jpeg" },
  body: jpegBytes,
});
await page.goto(origin + meta.searchUrl); // /search/search-by-image?imageurl=<b64>&assettype=image
```

UI: `[data-testid=search-by-image-button]` → dropzone (canvas resize ≤1000px) then same API.

### 123RF

```js
const fd = new FormData();
fd.append("image", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg"); // NOT "file"
const up = await fetch("https://www.123rf.com/apicore/search/reverse/upload", {
  method: "POST",
  body: fd,
  credentials: "include",
});
const { data } = await up.json(); // data.fid = base64 of 123rf-sdl-ris-prod S3 URL
const jr = await fetch(
  "https://www.123rf.com/apicore/search/reverse-new?fid=" + encodeURIComponent(data.fid),
  { credentials: "include" }
).then((r) => r.json());
// HTML alt: /reverse-search/?fid=
// Plain /apicore/search/reverse?fid= often 500 — use reverse-new
```

### Bridgeman Images

```js
const fd = new FormData();
fd.append("file", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const up = await fetch("https://www.bridgemanimages.com/image-search/upload", {
  method: "POST",
  body: fd,
  credentials: "include",
});
const { imgHash } = await up.json(); // "upl:…"
await page.goto(
  "https://www.bridgemanimages.com/en-US/search/reverse-image?image-hash=" +
    encodeURIComponent(imgHash)
);
// Do NOT use ?url= — that is keyword FP, not CBIR
```

### SuperStock

```js
const fd = new FormData();
fd.append("file", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const uploadId = await fetch("https://www.superstock.com/api/FileUpload/uploadFile", {
  method: "POST",
  body: fd,
  credentials: "include",
}).then((r) => r.json()); // JSON string UUID
// optional: GET /api/Search/isUploadValidForSearch?uploadId=
await page.goto(
  "https://www.superstock.com/search?uploadId=" +
    encodeURIComponent(uploadId) +
    "&page=1&page-size=50&sort=Relevance"
);
// SPA also POSTs /api/Search with uploadId — HTML SERP is enough for captions
```

### Westend61

```js
// Prefer live page (Anubis blocks bare curl)
const fd = new FormData();
fd.append("file", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const res = await fetch("https://www.westend61.de/imageSearch/reverseImageSearch", {
  method: "POST",
  body: fd,
  credentials: "include",
});
// Parse HTML/JSON for reverse-search result cards / H1 count
```

### Mauritius Rocketloop

`POST https://mauritius.rocketloop.ai/image_search` with `image_file` + `similar_items=100` (or `image_url`). Resolve IDs via `api.mauritius.sodatech.com/assets/{id}`. May miss non-stock seeds.

### IMAGO Creative

```js
// Warm Creative SPA first; take Authorization: Basic from SearchPageLayout axios (frontend credential — not a user login).
const fd = new FormData();
fd.append("image", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg"); // ≤10MB; jpeg/png/heic/webp/bmp/tiff
const res = await fetch("https://api.imago-images.com/ewb/search/reverse-image", {
  method: "POST",
  headers: { Authorization: basicFromSpa },
  body: fd,
});
// → [{took,total},{pictures:[{pictureid,caption,owner,mediathumb,...}]}]
// UI: data-testid=search-by-image / reverse-image-search (Creative only)
// Bare www HTML may hit Bunny Shield — prefer API host after SPA warm
```

### Ximilar Stock Photo Search Demo

```js
const body = {
  service: "ID-Photo-Similarity",
  skip_recaptcha: 1,
  records: [{ _url: publicImageUrl }], // or { _base64: "data:image/jpeg;base64,..." }
};
const res = await fetch("https://api.ximilar.com/demo/call_service_rest", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    Origin: "https://demo.ximilar.com",
    Referer: "https://demo.ximilar.com/stock-photo-search",
  },
  body: JSON.stringify(body),
});
// → answer_count, answer_records[{_id,_url}] on cdn.stockmediaserver.com
```

### Unsplash Visual Search

Run the upload on unsplash.com with same-origin cookies. Build a result URL
only after a successful response with a UUID:

```js
// after goto https://unsplash.com/
// bytes: a Uint8Array containing the selected JPEG image.
const fd = new FormData();
fd.append('file', new Blob([bytes], {type:'image/jpeg'}), 'seed.jpg');
const up = await fetch('https://unsplash.com/napi/search/by_image', {
  method: 'POST', body: fd, credentials: 'include'
});
if (!up.ok) throw new Error('Unsplash upload failed: HTTP ' + up.status);
// 201 → {"uuid":"..."}
const {uuid} = await up.json();
if (!uuid) throw new Error('Unsplash upload returned no UUID');
```

Then outside CDP:

```bash
moli fetch --dump markdown --wait-until done \
  "https://unsplash.com/s/visual/$UUID"
```

### PIXTA

```js
// Land www.pixtastock.com; CSRF from cookie / meta
// 1) POST /search/image/presigned FormData extension=jpeg|png + X-CSRF-Token + X-Requested-With
//    → { data: { key, url, policy, x-amz-*, … } }
// 2) POST data.url (proximus-production S3) FormData: all data fields except url + file
// 3) POST /search/retrieve_vsi FormData key= + CSRF → { result: <code> }
// 4) GET /search/image/vsi.json?code=<code>&page=2 → numFound + items[]
```

### Stocksy

```js
// Canvas-resize ≤512px JPEG; strip data:image/jpeg;base64, prefix → raw b64
const fd = new FormData();
fd.append("base64Img", rawB64);
fd.append("filename", "seed.jpg");
const { id: ref } = await fetch("https://www.stocksy.com/home/visualsearchimgendpoint", {
  method: "POST",
  body: fd,
  credentials: "include", // ZEND-XSRF-TOKEN
}).then((r) => r.json());
const feed = await fetch(
  "https://www.stocksy.com/api1/imagefeeds?page=1&pageSize=100&type=visualSearch&collectionType=visual-search&reference=" +
    encodeURIComponent(ref),
  { credentials: "include" }
);
// Also: GET /home/visualsearchresults?reference=
// Do NOT use /search/visual?url= — keyword FP
```

## Shopping and Fashion

### Lykdat

UI file assign alone is insufficient. From a page that holds the key (or after visiting lykdat.com):

```js
const fd = new FormData();
fd.append("image", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
// or: fd.append("image_url", publicUrl);
const r = await fetch(
  "https://api.lykdat.com/search?api_key=" + apiKey + "&version=custom",
  { method: "POST", body: fd }
);
// → { public_url, results[].item_name, similar_products[] }
```

Fashion-only index; general photos may mis-tag as clothing.

### searchbyimage.com

Multipart `#image_upload` alone does not POST. Use:

```js
const b64 = Buffer.from(bytes).toString("base64"); // in Node; or btoa in page
const dataUrl = "data:image/jpeg;base64," + b64;
const body = new URLSearchParams();
body.set("search_query", encodeURIComponent(dataUrl)); // site expects double-encoding
// also send `acab` meta fields as sniffed from a successful URL search
await fetch("/search.php", { method: "POST", body, credentials: "include",
  headers: { "Content-Type": "application/x-www-form-urlencoded" } });
```

### Amazon StyleSnap

```js
await page.goto("https://www.amazon.com/shopthelook");
const token = await page.$eval('input[name=stylesnap]', (el) => el.value);
const fd = new FormData();
fd.append("explore-looks.jpg", new Blob([bytes], { type: "image/jpeg" }), "explore-looks.jpg");
const res = await fetch(
  "https://www.amazon.com/stylesnap/upload?stylesnapToken=" + encodeURIComponent(token),
  { method: "POST", body: fd, credentials: "include" }
);
// → queryId, wall-object boxes, ASINs (shopping CBIR)
```

## Art and Specialist Indexes

### trace.moe

```js
const fd = new FormData();
fd.append("image", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const r = await fetch("https://api.trace.moe/search?anilistInfo=1", { method: "POST", body: fd });
// → { result: [ { anilist, similarity, from, to, … } ] }
```

### Flim

```js
const up = await fetch(
  "https://r63www2ltd.execute-api.eu-central-1.amazonaws.com/default/uploadSimilarImage",
  {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ url: publicImageUrl, image: null }),
    // or { image: "data:image/jpeg;base64,…", url: null }
  }
);
const { id } = await up.json();
const sr = await fetch("https://api.flim.ai/2.0.0/search", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    Origin: "https://app.flim.ai",
    Referer: "https://app.flim.ai/",
  },
  body: JSON.stringify({
    page: 0,
    number_per_pages: 40,
    search: {
      full_text: "",
      similar_picture_id: id,
      movie_id: "",
      collection_id: "",
      board_id: "",
      saved_images: false,
      filters: { safety_content: [] },
      negative_filters: { safety_content: [] },
    },
  }),
});
// UI image-search paywalled — APIs work anonymously
```

### StillsLab

```js
// Anon text CLIP + similar (true reverse upload needs sign-in)
const frames = await fetch("https://stillslab.com/data/frames?search=" + encodeURIComponent("astronaut NASA")).then(r=>r.json());
const id = frames[0].id; // e.g. 5410 Interstellar
const sim = await fetch("https://stillslab.com/data/frame/" + id + "/similar").then(r=>r.json());
// POST /api/visual-search FormData image+vec[512] → 401 without auth
```

### imgs.ai

```js
const fd = new FormData();
fd.append("upload", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
fd.append("emb_type", "clip_vit"); // or vgg19|raw|poses
fd.append("n", "50");
fd.append("metric", "manhattan");
fd.append("model", "Smithsonian"); // Smithsonian_Local|Metropolitan|Rijksmuseum|Getty|ImageNet|MoMA
const res = await fetch("https://imgs.ai/interface", {
  method: "POST",
  body: fd,
  credentials: "include", // keep cookie jar after GET land
});
// HTML grid .item#<id> museum hrefs
```

### Museum Semantic Search

```js
const body = {
  image: "data:image/jpeg;base64," + b64,
  limit: 10,
};
const res = await fetch(
  "https://museum-semantic-search.vercel.app/api/mcp/image-search",
  {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }
);
// also POST /api/image-search ; GET /api/mcp/similar/met_<id>
```

### EarthExplorer

```js
// 1) POST /gradio_api/upload?upload_id=… multipart → /tmp/gradio/<hash>/nasa.jpg
// 2) POST /gradio_api/call/search_image
//    data: [FileData, top_permille(10), "DINOv2"|"SigLIP"|"FarSLIP"|"SatCLIP"]
// 3) GET /gradio_api/call/search_image/<event_id> SSE until complete
```

### Icons8

```js
const fd = new FormData();
fd.append("file", new Blob([bytes], { type: "image/jpeg" }), "seed.jpg");
const res = await fetch(
  "https://search-app.icons8.com/api/iconsets/vector/search/file?page=1&limit=100",
  { method: "POST", body: fd }
);
// UI: icons8.com Search-by-image → /icons/search-by-image
```

### FuzzySearch

```js
// Public client x-api-key: scrape from fuzzysearch.net frontend (do not hardcode stale keys if rotated)
const fd = new FormData();
fd.append("image", blob, "seed.jpg");
const res = await fetch("https://api.fuzzysearch.net/v1/image?distance=3", {
  method: "POST",
  headers: { "x-api-key": publicClientKey },
  body: fd,
});
// Also: GET /v1/url?url= ; client WASM hash → GET /v1/hashes?hash=
// NASA often []; furry-art index
```

### Portrait Matcher

```js
const fd = new FormData();
fd.append("im", blob, "face.jpg");
fd.append("agree", "1");
const res = await fetch("https://zeus.robots.ox.ac.uk/portraitmatcher/getFace", {
  method: "POST",
  body: fd,
});
// or ?url= public image; → top-3 ArtUK painting matches
```

### WhatFontIs

Land whatfontis.com → multipart `userfile` → crop → optimize → character labels → `step5-results` font list.

## Trademarks and Designs

### EUIPO eSearch

`POST /copla/imagesearch/uploadAndSegments?searchType=basic` (multipart image) → then `ctm/search` + `rcd/search` with returned segments.

### INPIT GrIP

Land `https://www.graphic-image.inpit.go.jp/` → `#ImageFile` → `#photo_image` → submit 結果を表示 → `/search/`.
Supplied service notice: planned end 2026-12-28, 17:00 JST; verify on use.

### IP Australia

Land: `https://search.ipaustralia.gov.au/trademarks/search/advanced`

```js
// Dropzone input.dz-hidden-input
const up = await fetch(
  "https://search.ipaustralia.gov.au/trademarks/search/internal/image",
  { method: "POST", headers: { "X-XSRF-TOKEN": xsrf }, body: fdFile, credentials: "include" }
);
// → { imageId, segments:[{segmentType,bbox}] }
// Then POST /trademarks/search/doSearch → 302 /trademarks/search/result?s=
// Trust count + cdn2.search.ipaustralia.gov.au TRADE_MARK thumbs (SPA hydrates slow)
```

### IPONZ

Land: `https://app.iponz.govt.nz/app/TradeMarkCheck`

```js
// POST …/PtoService/Common/PublicSearch/Handler/UploadImageHandler.ashx?p=<session>
// → files[0].openUrl / imagePath under root\PublicQuestionPictures\…
await fetch("…/PublicSearchHandler.ashx?ty=2", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ imagePath: "root\\PublicQuestionPictures\\…", markStatus: false }),
});
// → tradeMarks[] with figurativeHitType:"Image" (DOM may show login chrome — prefer JSON)
```

### TMDN DesignView and TMview

Origin: `https://www.tmdn.org/tmdsview-web/`

```js
const fd = new FormData();
fd.append("file", blob, "seed.jpg");
fd.append("clienttype", "desktop");

// DesignView
await fetch("https://www.tmdn.org/tmdsview-web/api/imageSearch/ds/uploadAndSegments", {
  method: "POST", body: fd, credentials: "include",
});
await fetch("https://www.tmdn.org/tmdsview-web/api/search/dsv/results?translate=true", {
  method: "POST", credentials: "include", /* body from SPA criteria */
});

// TMview (UI #/tmview may Rejected — use same origin)
await fetch("https://www.tmdn.org/tmdsview-web/api/imageSearch/tm/uploadAndSegments", {
  method: "POST",
  headers: { "X-XSRF-Token": xsrf },
  body: fd,
  credentials: "include",
});
await fetch("https://www.tmdn.org/tmdsview-web/api/search/results?translate=true", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    page: 1, pageSize: 20, criteria: "C",
    imageId, imageName, segmentLeft: 0, segmentRight, segmentTop: 0, segmentBottom,
    colour: false, ocr: false, imageSearch: true,
  }),
  credentials: "include",
});
```

Distinct from EUIPO eSearch copla.

### IMPI MARCia

Land: `https://marcia.impi.gob.mx/marcas/search/quick`

```js
// CSRF: meta[name=_csrf_header] + meta[name=_csrf]
const fd = new FormData();
fd.append("files", blob, "seed.jpg"); // MUST be files
const imgs = await fetch(
  "https://marcia.impi.gob.mx/marcas/search/internal/image/upload/bulk",
  { method: "POST", headers: csrfHeaders, body: fd, credentials: "include" }
).then((r) => r.json());
// → [{id, width, height, autoSegmentCrop}]
const rec = await fetch("https://marcia.impi.gob.mx/marcas/search/internal/record", {
  method: "POST",
  headers: { "Content-Type": "application/json", ...csrfHeaders },
  body: JSON.stringify({
    _type: "Search$Quick",
    query: "",
    images: [{ id: imgs[0].id, segment: imgs[0].autoSegmentCrop }],
  }),
  credentials: "include",
}).then((r) => r.json());
const page = await fetch("https://marcia.impi.gob.mx/marcas/search/internal/result", {
  method: "POST",
  headers: { "Content-Type": "application/json", ...csrfHeaders },
  body: JSON.stringify({
    searchId: rec.id,
    pageSize: 20,
    pageNumber: 0,
    sort: "IMAGE",
    statusFilter: [],
    viennaCodeFilter: [],
    niceClassFilter: [],
  }),
  credentials: "include",
}).then((r) => r.json());
// thumbs: https://marcia.impi.gob.mx/marcas/image/RM…
```

## Other Upload and API Methods

- **Lens App:** `POST /api/reverse.php?endpoint=lens|openai`, FormData `image`, `lang`, `country`.
- **Lenso.ai:** `POST /api/upload`, JSON raw base64 + `facial_search_consent: 0` + encrypted `El(tkn)` → upload id → `POST`/`GET` `/api/search`.
- **Picture-Alliance:** resize ≤1280, JPEG q≈0.5 → `POST https://simsearch-pa.sodatech.com/v1/fileupload/similarity`, multipart `file` + Bearer → `temporaryFilename` → `https://search.picture-alliance.com/{id}`.
- **IPOS Similar Mark:** `digitalhub.ipos.gov.sg/.../MN_TmSimilarMarkSearch` → `#inputImage` → `CM_Ajax/ConvertImage` → `#saveImgBtn` → `CM_Ajax/SearchSimilarMark`.
- **SearchThisImage:** `/v1/search/url` or `/v1/search/upload`, FormData `file` + `X-API-Key`; WEB_DETECTION response.
- **Google Lens:** `POST searchbyimage/upload`, multipart `encoded_image` → Lens `vsrid` URL.
- **SmilingWolf danbooru2022 HF:** Gradio upload → predict with Danbooru API key.
- **NumLookup:** `POST …/upload_photo`, field `file1`; search requires grecaptcha.
- **FaceCheck.ID:** `POST /api/upload_pic`, `images` + `id_search=NEW` → `/api/search`; recorded NEED_CAPTCHA, curl requires APIKEY.
- **Picarta.ai:** `/classify` requires Enterprise TOKEN; `/gei` reads EXIF only.
- **GeoInfer:** geolocation predict with Turnstile/key; no web-page CBIR.
- **Open-i NLM:** upload route `home`; see engine observations for the incomplete flow.

## Teardown

The driver closes its target and client in `finally`. Stop an owned server
from the shell that started it:

```bash
kill "$MOLI_SEARCH_PID"
wait "$MOLI_SEARCH_PID"
```

A server can handle successive searches; restart an owned server if its
session stalls.
