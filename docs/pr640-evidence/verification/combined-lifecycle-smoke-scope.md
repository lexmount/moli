# Combined lifecycle smoke

The seven-partition combined binary `ed844e7cf8c2f1f7d79a6215936c6055b1eb3b65`, tree `6fe6dd8f43edca8585379cd23b957057d902f0f1`, passed all three localhost HTTP/public-CDP scenarios on its first functional run. Binary SHA-256: `a5d78fef756d7ad81f01c889c1deff12c77c1cc7acb6c12ef0717234ce40473f`.

1. Completed text/plain: DOM normalizes CR while preserving literal markup inside one PRE; script does not execute. Network.getResponseBody preserves all 79 transmitted bytes independently of DOM normalization.
2. Explicit stop: streaming text is visible before completion, remains after Page.stopLoading, and its server connection closes before tail release. A successor document loads successfully; late old bytes do not replace it.
3. Ordinary parser retirement: document.open/write/close replaces a streaming HTML document without cancelling the held transfer. Network.loadingFinished and complete 143-byte response capture remain available, while the replacement DOM/title survive the released tail.

This is integration evidence for the combined binary, not individual-PR acceptance or an application/WebArena result. It covers main-document public behavior; it does not claim child-frame cancellation coverage or observation of internal owner states. No frozen runtime, scenario actions, application site, browser source or scoring was modified. The runner initially encountered sandbox EPERM binding localhost before any scenario; one permitted localhost execution then passed. The original functional result and script remain unchanged.
