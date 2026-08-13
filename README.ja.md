<p align="center">
  <img
    src="assets/moli-browser-banner.jpg"
    alt="Moli Browser — 構造を優先し、ピクセルは必要なときだけ。AIエージェント向けのオープンソースブラウザ。"
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <strong>日本語</strong> |
  <a href="README.de.md">Deutsch</a> |
  <a href="README.fr.md">Français</a> |
  <a href="README.es.md">Español</a>
</p>

Moli は AI エージェント向けのヘッドレスブラウザです。「オンデマンドレンダリング」という設計思想を採用し、すでに本番環境で利用できる水準に達しています。

標準で実際の JavaScript を実行し、実際の DOM を維持し、実際のブラウザ API を提供しますが、レイアウトの計算やピクセルのレンダリングは本当に必要なときにだけ行います。

CLI、CDP、WebDriver Classic、または WebDriver BiDi から利用できます。

## デモ

<p align="center">
  <a href="assets/moli-game.jpg">
    <img
      src="assets/moli-game.jpg"
      alt="Moli でレンダリングし、Chrome DevTools で検証している HTML5 ゲーム"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Moli でレンダリングし、Chrome DevTools からリアルタイムに検証している HTML5 ゲーム。</sub>
</p>

<p align="center">
  <a href="assets/moli-devtools-rust-lang.jpg">
    <img
      src="assets/moli-devtools-rust-lang.jpg"
      alt="Moli でレンダリングし、Chrome DevTools で検証している rust-lang.org"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Moli でレンダリングした rust-lang.org。ライブの DOM、CSS、ジオメトリを Chrome DevTools から確認できます。</sub>
</p>

## クイックスタート

次の文を AI コーディングエージェントに渡してください。

> `https://github.com/lexmount/moli/tree/main/skills` 以下の skills をインストールし、その指示に従って最新のビルド済み Moli バイナリをダウンロードしてインストールしたうえで、`moli-webfetch` を使って `https://example.com` を取得し、結果を見せてください。

## CLI の使い方

### ページを抽出する

Moli の標準の完了判定を使い、ページを Markdown としてレンダリングします。

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

または、構造がコンパクトでモデルが扱いやすいセマンティックツリーを直接返します。

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

`fetch --help` を実行すると、出力形式、ページ読み込み／レスポンスの待機条件、プロファイル、プロキシ設定、リソースポリシー、トレースオプションを含む完全なパラメーター一覧を確認できます。

### 自動化サーバーを起動する

```bash
# DOM 優先のワークロード向け基本自動化サーバー
moli serve

# 実際のジオメトリ、座標入力、スクリーンショット／スクリーンキャスト機能を有効化
moli serve --layout

# オプションの画像、フォント、音声、動画、メディア、テキストトラックの各リソースも取得
moli serve --layout --resource
```

同じエンドポイントが CDP、WebDriver Classic、WebDriver BiDi の 3 つのプロトコルをすべて提供します。Playwright は CDP 経由で直接接続できます。

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

エージェントのワークロードで最も重要なのは 3 つの特長であり、Moli はそのすべてを兼ね備えています。

- **フル機能** — 実際の JavaScript、DOM、CSS、ネットワーク、ストレージ、レイアウト、スクリーンショット、標準の自動化プロトコルを、すべて 1 つのヘッドレスブラウザに統合しています。
- **高速** — ほとんどの自動化リクエストには視覚的なレンダリングが不要なため、構造優先の操作ではレイアウトと描画を完全に省略します。
- **高いリソース効率** — レイアウトとピクセルは必要なときだけ生成されるため、Moli はレンダリング済みの視覚状態一式を継続的に維持、更新する必要がありません。

ほとんどのブラウザ自動化タスクで本当に必要なのは、継続的にレンダリングされた視覚世界ではなくページ構造です。Moli はネイティブ DOM とスタイル状態を唯一の信頼できる情報源とし、レイアウトやソフトウェア描画が本当に必要な操作だけで対応する計算を実行します。

| エージェントのリクエスト | Moli の処理 |
| --- | --- |
| HTML/Markdown の抽出、DOM の照会、JS の実行、ネットワーク／ストレージの検証 | ブラウザランタイムの状態を直接読み取り — レイアウトも描画も発生しない |
| 要素の境界ボックスの読み取り、座標のヒットテスト、座標入力の送信 | レイアウト計算を 1 回実行し、最新のジオメトリスナップショットだけを保持 |
| スクリーンショットの撮影、スクリーンキャストの更新 | 現在の DOM／スタイルから再構築し、新しいフレームをレンダリングして使用後すぐに破棄 |

