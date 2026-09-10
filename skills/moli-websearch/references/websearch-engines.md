# Web search engines

Select by task and current response; row order does not imply priority.
Outcomes depend on network, region, session, input, and date, not a fixed
success rate.

`Q` is the encoded query (local test: `lexmount moli browser github`).
`listings` describes results, not their relevance; `no` means no matches.
Access and transport errors remain separate from index misses.

See [keyword recipes](websearch.md) for execution.

| Engine | Result URL | Supplied observations | Local observations · 2026-09-10 |
| --- | --- | --- | --- |
| 360 | `https://www.so.com/s?q=Q` | Readiness: `done` | listings: broad or unrelated entries mixed into the results. |
| Baidu | `https://www.baidu.com/s?wd=Q` | Readiness: `done` | listings: recheck returned LexMount and a Moli video among unrelated results. |
| Bing | `https://www.bing.com/search?q=Q` | Readiness: `done`; unrelated or cloaked result rows were observed. | listings: 10 extracted rows, but unrelated to the query. |
| Brave | `https://search.brave.com/search?q=Q` | Empty output or HTTP 429 | listings: Relevant lexmount/moli repository and related result rows. |
| Daum | `https://search.daum.net/search?q=Q` | Readiness: `done` | no listings: explicit no-results message for this query. |
| DuckDuckGo HTML | `https://html.duckduckgo.com/html/?q=Q` | Network failure after about five seconds | listings: Relevant lexmount/moli repository and project result rows. |
| DuckDuckGo Lite | `https://lite.duckduckgo.com/lite/?q=Q` | Network failure after about five seconds | listings: Relevant lexmount/moli repository and project result rows. |
| Ecosia | `https://www.ecosia.org/search?q=Q` | Readiness: `done`; a robot wall was observed in some runs. | anti-bot: empty Markdown; HTML inspection showed a Cloudflare challenge. |
| Google | `https://www.google.com/search?q=Q` | CAPTCHA | listings: Repository and other relevant web results are present alongside a German consent notice. |
| Kagi | `https://kagi.com/search?q=Q` | Turnstile challenge | login: Sign-in page. |
| Marginalia | `https://search.marginalia.nu/search?query=Q` | Readiness: `done`; Returned results after earlier bot-wait failures; sparse index may miss long queries | anti-bot: robot waiting page. |
| MetaGer | `https://metager.org/meta/meta.ger3?eingabe=Q` | Access key required | login/key: Access-key landing page. |
| Mojeek | `https://www.mojeek.com/search?q=Q` | HTTP 403 | anti-bot: ALTCHA verification page. |
| Nate | `https://search.nate.com/search/all.html?q=Q` | Readiness: `done`; Korean blogs and shopping | no listings: explicit no-results message for this query. |
| Naver | `https://search.naver.com/search.naver?query=Q` | Readiness: `domcontentloaded` | listings: repository and relevant project descriptions; `domcontentloaded`. |
| Public Searx instances | No instance URL supplied; use an instance selected for the task | Anti-bot responses | Not tested: no specific instance supplied |
| Qwant | `https://www.qwant.com/?q=Q&t=web` | HTTP 403 | access: The page reports temporary unavailability and HTTP 403. |
| Rambler | `https://nova.rambler.ru/search?query=Q` | SmartCaptcha | anti-bot: Text CAPTCHA; no organic results retrieved. |
| Seznam | `https://search.seznam.cz/?q=Q` | Readiness: `done` | listings: LexMount GitHub entries after switching from empty `done` output to `domcontentloaded`. |
| Shenma | `https://m.sm.cn/s?q=Q` | Readiness: `done` | listings: articles about lexmount/moli. |
| Sogou | `https://www.sogou.com/web?query=Q` | Readiness: `done` | anti-bot: CAPTCHA page. |
| Sogou Weixin | `https://weixin.sogou.com/weixin?type=2&query=Q` | Readiness: `domcontentloaded`; WeChat articles; bare `moli` also matches 魔力 | listings: Moli project articles; `domcontentloaded`. |
| Startpage | `https://www.startpage.com/sp/search?query=Q` | Anubis challenge | anti-bot: Anubis verification page; challenge JavaScript did not load. |
| Toutiao | `https://so.toutiao.com/search?keyword=Q` | Readiness: `done`; empty/snssdk143 shell was retried with `domcontentloaded` | listings: relevant Moli articles; recheck required `domcontentloaded` after `done` followed an unsupported `snssdk143` URL. |
| Wiby | `https://wiby.me/?q=Q` | Readiness: `done`; Small-web index | no listings: no result rows for this query. |
| Yahoo JP | `https://search.yahoo.co.jp/search?p=Q` | Readiness: `done` | access: EEA/UK regional restriction page. |
| Yahoo TW | `https://tw.search.yahoo.com/search?p=Q` | Readiness: `done`; Traditional Chinese and English results, including `lexmount/moli` | consent: privacy/consent page. |
| Yahoo UK | `https://uk.search.yahoo.com/search?p=Q` | Readiness: `done`; English results, including `github.com/lexmount/moli` | listings: repository, releases, and documentation. |
| Yahoo US | `https://search.yahoo.com/search?p=Q` | Temporary-problem pages or irrelevant results; UK/TW were more useful | listings: Repository, releases, README, and documentation result rows. |
| Yandex.com | `https://yandex.com/search/?text=Q` | Readiness: `done`; Organic results after earlier SmartCaptcha failures; a second query ranked `lexmount/moli` | anti-bot: SmartCaptcha. |
| Yandex.ru | `https://yandex.ru/search/?text=Q` | Readiness: `done`; Alice answer block plus organic results; Russian coverage | anti-bot: SmartCaptcha. |
| You.com | `https://you.com/search?q=Q` | Login required | anti-bot: Cloudflare blocked page. |
