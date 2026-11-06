<p align="center">
  <picture>
    <source
      media="(prefers-color-scheme: dark)"
      srcset="../assets/moli-browser-banner-dark.jpg"
    />
    <source
      media="(prefers-color-scheme: light)"
      srcset="../assets/moli-browser-banner.jpg"
    />
    <img
      src="../assets/moli-browser-banner.jpg"
      alt="Moli Browser — 構造を優先し、画素は必要なときだけ。AI エージェント向けのオープンソースブラウザ。"
      width="1086"
    />
  </picture>
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="../README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <strong>日本語</strong> |
  <a href="README.de.md">Deutsch</a> |
  <a href="README.fr.md">Français</a> |
  <a href="README.es.md">Español</a> |
  <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <a href="https://discord.gg/4eXGtdWWnq">
    <img src="https://img.shields.io/badge/Discord-Join%20Community-5865F2" alt="Discord" />
  </a>
  <a href="../assets/moli-feedback-wechat-group.png">
    <img src="https://img.shields.io/badge/WeChat-Join%20Community-07C160" alt="WeChat" />
  </a>
</p>

<p align="center">
  <a href="https://www.producthunt.com/products/lexmount-browser?launch=moli-by-lexmount" target="_blank" rel="noopener noreferrer">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=moli-by-lexmount&amp;period=daily&amp;theme=dark" />
      <img src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=moli-by-lexmount&amp;period=daily&amp;theme=light" alt="Moli by Lexmount - #1 Product of the Day | Product Hunt" width="250" height="54" />
    </picture>
  </a>
  <a href="https://trendshift.io/repositories/128243?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-128243" target="_blank" rel="noopener noreferrer">
    <img src="https://trendshift.io/api/badge/trendshift/repositories/128243/daily" alt="lexmount%2Fmoli | Trendshift" width="250" height="55" />
  </a>
  <a href="https://trendshift.io/repositories/128243?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-128243" target="_blank" rel="noopener noreferrer">
    <img src="https://trendshift.io/api/badge/trendshift/repositories/128243/weekly?language=Rust" alt="lexmount%2Fmoli | Trendshift" width="250" height="55" />
  </a>
</p>

Moli は、AI エージェント向けのヘッドレスブラウザで、本番環境でも利用できます。必要なときだけレイアウトと描画を行うことで、ブラウザの実行環境を一通り備えながら、計算資源の使用量を抑えます。

Moli は、AI エージェントが行う Web ページの取得や情報抽出、Web 検索、ブラウザ操作の自動化を支援します。

コマンドライン（CLI）、CDP、WebDriver Classic、WebDriver BiDi から利用できます。

Moli は Linux、macOS、Windows に対応しています。

## すぐに試す

次の指示を AI エージェントに渡してください。

```text
https://github.com/lexmount/moli/tree/main/skills にあるスキルをインストールしてください。
各スキルの手順に従って、ビルド済みの最新 Moli 実行ファイルをダウンロードし、インストールしてください。
最後に、moli-webfetch で https://example.com を取得し、結果を表示してください。
```

### 直接インストール

Linux または macOS の場合：

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/lexmount/moli/releases/latest/download/moli-installer.sh | sh
```

Windows の場合は、PowerShell で次のコマンドを実行します：

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/lexmount/moli/releases/latest/download/moli-installer.ps1 | iex"
```

## 動作例

<p align="center">
  <a href="../assets/moli-game.jpg">
    <img
      src="../assets/moli-game.jpg"
      alt="Moli で描画した HTML5 ゲームを Chrome DevTools で確認"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Moli で描画した HTML5 ゲームを、Chrome DevTools でその場で確認できます。</sub>
</p>

<p align="center">
  <a href="../assets/moli-devtools-rust-lang.jpg">
    <img
      src="../assets/moli-devtools-rust-lang.jpg"
      alt="Moli で描画した rust-lang.org を Chrome DevTools で確認"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Moli で描画した rust-lang.org では、現在の DOM、CSS、位置・寸法を Chrome DevTools で確認できます。</sub>
</p>

## CLI の使い方

### ページの内容を取り出す