<p align="center">
  <a href="assets/moli_ondemand_rendering_flow.svg">
    <img
      src="assets/moli_ondemand_rendering_flow.svg"
      alt="Moli のリクエスト処理：標準では DOM を優先し、レイアウトと描画は必要なときだけ新たに構築"
      width="680"
    />
  </a>
</p>

Moli は V8、CSS、レイアウト、テキスト組版、ヒットテスト、ソフトウェア描画などの完全な機能を引き続き内蔵しています。違いは、視覚処理を*いつ*実行し、その結果を*どのくらいの期間*保持するかだけです。このコストモデルは、クローリング、ブラウザ操作エージェント、検索パイプライン、評価環境、強化学習ワークロードに特に適しています。

## 現在サポートしている機能

- **完全な Web ランタイム** — ストリーミング HTML パース、ネイティブ DOM、V8 JavaScript、モジュール／タイマー／マイクロタスク／イベント、iframe と worker、CSS カスケード、Fetch/XHR/WebSocket、Cookie、WebCrypto、プロファイル単位のストレージ（localStorage、IndexedDB、OPFS）。
- **抽出向けに最適化された出力** — CLI から HTML、Markdown、JSON、セマンティックテキストツリー、フレーム情報を含むシリアライズ結果を直接出力でき、セレクター／スクリプト／レスポンス待機とネットワークトレースにも対応します。
- **統合された自動化プログラム** — CDP、WebDriver Classic、WebDriver BiDi は同じカーネルとスケジューラーを共有します。ChromeDriver、geckodriver、ブラウザ本体を別途インストールする必要はありません。
- **実際の視覚機能をオンデマンドで有効化** — `--layout` を追加すると、完全なボックス構築、Taffy レイアウト、Parley テキスト組版、レイアウトに基づくヒットテスト／入力、ビューポートのスクリーンショット、CPU で低頻度にレンダリングする DevTools スクリーンキャストを利用できます。
- **制御可能な運用オプション** — プロファイル、Cookie、HTTP キャッシュ、プロキシ、リソース種別、接続数制限、タイムアウト、プライベートネットワークポリシー、User-Agent の上書き、構造化ログ、ネットワーク診断を一通り備えています。

## Moli と Lexmount の関係

Moli は Lexmount のオープンソース・ヘッドレスブラウザであり、Lexmount Browser はそれを中心に構築されたマネージドクラウドランタイムおよびコントロールプレーンです。

**Lexmount Browser に依存せず、このオープンソース・ヘッドレスブラウザ単体ですべての機能を利用できます。**

## コスト制御

Moli では、負荷の高いブラウザ処理は明示的に有効化する必要があり、標準ではオンになりません。

| モードまたはオプション | 動作 |
| --- | --- |
| 標準 | `LayoutPolicy::Mock` — 決定論的で形式互換のジオメトリを返し、実際のレイアウトや描画は行わない |
| `--layout` | `LayoutPolicy::OnDemand` — 実際のレイアウト、ジオメトリ、ヒットテスト、座標入力、スクリーンショット、スクリーンキャストを提供 |
| `--resource` | オプションの視覚／メディアリソースを全種取得 |
| `--image`、`--font`、`--audio`、`--video`、`--media`、`--text-track` | 指定した種類のオプションリソースを有効化 |
| `--profile-dir`、`--http-cache-dir`、`--cookie-file` | ワークロードの必要に応じて永続化機能を選択的に有効化 |

レイアウト結果は継続的に維持される状態ではなく、オンデマンドで取得するスナップショットです。最初のジオメトリ要求（コールドスタート）では現在の DOM／スタイルから完全なレイアウトを 1 回構築し、最新の `LayoutPassOutput` だけを保持します。その後はページが変化していても、通常のジオメトリ読み取りがこのスナップショットを再利用する場合があります。一方、スクリーンショットとスクリーンキャストは毎回再構築し、古い結果を再利用しません。

## アーキテクチャ

Moli は Chromium のラッパーではなく、独立したブラウザカーネルです。Rust を基盤に構築され、独自の所有権とライフサイクル規則を持っています。主な依存技術は次のとおりです。

