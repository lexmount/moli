# Reverse image search engines

Select by task and current response; row order does not imply priority.
Outcomes depend on network, region, session, input, and date, not a fixed
success rate. “Not retested locally” means no new evidence.

`ENC` is an encoded image URL; `RAW` is the original URL value, encoded by
the parameter builder when inserted into a query. Local image tests used D,
except the Baidu upload used F:

- **D:** NASA Aldrin photo, `https://images-assets.nasa.gov/image/as11-40-5903/as11-40-5903~medium.jpg`.
- **A:** Google logo, `https://www.google.com/images/branding/googlelogo/2x/googlelogo_color_272x92dp.png`.
- **F:** Baidu logo, `https://www.baidu.com/img/flexible/logo/pc/result.png`.
- **W:** Wikimedia Commons Starry Night thumbnail, retained as a historical hotlink negative control; its exact URL was not supplied.

`listings` describes results, not their relevance; `no` means no matches.
`method OK` means submission succeeded. `near`, `near-win`, `partial`, and
`unconfirmed` do not establish a completed search. Access and transport errors
remain separate from index misses.

See [image recipes](imagesearch.md) and [CDP/API recipes](cdp-driver.md) for
execution. Partial paths are relative to the named service. Search this table
by engine, index, endpoint, field, or error.