Moli の標準の完了判定を使い、ページを Markdown 形式で出力します。

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

または、ページの意味構造を、AI モデルが扱いやすい簡潔な形式で直接出力します。

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

画面出力を行う場合は、レイアウト計算を有効にすると、表示領域の PNG 画像、ページ全体の PNG 画像、またはページ分割した PDF を直接生成できます。

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump screenshot_full https://example.com > full-page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

`fetch --help` を実行すると、出力形式、ページ読み込みや応答の待機条件、プロファイル（利用者設定）、プロキシ設定、取得対象の設定、動作追跡など、指定できる引数をすべて確認できます。

### 自動化サーバーを起動する

```bash
# DOM を優先する処理向けの基本的な自動化サーバー
moli serve

# 実際の位置・寸法、座標入力、画面画像、連続画面配信（スクリーンキャスト）を有効化
moli serve --layout

# 通常は省略する画像、フォント、音声、動画、メディア、字幕などの文字情報も取得
moli serve --layout --resource
```

同じ接続先で、CDP、WebDriver Classic、WebDriver BiDi の 3 つの通信規格を利用できます。Playwright からは CDP で直接接続できます。

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const context = browser.contexts()[0];
const page = context.pages()[0] ?? await context.newPage();

await page.goto("https://example.com");
console.log(await page.locator("body").innerText());