- `libcurl` — ネットワーク転送と複数リクエストのランタイム
- `html5ever` — HTML パース
- `rusty_v8` / V8 — JavaScript 実行
- Servo/Stylo — セレクター、カスケード、計算済みスタイル
- Taffy + Parley — ボックスとテキストのレイアウト
- AnyRender/Vello CPU、`usvg`、Rust の画像エコシステム — ソフトウェアレンダリング

ドキュメントとスタイルには、ネイティブ DOM と Stylo の統合という唯一の信頼できる情報源があります。実際の更新のたびに、この情報源からレイアウトを再構築し、その結果を DOM に依存しない不変データへ変換した後、そのレイアウトと描画で生じた一時状態を破棄します。システム全体に、増分レイアウトツリー、ダメージグラフ、保持型ディスプレイリスト、GPU コンポジター、永続ウィンドウはありません。

## テストデータ

以下の 2 組の実測データは、Moli の現在の能力範囲を示しています。テストは実際の Web サイト、実際の自動化クライアント、対象を絞った Chromium/WPT の挙動検証、大規模な nextest 回帰テストスイートを対象としています。

### 公開 Web の混合クロールテスト

中国国内および世界の主要サイトから 192 件の公開 URL を対象としました。成功の条件は、JavaScript 実行後に実質的な内容が生成されることです。HTTP 200 が返るだけの場合、検証用のチャレンジページ、ログインウォール、空のレスポンス、外枠だけのアプリ画面は成功に数えません。

| ブラウザ | 有用なページ | 成功率 | 中央値の時間 | RSS 中央値 |
| --- | ---: | ---: | ---: | ---: |
| **Moli** | **103** | **53.6%** | **1.43 s** | **73 MiB** |
| Chrome Headless | 101 | 52.6% | 1.43 s | 773 MiB |
| Lightpanda | 85 | 44.3% | 0.97 s | 40 MiB |
| Obscura | 57 | 29.7% | 1.30 s | 39 MiB |

### エージェントワークロードのサンプル

| 指標 | Moli | Chromium |
| --- | ---: | ---: |
| CDP 準備完了 | 34.85 ms | 169.37 ms |
| エピソード稼働時間 p50 | 33.40 ms | 57.13 ms |
| ピーク PSS | 102.46 MiB | 348.82 MiB |
| 最大プロセス数／スレッド数 | 1 / 24 | 11 / 123 |

現在 Moli のエージェントブラウザ機能の範囲を検証する WPT テスト群では、1 回の完全なテスト実行で **161万2,000 件のテスト成功**を記録しました。

## プロジェクトの対象範囲

ドキュメントで定義されたエージェントブラウザの利用場面において、Moli はすでに本番環境で利用できる水準に達しており、現在も継続的に開発されています。

現在、意図的に残している境界は次のとおりです。

- GUI ブラウザ、永続ウィンドウ、GPU コンポジターは提供せず、保持型のマルチフレーム描画アーキテクチャも実装しません。
- Chrome とピクセル単位で一致するレンダリングは追求せず、高忠実度の Canvas/WebGL／メディア再生も提供しません。
- CDP、WebDriver Classic、WebDriver BiDi の一部機能だけを対象とし、プロトコル全体との互換性は実装しません。
- `--layout` モードではソフトウェアスクリーンショットとラスター方式の CDP PDF 生成に対応しますが、Chrome のすべてのスクリーンショット／印刷モードを実装しているわけではありません。
- リソースの読み込み、ジオメトリの鮮度、視覚レンダリングのコストは、継続的に標準で有効になるものではなく、常に明示的なポリシーとして設定します。

未対応のプロトコル経路では明確なエラーを返します。Moli がブラウザ操作、イベント、ネットワーク観測、視覚結果を実行済みと装うことはありません。

メンテナーは[リリースガイド](RELEASING.md)に従い、GitHub Actions からタグ付きバイナリリリースを公開できます。

## ライセンス

ファイルまたはディレクトリに別の記載がない限り、Moli は [Apache License 2.0](LICENSE-APACHE) または [MIT License](LICENSE-MIT) のいずれかを選択して利用できます。個別のライセンスが適用されるサードパーティ製コンポーネントとフィクスチャは、引き続きそれぞれのライセンスと告知に従います。