| Engine | Search / index / route | Entry point / method | Supplied observations | Local observations · 2026-09-10 |
| --- | --- | --- | --- | --- |
| 123RF | Image · Stock · API / browser | `123rf.com`; `POST /apicore/search/reverse/upload` field `image` → `reverse-new?fid=` | listings; Stock CBIR; Apollo alts. | Not retested locally |
| 1688 | Image · Shopping / fashion · Browser | `s.1688.com/` | login; `login.taobao.com/?…_____tmd_____…login_jump` | Not retested locally |
| 1688 | Image · Shopping / fashion · URL | `https://s.1688.com/youyuan/index.htm?tab=imageSearch&imageAddress=ENC` | anti-bot; “unusual traffic” / slider captcha | unconfirmed: D; blank Markdown; HTML contains scripts but no visible result text. |
| 360 | Image · Web images · Browser | `st.so.com/`; `#stUpload` `input[name=upload]` | listings (D); `/r?src=st&srcsp=home` 「相似图片」, hrefs `q=人类登月球图片`, `q=月球`, `q=阿波罗宇航服`, `q=尼尔阿姆斯特朗`; image 1280x1249 | unconfirmed: D; first upload navigated without confirmed result cards; a follow-up did not leave the upload page within 15 seconds. |
| 360 | Image · Web images · Browser | `st.so.com/`; paste `#stInput` + `.st_submit` only | no / flaky; often 「暂时无法为您识别」 | Not retested locally |
| 360 | Image · Web images · URL | `https://st.so.com/r?imgurl=ENC` | no; old `imgurl` param ignored | unconfirmed: D; upload/search landing page; no confirmed result cards. |
| 360 | Image · Web images · URL | `https://st.so.com/r?src=st&img_url=RAW` | listings (D); Also `#stInput[name=img_url]` / POST FormData `img_url`. | no listings: D; request completed, page reported no related content. |
| 360 | Image · Web images · URL | `https://st.so.com/stu?imgurl=ENC` | no; landing page, not results | unconfirmed: D; upload/search landing page; no confirmed result cards. |
| 360 img_url | Image · Web images · Browser | `st.so.com`; GET `/r?src=st&img_url=` or POST FormData `img_url` | listings (D); NASA → 1280x1249 + 相似图片 + Apollo captions | Not retested locally |
| 3d.iqdb.org | Image · Anime / art · API / browser | `https://3d.iqdb.org/`; file or `?url=`; URL: `https://3d.iqdb.org/?url=ENC` | method OK / no on NASA; 3D index; Possible match rows. | listings: D; no relevant matches, but weak Possible match rows; no identification of the photo. |
| akg-images | Image · Stock · API / browser | `akg-images.com`; ImageHash → `SearchAssetsByHashes` | near; CDP filechooser unstable. | Not retested locally |
| Alamy imageurl | Image · Stock · Method / access check | `https://www.alamy.com/search.html?imageurl=ENC`; fetch | no (D); Reverse UI present; “Page 0 of 0”. CDP file input assignable; no NASA captions that run. | unconfirmed: D; search/upload chrome; no confirmed image matches. |
| Alibaba.com | Image · Shopping / fashion · Browser | `alibaba.com/`; Image Search tab (119×64) | no; `nFile=0`; their Lens is a Chrome extension CTA | Not retested locally |
| Alibaba.com | Image · Shopping / fashion · URL | `https://www.alibaba.com/picture-search.html?imageAddress=ENC` | no; 404 CMS page | route error: D; CMS 404-Error page, with ordinary site navigation. |
| Amazon StyleSnap | Image · Shopping / fashion · API / browser | `amazon.com/shopthelook`; `/stylesnap/upload?stylesnapToken=` | listings (shop); ASINs/boxes. | Not retested locally |
| ascii2d | Image · Anime / art · Browser | `ascii2d.net/`; `#file-form` + that form's submit | anti-bot; homepage loads (二次元画像詳細検索, nFile=1), `/search/file` hits CF | Not retested locally |
| ascii2d | Image · Anime / art · URL | `https://ascii2d.net/search/url/ENC` | anti-bot; Cloudflare | anti-bot: D; security-verification waiting page. |
| Baidu | Image · Web images · Browser | `graph.baidu.com/`, `m.baidu.com`, `image.baidu.com`, `image.baidu.com/search/index` | no widget; goto timeout / mobile home / AI text-to-image homepage; `nFile=0`; `#ci-file-upload-btn` is a DIV | Not retested locally |
| Baidu | Image · Web images · Browser | `graph.baidu.com/pcpage/index` (also `?tpl_from=pc`); `setInputFiles` or DataTransfer on `input.general-upload-file`, **no** 识图一下 click; `input[name=file]` is another recorded selector | listings (F); `POST /upload?uptime=` → `graph.baidu.com/s?...session_id=`; `window.cardData` cardHeader 百度 / product 相关商品 / simipic; `GET /ajax/pcsimi?...limit=30` returns 30; wise view 「相似度81% 百度 百度百科」 | listings: F; upload and logo identification succeeded. `window.cardData` was an array of `cardHeader`, `product`, `same`, and `simipic` records; readiness rechecked. |
| Baidu | Image · Web images · Browser | `graph.baidu.com/pcpage/index` (also `?tpl_from=pc`); `setInputFiles` or DataTransfer on `input.general-upload-file`, no extra click; seed D or `bd_logo1.png` | no; `errpage?pron=pcupload` 「未找到相关结果」 — index miss, upload itself succeeded | Not retested locally |
| Baidu | Image · Web images · Browser | `graph.baidu.com/pcpage/index` (also `?tpl_from=pc`); assign **then click 识图一下** | stay; aborts the POST, page never leaves `pcpage/index`. The extra click cancels the automatic upload | Not retested locally |
| Baidu | Image · Web images · URL | `https://graph.baidu.com/details?isfromtusoupc=1&tn=pc&carousel=0&image=ENC` | no; 「未找到相关结果」, referer `errpage?pron=details` | no listings: D; 未找到相关结果; details error page. |
| Baidu | Image · Web images · URL | `https://graph.baidu.com/pcpage/index?image=ENC` | no; upload landing 「拖拽、Ctrl + V」; param ignored | unconfirmed: D; upload landing page and upload-failed text; no confirmed result cards. |
| Baidu | Image · Web images · URL | `https://graph.baidu.com/s?src=0&image_source=other&rt=0&fr=1&image=ENC` | no; 「未找到相关结果」 on D, A, F and `bd_logo1.png`; `--delay-ms 8000` no help | no listings: D; 未找到相关结果. |
| Baidu | Image · Web images · URL | `https://image.baidu.com/n/pc_search?queryImageUrl=ENC&uptype=urlsearch` | no; 「图片上传失败，请重新上传」 | submission error: D; 图片上传失败，请重新上传 on the upload landing page. |
| Baidu | Image · Web images · URL | `https://image.baidu.com/pcdutu?queryImageUrl=ENC` | no; same upload-fail homepage | submission error: D; 图片上传失败，请重新上传 on the upload landing page. |
| Baidu | Image · Web images · URL | `https://image.baidu.com/search/wiseala?tn=wiseala&isWiseImg=1&image=ENC` | no; 0-byte markdown, 39-byte blank html | unconfirmed: D; empty Markdown and a 39-byte blank HTML document. |
| Baidu sign deeplink | Image · Web images · API / browser | `graph.baidu.com/s?sign=HEX&tpl_from=pc&f=all`; reopen after upload | listings; no re-upload; session_id alone errpage | Not retested locally |
| Berify / Everypixel / Reversely.ai | Image · Web images · Method / access check | homes; upload | no / anti-bot; 404 search.html / CF on /search / marketing stay. | Not retested locally |
| Bing | Image · Web images · URL | `https://www.bing.com/images/search?q=ENC` | no; Visual Search “Can't use this link”, not visual matches | unsupported route: D; URL-as-keyword image results plus “Can't use this link”; no confirmed reverse-image match. |
| Bing | Image · Web images · URL | `https://www.bing.com/images/search?q=imgurl%3AENC&view=detailv2&iss=sbi` | no; “Can't use this link.”; 1.1.3 returned NASA listings on URL-paste, while the 1.1.4 supplied recheck again failed. | listings: D; Apollo 11 query, visual-match cards, and outbound pages. |
| Bing | Image · Web images · URL | `https://www.bing.com/images/search?q=imgurl:RAW&view=detailv2&iss=sbi&form=SBIHLP` | no; “Can't use this link.”; 1.1.3 returned NASA listings on URL-paste, while the 1.1.4 supplied recheck again failed. | listings: D; Apollo 11 query, visual-match cards, and outbound pages. |
| Bing | Image · Web images · URL | `https://www.bing.com/images/search?view=detailv2&iss=sbi&form=SBIVSP&sbisrc=UrlPaste&q=imgurl:RAW` | no; “Can't use this link.”; 1.1.3 returned NASA listings on URL-paste, while the 1.1.4 supplied recheck again failed. | listings: D; Apollo 11 query, visual-match cards, and outbound pages. |
| Bing | Image · Web images · URL | `https://www.bing.com/images/search?view=detailv2&iss=sbi&imgurl=ENC` | no; “Can't use this link.”; 1.1.3 returned NASA listings on URL-paste, while the 1.1.4 supplied recheck again failed. | listings: D; Apollo 11 query, visual-match cards, and outbound pages. |
| Bing | Image · Web images · URL | `https://www.bing.com/images/searchbyimage?FORM=IRSBIQ&cbir=sbi&imgurl=ENC` | no; “Can't use this link.”; 1.1.3 returned NASA listings on URL-paste, while the 1.1.4 supplied recheck again failed. | listings: D; Apollo 11 query, visual-match cards, and outbound pages. |
| Bing | Image · Web images · URL | `https://www.bing.com/visualsearch/share?imgurl=ENC` | no; Page not found | route error: D; page not found. |
| Bing | Image · Web images · URL | `https://www.bing.com/visualsearch?q=imgurl:ENC` | no; marketing landing, query ignored | Not retested locally |
| Bing CN | Image · Web images · URL | `https://www.bing.cn/…` | network; DNS `failed to resolve www.bing.cn:443` | Not retested locally |
| Bing ImageKnowledge | Image · Web images · API / browser | `GET /images/api/custom/knowledge?…&skey=&imgurl=`; warm /images, scrape skey | listings (JSON); structured entities; needs skey | Not retested locally |
| Bing kblob XHR | Image · Web images · Browser | `bing.com/images`; `POST /images/kblob?iss=sbiupload&FORM=SBIIRP&sbisrc=ImgPicker` FormData field **`image`** → follow `redirectUrl` | listings (NASA→Apollo 11; google.png→q=Google); 2026-09-06. Also public `imgurl:` UrlPaste GET works for NASA. Raw body / `file` / `imageBinary` fail; the supplied 2026-09-09 recheck was inconsistent. | partial: D; kblob HTTP 200 and an Apollo query URL; the subsequent result page required verification. |
| Bing native file | Image · Web images · Browser | `bing.com/images`; `#sb_sbi` + `#sb_fileinput` setInputFiles | no; auto kblob still 400/500 (1.1.3 recheck) | Not retested locally |
| Bridgeman Images | Image · Stock · API / browser | `bridgemanimages.com`; `POST /image-search/upload` → `reverse-image?image-hash=` | listings; `?url=` not CBIR. | Not retested locally |
| CN social XHS/Douyin/Kuaishou/Weibo | Image · Web images · Method / access check | No request details supplied | anti-bot / network / no-widget; No usable logged-out 以图搜图. | Not retested locally |
| CopyChecker | Image · Web images · API / browser | `copychecker.com`; `POST /api/ai-reverse-image-search` raw b64 | listings; WEB_DETECTION; Apollo pages. | listings: D; earlier JSON HTTP 200 with `webEntities`, matching pages, and `bestGuessLabels` for Buzz Aldrin/Apollo 11; recheck hit the 8-second evaluation timeout. |
| CopySeeker | Image · Web images · API / browser | `copyseeker.net`; `GET /search?imageurl=` → RSC discovery JSON; URL: `https://copyseeker.net/search?imageurl=ENC` | listings; NASA → Aldrin/Apollo; 187 pages. Trust RSC not UI. | unconfirmed: D; URL fetch showed marketing/consent content; the full RSC discovery flow was not rechecked. |
| danbooru2022 HF | Image · Anime / art · API / browser | SmilingWolf space; Gradio upload → predict | near / needs API; Anime index. | Not retested locally |
| Decopy.ai RIS | Image · Web images · API / browser | `api.decopy.ai`; upload-image → search-by-image → get-job | listings (fanout); quota/login caveats. | Not retested locally |
| Depositphotos | Image · Stock · API / browser | `depositphotos.com`; `GET /search/by-images.html?url=` / upload `dp_files` | listings; Stock CBIR; NASA moon alts. | listings: D; earlier public URL search returned astronaut/moon stock results; recheck timed out waiting for headers with both readiness modes. Upload variant not retested. |
| DuckDuckGo | Image · Web images · Method / access check | No request details supplied | network; TLS EOF from the supplied test environment. | Not retested locally |
| DupliChecker / SmallSEOTools / PrepostSEO | Image · Web images · Method / access check | reverse-image pages; upload | anti-bot / no-widget; CF/captchaModal; PrepostSEO has no real file input in HTML. | Not retested locally |
| EarthExplorer HF | Image · Geolocation / satellite · API / browser | `ml4sustain-earthexplorer.hf.space`; Gradio `/search_image` | listings (sat); Thematic satellite knn. | Not retested locally |
| Ecosia images | Image · Web images · Browser | `ecosia.org/images?q=` | anti-bot; “Confirm you're not a robot” | Not retested locally |
| EUIPO eSearch | Image · Trademarks / designs · API / browser | EUIPO copla; uploadAndSegments → ctm/rcd search | listings (TM); No Apollo captions. | Not retested locally |
| FaceCheck.ID | Image · Faces · API / browser | facecheck.id; upload_pic OK; search NEED_CAPTCHA | near; Face index. | Not retested locally |
| Flim | Image · Film / TV · API / browser | `api.flim.ai`; uploadSimilarImage → `/2.0.0/search` similar_picture_id | listings; Film/TV stills; Apollo titles. | Not retested locally |
| FuzzySearch | Image · Anime / art · API / browser | `api.fuzzysearch.net`; `/v1/image` + public x-api-key | listings (FA); NASA often miss. | Not retested locally |
| GeoInfer | Image · Geolocation / satellite · API / browser | api.geoinfer.com; predict trial/key | near; Geoloc not CBIR pages. | Not retested locally |
| Getty / iStock | Image · Stock · API / browser | Unisporkal; upload-data → S3 PUT → search-by-image | listings; Apollo stock alts. | Not retested locally |
| Google | Image · Web images · Browser | `images.google.com`, `google.co.jp/imghp`; JS-click 0×0 camera, then `input[name=encoded_image]` | no; assign ok, no Lens nav, empty innerText (layout 0×0, not a wall). Earlier `.com` paste path gave Lens 403 | Not retested locally |
| Google | Image · Web images · URL | `https://www.google.com/search?tbm=isch&tbs=sbi:ENC` | no; wrong template; Images chrome | consent: D; consent page; no retrieved image matches. |
| Google | Image · Web images · URL | `https://www.google.com/searchbyimage?sbisrc=4chanx&image_url=ENC` | anti-bot; “unusual traffic” CAPTCHA interstitial | consent: D; consent page; no retrieved image matches. |
| Google JP | Image · Web images · URL | `https://www.google.co.jp/searchbyimage?image_url=ENC` | anti-bot; “unusual traffic” CAPTCHA interstitial | consent: D; Japanese consent page; no retrieved image matches. |
| Google Lens | Image · Web images · URL | `https://lens.google.com/uploadbyurl?url=ENC` | anti-bot; “unusual traffic” CAPTCHA; same with Chrome/131 UA | anti-bot: D; unusual-traffic CAPTCHA. |
| Google Lens upload | Image · Web images · API / browser | `searchbyimage/upload`; multipart `encoded_image` → vsrid | near / CAPTCHA; Recipe exact. | Not retested locally |
| huaban | Image · Web images · Browser | `huaban.com/explore` | anti-bot; Tencent EdgeOne security verification | Not retested locally |
| Icons8 | Image · Icons / fonts · API / browser | `search-app.icons8.com`; `POST …/search/file` field `file` | listings (icons) | Not retested locally |
| IconScout / Shutterstock / Adobe Stock | Image · Stock · Method / access check | RIS / home | anti-bot; Cloudflare still. | Not retested locally |
| IMAGO Creative | Image · Stock · API / browser | `api.imago-images.com`; `POST /ewb/search/reverse-image` field `image` + SPA Basic | listings; Creative index; NASA Aldrin/Apollo. | Not retested locally |
| ImgOps | Image · Hub · URL | `https://imgops.com/RAW` | no; link hub, “IMAGE IS NOT HOSTED HERE.” | hub: D; image-operation and outbound search links; no independent index result. |
| ImgOps hub | Image · Hub · Method / access check | `https://imgops.com/` + public host/path (no scheme); fetch or CDP | hub (D); Fan-out links: Lens uploadbyurl, Google searchbyimage, Bing searchbyimage, Yandex imageview, TinEye, Baidu graph, IQDB, SauceNAO, Alamy `?imageurl=`. Needs already-public URL (no anonymous hosts). | hub: D; image-operation and outbound search links; no independent index result. |
| imgs.ai | Image · Anime / art · API / browser | imgs.ai; `POST /interface` clip_vit + museum model | listings; Museum CLIP. | Not retested locally |
| IMPI MARCia | Image · Trademarks / designs · API / browser | Mexico; upload/bulk → record → result IMAGE | listings (TM) | Not retested locally |
| INPIT GrIP | Image · Trademarks / designs · API / browser | jpDesign; `#ImageFile` → `/search/` | listings (design); Observed access ends 2026-12-28. | Not retested locally |
| IP Australia | Image · Trademarks / designs · API / browser | au TM; `/trademarks/search/internal/image` + doSearch | listings (TM) | Not retested locally |
| IPONZ | Image · Trademarks / designs · API / browser | nz TM; UploadImageHandler → PublicSearchHandler ty=2 | listings (TM) | Not retested locally |
| IPOS Similar Mark | Image · Trademarks / designs · API / browser | Singapore; ConvertImage → SearchSimilarMark | near / recaptcha | Not retested locally |
| IQDB | Image · Anime / art · Browser | `https://iqdb.org/`; `#url` paste or file | listings; Possible-match cards; first visit may bounce to `danbooru.iqdb.org?ckatt=1`, retry stays multi-service | Not retested locally |
| IQDB | Image · Anime / art · URL | `https://iqdb.org/?url=ENC` | listings (A, D); `Retrieving … OK` + Possible match 33–46% (anime; does not ID real photos) | listings: D; weak Possible match scores around 44–46%; no identification of the photograph. |
| IQDB (W) | Image · Anime / art · URL | `https://iqdb.org/?url=ENC` (W) | no; “Not an image … application/octet-stream” | Not retested locally |
| JD | Image · Shopping / fashion · Browser | `search.jd.com/Search?keyword=`; `#searchImgInput` | login; assign ok → `passport.jd.com/new/login.aspx`; `jd.com` itself redirects to corporate | Not retested locally |
| JD | Image · Shopping / fashion · URL | `https://search.jd.com/image?imgurl=ENC` | anti-bot; 扫码登录 | login: D; JD QR/password/SMS login page. |
| Karma Decay | Image · Web images · URL | `https://karmadecay.com/search?q=ENC` | network; failed in about 5s, also via curl | network: D; response-header readiness timed out after 25 seconds on both attempts. |
| Lens App | Image · Web images · API / browser | `lensapp.io`; `POST /api/reverse.php?endpoint=lens\|openai` FormData `image`+`lang`+`country` | method OK / rate_limited; Recipe exact; daily rate_limit that run. | Not retested locally |
| Lenso.ai | Image · Web images · API / browser | `lenso.ai`; `POST /api/upload` JSON base64+`El(tkn)` → `/api/search` | near-win / 429; Upload OK; search human-verify queue. | Not retested locally |
| Lenso.ai | Image · Web images · Browser | `lenso.ai/en`; `POST /api/upload` JSON base64+`El(tkn)` | anti-bot; Upload API works → `/en/results/<id>` but Prosopo captcha blocks listings (2026-09-06). Listings were still blocked. | Not retested locally |
| Lenso.ai | Image · Web images · URL | `https://lenso.ai/en?url=ENC` | no; Cookiebot/Turnstile + marketing | unconfirmed: D; marketing/upload landing page; no confirmed image matches. |
| Lenso.ai | Image · Web images · URL | `https://lenso.ai/search?url=ENC` | no; “This page doesn't work. Upload a new image instead.” | route error: D; page not found. |
| Lykdat | Image · Shopping / fashion · API / browser | `lykdat.com` / `api.lykdat.com/search`; FormData field **`image`** (or `image_url`) + page `api_key` | listings (fashion); 2026-09-06. UI assign alone insufficient. Fashion-biased product CBIR. | Not retested locally |
| Mauritius Rocketloop | Image · Stock · API / browser | `mauritius.rocketloop.ai`; `POST /image_search` | method-OK / NASA miss; Resolve IDs via sodatech assets API. | Not retested locally |
| museum-semantic-search | Image · Anime / art · API / browser | vercel.app; `POST /api/mcp/image-search` dataURL | listings; Jina-CLIP Met OA. | Not retested locally |
| NumLookup | Image · Web images · API / browser | numlookup.com; upload_photo `file1` → search+captcha | near | Not retested locally |
| Open-i NLM | Image · Medical images · API / browser | openi.nlm.nih.gov; upload+qimg | near / broken upload | Not retested locally |
| Pic Detective | Image · Web images · API / browser | `picdetective.com`; `POST /api/upload` FormData **`file`** → `GET /api/search?url=&search_type=similar` | listings; NASA → Apollo/Aldrin matches; the supplied 2026-09-09 recheck was inconsistent. | Not retested locally |
| Picarta.ai | Image · Geolocation / satellite · API / browser | picarta.ai; /classify TOKEN enterprise | near; Quota/geoloc. | Not retested locally |
| Picture-Alliance | Image · Stock · API / browser | sodatech simsearch-pa; fileupload/similarity + Bearer | near / login 403; Recipe exact; needs auth. | Not retested locally |
| Pinterest | Image · Web images · Browser | `/`, `/search/pins/`, pin-builder | login/no widget; logged-out, `nFile=0`, no visual-search input | Not retested locally |
| Pinterest | Image · Web images · URL | `https://www.pinterest.com/visual-search/?image_url=ENC` | no; logged-out welcome SPA | login: D; logged-out welcome page. |
| PIXTA | Image · Stock · API / browser | `pixtastock.com`; presigned → S3 → retrieve_vsi | listings | Not retested locally |
| Portrait Matcher | Image · Anime / art · API / browser | ox.ac.uk; `POST getFace` field `im` | listings (ArtUK) | Not retested locally |
| RepostSleuth | Image · Reddit · API / browser | `api.repostsleuth.com`; `GET /api/image?url=&target_match_percent=50` or `POST` multipart `image`; URL: `https://api.repostsleuth.com/api/image?url=ENC&target_match_percent=50` | listings; Reddit pHash; NASA → Aldrin posts. | listings: D; JSON returned 993 matches, including Apollo/Aldrin Reddit posts; individual relevance varies. |
| RootAbout | Image · Archive / books · Browser | `https://rootabout.com/`; `#loadfile` + form POST `/search.php` | no (method OK; Archive/OL miss on cow seed); Local upload works (2026-09-06). Index is Internet Archive + OpenLibrary only — not a general web CBIR. | Not retested locally |
| RootAbout NASA check | Image · Archive / books · Method / access check | `https://rootabout.com/`; `#loadfile` + Internet Archive | no (method OK); `POST /search.php` body: “RootAbout found no matches at the Internet Archive.” Archive/OL-only index. | Not retested locally |
| same.energy | Image · Web images · API / browser | `imageapi.same.energy`; `PUT /upload?length=` → `GET /search?i=` | listings; CLIP general-web; Apollo titles. | Not retested locally |
| SauceNAO | Image · Anime / art · Browser | `https://saucenao.com/` homepage; `#urlInput` + `form.submit()`, or `#fileInput` | listings (D); `search.php` “Sauce Found?”; 83.23% Apollo 11, 79.10% Apollo 11 (on moon), 80.77% Moon Landing Conspiracy. Homepage submission succeeded while direct GET was blocked | listings: D; tested the homepage URL form, which returned similarity rows and Apollo/Moon entries. File variant not retested. |
| SauceNAO | Image · Anime / art · URL | `https://saucenao.com/search.php?db=999&url=ENC` | anti-bot; Cloudflare on GET | anti-bot: D; security-verification waiting page. |
| SauceNAO | Image · Anime / art · URL | homepage → `POST /search.php` FormData `url=` | listings (D); 80%+ Apollo/Moon. Optional `db=999`. | Not retested locally |
| searchbyimage.com | Image · Shopping / fashion · Method / access check | `https://www.searchbyimage.com/?lang=en`; fill `#search_query` with **public** image URL + Enter → `POST /search.php` | listings (D); AliExpress product cards; NASA → “Astronaut…” titles; UI `Aliexpress(30)`. Local file via form-urlencoded `search_query=encodeURIComponent(data:image/jpeg;base64,…)` + `acab` (double-encode); multipart alone fails. Marketplace CBIR. | Not retested locally |
| SearchThisImage | Image · Web images · API / browser | Vision hub; `/v1/search/url` + upload `file` + API key | near / 503; CopyChecker-shaped. | Not retested locally |
| Sogou | Image · Web images · Browser | `pic.sogou.com/`, `index/pic.html`, `www.sogou.com/` | no widget; keyword chrome, `nFile=0` | Not retested locally |
| Sogou | Image · Web images · Browser | `pic.sogou.com/ris`; hidden `input[type=file][accept=image/*]`, no extra click | listings (D); auto-nav 3.6–4.5s to `ris.sogou.com/ris?flag=1&from=pic_result_list&query=http://img04.sogoucdn.com/…`; 「该图片可能是：月球表面」; 巴兹·奥尔德林 / buzz aldrin cards; sohu.com outbound | listings: D; upload HTTP 200, automatic navigation, Aldrin/Apollo source cards. |
| Sogou | Image · Web images · URL | `https://pic.sogou.com/ris?flag=1&w=1000&h=1000&url=ENC` | no; 「很抱歉没有找到相关图片结果」 | unconfirmed: D; image-search upload shell; no confirmed result cards. |
| Sogou | Image · Web images · URL | `https://pic.sogou.com/ris?pic_url=ENC&flag=1` | no; 「很抱歉没有找到相关图片结果」 | unconfirmed: D; image-search upload shell; no confirmed result cards. |
| Sogou | Image · Web images · URL | `https://pic.sogou.com/ris?query=ENC&flag=1` | no; 「很抱歉没有找到相关图片结果」 | unconfirmed: D; image-search upload shell; no confirmed result cards. |
| Sogou CDN query | Image · Web images · API / browser | `ris.sogou.com/ris?query=http://imgXX.sogoucdn.com/app/a/100520146/{hash}`; reuse upload CDN URL | listings; external public URLs still forbid | Not retested locally |
| Startpage images | Image · Web images · Method / access check | No request details supplied | anti-bot; Warp captcha-block. | Not retested locally |
| StillsLab | Image · Film / TV · API / browser | stillslab; `/data/frames?search=` + `/frame/<id>/similar` | listings (text+sim); Visual upload 401. | Not retested locally |
| Stocksy | Image · Stock · API / browser | `stocksy.com`; `POST /home/visualsearchimgendpoint` → imagefeeds | listings; (url= was FP). | Not retested locally |
| SuperStock | Image · Stock · API / browser | `superstock.com`; `POST /api/FileUpload/uploadFile` → `/search?uploadId=` | listings; Apollo captions. | Not retested locally |
| Taobao | Image · Shopping / fashion · Browser | `s.taobao.com/`; `#image-search-custom-file-input` | login/no; assign ok, stays on 搜同款 chrome with `login.jhtml` links | Not retested locally |
| Taobao | Image · Shopping / fashion · URL | `https://s.taobao.com/list?imgfile=ENC` | anti-bot; login | login: D; password/SMS/QR login page. |
| Taobao | Image · Shopping / fashion · URL | `https://s.taobao.com/search?app=imgsearch&image_url=ENC` | no; 请登录 + 图搜 UI | login/unconfirmed: D; search chrome with login links; no confirmed image matches. |
| Taobao s. | Image · Shopping / fashion · Method / access check | `#image-search-custom-file-input`; upload | login; Unchanged. | Not retested locally |
| TinEye | Image · Web images · Browser | `tineye.com/`; wait 10s / 25s | anti-bot; “Just a moment…”, GET `/` 403, no `#url_box` | Not retested locally |
| TinEye | Image · Web images · URL | `https://tineye.com/search/?url=ENC` | anti-bot; “Checking your browser…” Ray ID | unconfirmed: D; TinEye upload/search shell; no confirmed matches in this fetch. |
| TMDN DesignView | Image · Trademarks / designs · API / browser | `#/dsview`; ds/uploadAndSegments → dsv/results | listings (design) | Not retested locally |
| TMview API | Image · Trademarks / designs · API / browser | TMDN tmdsview-web; tm/uploadAndSegments → search/results | listings (TM); UI may Rejected. | Not retested locally |
| trace.moe | Image · Anime / art · Browser | `trace.moe/`; `input[name="files[]"]` | no; stays on “paste or drop image here”; later gotos time out | Not retested locally |
| trace.moe | Image · Anime / art · URL | `https://trace.moe/?url=ENC` | no; consumes `/image-proxy?url=`, then “No exact matching results found” (anime scenes only) | no listings: D; the page reports no exact matching results. |
| trace.moe API | Image · Anime / art · API / browser | `https://api.trace.moe/search?anilistInfo=1`; FormData `image` / raw jpeg / `?url=`; URL: `https://api.trace.moe/search?anilistInfo=1&url=ENC` | listings (anime); Homepage drop often stalls. Anime-only; NASA false-positives. | listings: D; JSON returned 10 anime candidates (top similarity about 0.77); false matches for this photograph. |
| tuxiang.weibo.com | Image · Web images · Browser | No request details supplied | network; `net::ERR_FAILED` | Not retested locally |
| Unsplash visual | Image · Stock · API / browser | `unsplash.com`; `POST /napi/search/by_image` FormData `file` → `{uuid}` then **`moli fetch`** `https://unsplash.com/s/visual/{uuid}` | listings; Moli 1.1.4 recheck (was near/Anubis). SPA markdown has Visually similar + Aldrin; NAPI result GET still Anubis. | access: D; upload HTTP 401, so no result UUID or listings were obtained. |
| Westend61 | Image · Stock · API / browser | `westend61.de`; `POST /imageSearch/reverseImageSearch` field `file` | listings; Anubis on bare curl. | Not retested locally |
| WhatFontIs | Image · Icons / fonts · API / browser | whatfontis.com; userfile → step5-results | listings (fonts) | Not retested locally |
| Ximilar Stock demo | Image · Stock · API / browser | `api.ximilar.com/demo`; `call_service_rest` ID-Photo-Similarity | listings (stock-sim); Weak Apollo captions; visual-sim OK. | Not retested locally |
| ya.ru / yandex.com.tr | Image · Web images · API / browser | `https://ya.ru/images/search?rpt=imageview&url=ENC`; `https://yandex.com.tr/images/search?rpt=imageview&url=ENC`; fetch | listings; Regional Yandex parity. | listings: D; ya.ru and yandex.com.tr each returned Apollo/Buzz Aldrin tags, similar images, and source links. |
| Yahoo Images | Image · Web images · URL | `https://images.search.yahoo.com/images/view?imgurl=ENC` | no; chrome only | unconfirmed: D; Yahoo Images home/search chrome; no confirmed image matches. |
| Yahoo JP / Naver / Daum / DuckDuckGo / Weibo search / Toutiao | Image · Web images · Browser | homepages + image SERPs; inspect | no widget; `nFile=0`; keyword image SERPs only. DDG did not network-fail under CDP | Not retested locally |
| Yahoo JP / Naver / Daum / Qwant / Brave / Seznam / Rambler / Mail.ru images | Image · Web images · Method / access check | image SERPs; inspect | no-widget; Keyword only; Yahoo JP camera is app-only. | Not retested locally |
| Yandex (W) | Image · Web images · URL | `https://yandex.com/images/search?rpt=imageview&url=ENC` (W) | no; `![Loaded image]` + “No matching results.” — seed problem, not engine | Not retested locally |
| Yandex .kz/.by/.uz | Image · Web images · API / browser | `yandex.kz` / `.by` / `.uz`; `?rpt=imageview&url=`; `/images/search?rpt=imageview&url=ENC` on each host | listings (alias); Same CBIR as .com/.ru/.tr. | listings: D; .kz, .by, and .uz each returned Apollo/Buzz Aldrin tags, similar images, and source links. |
| Yandex apphost XHR | Image · Web images · Browser | `yandex.com/images/` then fetch imageview; `POST /images-apphost/image-download?cbird=111… (raw Blob, not FormData)` → `cbir_id` → `moli fetch ?rpt=imageview` | listings (local cow→《牛来》); Supports local files without a public URL. Engine stores preview under `avatars.mds.yandex.net/get-images-cbir/`. Fetched Markdown on the imageview URL had more content than the CDP SPA. | listings: D; raw upload HTTP 200, `cbir_id`, then Apollo/Aldrin tags and source links in fetched Markdown. |
| Yandex file input | Image · Web images · Browser | `yandex.com/images/`; `.CbirCore-FileInput` | no; Popup 0×0; setInputFiles ok but no upload POST (still true 1.1.3) | Not retested locally |
| Yandex touch | Image · Web images · URL | `https://yandex.com/images/touch/search?rpt=imageview&url=ENC` | listings (D); same index as com | listings: D; Apollo/Buzz Aldrin tags, similar images, and source links. |
| Yandex.com | Image · Web images · URL | `https://yandex.com/images/search?rpt=imageview&url=ENC` | listings (A, D); `## Image appears to contain` man on the moon / buzz aldrin / apollo 11; `## Similar images`; `## Sites` | listings: D; Apollo/Buzz Aldrin tags, similar images, and source pages. |
| Yandex.ru | Image · Web images · URL | `https://yandex.ru/images/search?rpt=imageview&url=ENC` | listings (A, D); `## Кажется, на изображении` + `## Похожие` + `## Сайты` | listings: D; Apollo/Buzz Aldrin tags, similar images, and source pages. |
| Yandex.ru apphost | Image · Web images · API / browser | `yandex.ru/images/`; same raw-Blob POST as `.com` | listings; Parity confirmed (Apollo tags). | Not retested locally |