await browser.close();
```

## Moli を選ぶ理由

AI エージェントの処理では、機能の充実度と速度に加え、少ない計算資源で動作することが重要です。Moli は、この 3 つを兼ね備えています。

- **高機能** — JavaScript、DOM、CSS、通信、保存領域、レイアウト、画面画像、標準的な自動操作方式を、一つのヘッドレスブラウザに統合しています。
- **高速** — ほとんどの自動化処理では画面描画が不要なため、構造を扱う操作ではレイアウト計算と描画を省きます。
- **少ない計算資源で動作** — レイアウトと画素は必要なときだけ生成します。描画済みの画面状態を常に保持し、更新し続ける必要はありません。

ほとんどのブラウザ自動化で必要なのは、常に描画された画面ではなく、ページの構造です。Moli は、ブラウザ本来の DOM とスタイル情報を唯一の正しい情報源とし、必要な操作に限ってレイアウト計算やソフトウェア描画を行います。

| AI エージェントが求める処理 | Moli の動作 |
| --- | --- |
| HTML や Markdown の抽出、DOM の照会、JavaScript の実行、通信や保存領域の確認 | ブラウザの実行状態を直接読み取ります。レイアウト計算も描画も行いません。 |
| 要素の位置・寸法の取得、座標判定、座標入力 | レイアウト計算を 1 回行い、最新の凍結レイアウト木だけを保持します。 |
| 画面画像の取得、連続画面配信の更新 | 現在の DOM とスタイルから再構築して凍結木を置き換え、新しいフレームを描画し、フレームは使用後すぐに破棄します。 |

<p align="center">
  <a href="../assets/moli_ondemand_rendering_flow.svg">
    <img
      src="../assets/moli_ondemand_rendering_flow.svg"
      alt="Moli の処理の流れ：標準では DOM を優先し、レイアウトと描画は必要なときだけ新たに構築"
      width="680"
    />
  </a>
</p>

Moli は、V8、CSS、レイアウト、文字組み、座標判定、ソフトウェア描画などの機能をすべて内蔵しています。一般的なブラウザとの違いは、画面処理を*いつ*実行し、その結果を*いつまで*保持するかだけです。この方式は、Web の巡回収集、ブラウザ操作を行う AI エージェント、検索・取得処理、評価環境、強化学習に特に適しています。

## 現在の対応機能

- **完全な Web 実行環境** — HTML の逐次解析、ブラウザ本来の DOM、V8 JavaScript、モジュール、タイマー、マイクロタスク、イベント、iframe、worker、CSS カスケード、Fetch、XHR、WebSocket、Cookie、WebCrypto、利用者設定ごとの保存領域（localStorage、IndexedDB、OPFS）を備えています。
- **情報抽出に適した出力** — CLI から HTML、Markdown、JSON、意味構造を表す文字列、フレーム情報付きのデータを直接出力できます。指定した要素、JavaScript の判定、応答を待つ機能や、通信内容の追跡にも対応します。
- **自動操作機能を一つに統合** — CDP、WebDriver Classic、WebDriver BiDi は、同じブラウザ中核と処理管理機構を共有します。ChromeDriver、geckodriver、ブラウザ本体を別途インストールする必要はありません。
- **必要なときに使える実際の画面処理** — `--layout` を追加すると、完全なボックス構築、Taffy による配置、Parley による文字組み、レイアウトに基づく座標判定と入力、表示領域の画面画像、CPU 描画による低頻度の DevTools 向け連続画面配信を利用できます。
- **運用に必要な設定** — プロファイル、Cookie、HTTP キャッシュ、プロキシ、取得するデータの種類、同時接続数、待ち時間、プライベートネットワークへの接続方針、User-Agent の変更、構造化ログ、通信診断を一通り備えています。

## Moli と Lexmount の関係

Moli は、Lexmount がオープンソースで公開しているヘッドレスブラウザです。Lexmount Browser は、Moli を中核として構築された、運用管理付きのクラウド実行環境と管理基盤です。

**Lexmount Browser を利用しなくても、このオープンソース版だけですべての機能を利用できます。**

## 処理負荷の制御

Moli では、処理負荷の高い機能は明示的に有効にした場合だけ動作します。標準では無効です。

| 動作方式または引数 | 動作 |
| --- | --- |
| 標準 | `LayoutPolicy::Mock` — 再現可能な位置・寸法情報を互換形式で返します。実際のレイアウト計算や描画は行いません。 |
| `--layout` | `LayoutPolicy::OnDemand` — 実際のレイアウト、位置・寸法、座標判定、座標入力、画面画像、連続画面配信を提供します。 |
| `--resource` | 通常は省略する画像・音声などのデータをすべて取得します。 |
| `--image`、`--font`、`--audio`、`--video`、`--media`、`--text-track` | 指定した種類のデータを取得します。 |
| `--profile-dir`、`--http-cache-dir`、`--cookie-file` | 必要な保存機能だけを選んで有効にします。 |

レイアウト結果は常に更新される状態ではなく、要求を受けた時点の状態です。最初に位置・寸法を要求されたときは、現在の DOM とスタイルから一時的な作業用レイアウト木を構築し、その確定済みジオメトリを DOM から独立した不変の `FrozenLayoutTree` に凍結して、最新の木だけを保持します。その後はページが変化していても通常の位置・寸法取得で古い木を再利用する場合があります。一方、画面画像と連続画面配信は毎回再構築して凍結木を置き換え、古い描画結果を再利用しません。

## 仕組み

Moli は、Chromium を外から操作するだけの仕組みではなく、独立したブラウザ実行基盤です。Rust で構築されており、独自の所有権管理と、構成要素の生成から破棄までの規則を持っています。主な構成技術は次のとおりです。

- `libcurl` — 通信と複数要求の同時処理
- `html5ever` — HTML 解析
- `rusty_v8` / V8 — JavaScript 実行
- Servo/Stylo — セレクター、カスケード、計算済みスタイル
- Taffy + Parley — ボックス配置と文字組み
- AnyRender/Vello CPU、`usvg`、Rust の画像関連ライブラリ — ソフトウェア描画

文書とスタイルの唯一の正しい情報源は、ブラウザ本来の DOM と Stylo の組み合わせです。更新のたびに一時的な作業木を構築し、必要なら新しい描画スナップショットを生成してその場で消費し、最終的なボックスと文字断片のジオメトリをコンパクトな `FrozenLayoutTree` に凍結します。その後、作業木、スタイル参照、レイアウトキャッシュ、診断情報、描画状態を破棄します。ソース対応表と hit-test 候補は問い合わせ時に凍結木から導出します。増分維持されるレイアウト木、描画差分の追跡情報、保持型の描画命令一覧、GPU による画面合成機構、常設のウィンドウは使用しません。

## 検証結果

### 公開テストセット

| 評価方法 | テストセット | 指標 |
| --- | --- | --- |
| Web 標準・回帰テスト | 選定した [WPT](https://github.com/web-platform-tests/wpt) テストと [Moli](https://github.com/lexmount/moli/blob/main/.github/workflows/ci.yml) 回帰テスト | 合格件数・不具合の再発 |
| ブラウザの自動操作 | [Lexbench](https://github.com/lexmount/Lexbench-Headless-Browser) · ブラウザ操作・Web 機能テスト 1,308 件 | 操作・機能テストの合格率 |
| CPU・メモリ使用量 | [Lexbench](https://github.com/lexmount/Lexbench-Headless-Browser/blob/main/docs/reports/resource-card-20260812.md) · CDP/Web の 557 タスク中、ローカル 4 ブラウザが完了した処理 | CPU 時間・ピークメモリ |
| Web Fetch | [lexbench-webfetch](https://github.com/lexmount/lexbench-webfetch) · 実サイト 430 タスク、[WebMainBench](https://github.com/lexmount/WebMainBench/tree/lex-main) · HTML スナップショット 545 件 | 実サイトでのタスク成功率、保存ページの抽出品質 |

### Web コンテンツの抽出

### AI エージェント処理の測定例

| 指標 | Moli | Chromium |
| --- | ---: | ---: |
| CDP が利用可能になるまで | 34.85 ms | 169.37 ms |
| 1 回の処理時間（p50） | 33.40 ms | 57.13 ms |
| PSS 最大値 | 102.46 MiB | 348.82 MiB |
| 最大プロセス数／最大スレッド数 | 1 / 24 | 11 / 123 |

### WPT テスト

Moli のエージェント向けブラウザとしての対応範囲を検証する現在の WPT テストセットでは、1 回の全件実行で **161万2,000件のテストに合格**しました。

### Lexbench-Headless-Browser における Moli の結果

[Lexbench-Headless-Browser](https://github.com/lexmount/Lexbench-Headless-Browser) の全タスクセットは 1,928 件で、生 CDP、Playwright・Puppeteer・Selenium などバージョンを固定した 13 種類の自動操作ツール、および Web プラットフォームのセマンティクスを対象とします。リモートエンドポイントとしてのみ提供される Kitesurf を含めるため、下のグラフではこのうち比較可能な 1,308 件を使用しています。すべてのブラウザに同じタスク選定ルールを適用しています。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-five-engine-caliber-b-dark.jpg">
  <img alt="5 種類のヘッドレスブラウザによる 1,308 件の比較可能なタスクの成功率：Chrome 99.8%、Moli 81.9%、Kitesurf 62.1%、Lightpanda 53.3%、Obscura 44.9%" src="../assets/lexbench-five-engine-caliber-b-light.jpg" width="100%">
</picture>

**Moli 0.1.1 は 1,071 件に合格し、成功率は 81.88% でした**。Kitesurf は 62.08%、Lightpanda は 53.29%、Obscura は 44.88%、参照エンジンの Chrome は 99.85% でした。Kitesurf は k=1 で実行され、未実行のタスクは不合格として数えられます。また、リモートサービスの再現条件はローカルバイナリとは異なります。全結果はベンチマークの[5 エンジンレポート](https://github.com/lexmount/Lexbench-Headless-Browser/blob/kitesurf-eval/docs/reports/five-engine-report-20260813.md)を参照してください。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-efficiency-map-dark.jpg">
  <img alt="4 つのローカルエンジンにおけるタスク成功率とタスクあたりピークメモリ中央値の関係：Chrome 99.9%・697 MiB、Moli 80.7%・92 MiB、Lightpanda 43.8%・34 MiB、Obscura 39.5%・39 MiB" src="../assets/lexbench-efficiency-map-light.jpg" width="100%">
</picture>

Kitesurf はリモートサービスのため、CPU、メモリ、プロセス数を測定できません。リソース比較は 4 つのローカルエンジンのみを対象とします。別に実施した 557 タスクの測定では、4 エンジンすべてが完了した処理だけを集計しています。Moli のタスクあたり中央値は **CPU 100.6 ms**、**ピークメモリ 92 MiB** でした。Chrome はそれぞれ **687 ms**、**697 MiB** でした。Moli の CPU 時間は Chrome の約 15%、ピークメモリは約 13% です。測定方法と全データはベンチマークの[リソースカード](https://github.com/lexmount/Lexbench-Headless-Browser/blob/main/docs/reports/resource-card-20260812.md)を参照してください。

### Web Fetch ベンチマーク

[`lexbench-webfetch`](https://github.com/lexmount/lexbench-webfetch) は実サイトに対して 430 タスクを実行するため、成功率にはサイトへのアクセスと正しい本文抽出の両方が含まれます。[WebMainBench](https://github.com/lexmount/WebMainBench/tree/lex-main) は保存済みの HTML 545 ページを使い、実サイトへのアクセスを除いた抽出品質だけを評価します。[ベンチマークの詳細](https://browser.lexmount.com/blog/webfetch-benchmark)。

| 方式 | 総合 | 静的ページ /100 | ボット対策 /100 | 人気サイト /100 | 動的サイト /100 | 文書・メディア /30 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Moli | **386/430 · 89.8%** | 97 | **88** | **91** | **90** | **20** |
| Chromium | 234/430 · 54.4% | **99** | 5 | 58 | 58 | 14 |
| Lightpanda | 228/430 · 53.0% | 92 | 37 | 38 | 44 | 17 |
| Obscura | 223/430 · 51.9% | 94 | 16 | 53 | 43 | 17 |
| Kitesurf | 192/430 · 44.7% | 90 | 10 | 40 | 41 | 11 |

各セルは合格タスク数です。最後の列は文書の内容、直接配信されるメディア、Web ページに埋め込まれたメディアをまとめたもので、表全体で 7 分類を網羅しています。

<details>
<summary><strong>WebMainBench · 保存済み 545 ページの抽出品質</strong></summary>

| 方式 | 本文を生成 | 総合 | 全文 | コード | 数式 | 表の内容 | 表の構造 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Moli | 545/545 | **39.27%** | 40.73% | **91.06%** | 28.20% | 22.62% | 13.75% |
| Lightpanda | 545/545 | 35.00% | 41.52% | 84.71% | 12.97% | 21.81% | **14.00%** |
| Obscura | 543/545 | 31.03% | **41.88%** | 81.82% | **31.39%** | 0.03% | 0.00% |
| Chromium | 545/545 | 27.02% | 39.40% | 35.86% | 21.12% | **25.01%** | 13.73% |
| Kitesurf | 524/545 | 33.43% | 36.99% | 80.70% | 19.32% | 18.25% | 11.87% |

</details>

## 対応範囲

文書に記載した AI エージェント向けの用途では、Moli はすでに本番環境で利用できる段階に達しています。現在も継続的に開発しています。

設計上、現在の対応範囲に含めていない機能は次のとおりです。

- GUI ブラウザ、常設のウィンドウ、GPU による画面合成は提供しません。複数の画面を保持し続ける描画方式も採用しません。
- Chrome と画素単位で同じ描画結果を目指すものではありません。高精度な Canvas、WebGL、音声・動画再生も提供しません。
- `--layout` では、ソフトウェア描画による画面画像と、画像方式の CDP PDF 生成に対応します。ただし、Chrome が備えるすべての画面取得・印刷方式には対応していません。

未対応の操作には、明確なエラーを返します。Moli は、実行していないブラウザ操作やイベント、通信状況の確認、画面出力を、実行済みであるかのように扱うことはありません。

## Star の推移

[lexmount/moli-metrics](https://github.com/lexmount/moli-metrics) が stargazer タイムラインから毎時自動生成しています。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/lexmount/moli-metrics/main/assets/star-history-dark.svg">
  <img alt="lexmount/moli の Star 推移" src="https://raw.githubusercontent.com/lexmount/moli-metrics/main/assets/star-history.svg" width="100%">
</picture>

## ライセンス

ファイルまたはディレクトリに別の記載がない限り、Moli は [Apache License 2.0](../LICENSE-APACHE) または [MIT License](../LICENSE-MIT) のいずれかを選択して利用できます。個別のライセンスが適用される第三者提供の構成要素やテスト用データについては、それぞれのライセンスと告知事項に従います。
