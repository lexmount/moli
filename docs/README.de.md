<p align="center">
  <img
    src="../assets/moli-browser-banner.jpg"
    alt="Moli Browser — Struktur zuerst. Pixel bei Bedarf. Open-Source-Browser für KI-Agenten."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="../README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <strong>Deutsch</strong> |
  <a href="README.fr.md">Français</a> |
  <a href="README.es.md">Español</a>
</p>

Moli ist ein produktionsreifer Headless-Browser für KI-Agenten. Durch sein bedarfsgesteuertes Layout- und Rendering-Design vereint er eine vollständige Browser-Laufzeitumgebung mit einem geringen Ressourcenbedarf.

Moli hilft deinem KI-Agenten dabei, Webseiten abzurufen und ihre Inhalte zu extrahieren, im Web zu recherchieren und Browser-Aufgaben zu automatisieren.

Du kannst Moli über die CLI, CDP, WebDriver Classic oder WebDriver BiDi ansteuern.

Moli unterstützt Linux, macOS und Windows.

## Schnellstart

Gib deinem KI-Agenten folgende Anweisung:

```text
Installiere die Skills unter https://github.com/lexmount/moli/tree/main/skills,
folge deren Anleitung zum Herunterladen und Installieren des neuesten
vorkompilierten Moli-Binaries, rufe anschließend mit moli-webfetch die Seite
https://example.com ab und zeig mir das Ergebnis.
```

### Direkte Installation

Unter Linux oder macOS:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/lexmount/moli/releases/latest/download/moli-installer.sh | sh
```

Unter Windows in PowerShell ausführen:

```powershell
irm https://github.com/lexmount/moli/releases/latest/download/moli-installer.ps1 | iex
```

## Demo

<p align="center">
  <a href="../assets/moli-game.jpg">
    <img
      src="../assets/moli-game.jpg"
      alt="Ein von Moli gerendertes und mit Chrome DevTools untersuchtes HTML5-Spiel"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Ein von Moli gerendertes HTML5-Spiel, live mit Chrome DevTools untersucht.</sub>
</p>

<p align="center">
  <a href="../assets/moli-devtools-rust-lang.jpg">
    <img
      src="../assets/moli-devtools-rust-lang.jpg"
      alt="Die von Moli gerenderte und mit Chrome DevTools untersuchte Website rust-lang.org"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Die von Moli gerenderte Website rust-lang.org — Live-DOM, CSS und Geometrie stehen direkt in Chrome DevTools zur Verfügung.</sub>
</p>

## CLI-Verwendung

### Eine Seite extrahieren

So renderst du eine Seite mit Molis Standard-Abschlussstrategie als Markdown:

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

Alternativ lässt du dir direkt einen kompakten, modellfreundlichen semantischen Baum ausgeben:

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

Für visuelle Ausgaben aktivierst du das On-Demand-Layout und erzeugst einen PNG-Screenshot des Viewports, einen PNG-Screenshot des vollständigen Dokuments oder ein mehrseitiges PDF:

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump screenshot_full https://example.com > full-page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

Die vollständige Liste aller Parameter — darunter Ausgabeformate, Wartebedingungen für Seiten- und Antwortladevorgänge, Profile, Proxy-Einstellungen, Ressourcenrichtlinien und Tracing-Optionen — zeigt dir `fetch --help`.

### Den Automatisierungsserver starten

```bash
# Einfacher Automatisierungsserver für DOM-orientierte Workloads
moli serve

# Echte Geometrie, Koordinateneingaben sowie Screenshot-/Screencast-Funktionen aktivieren
moli serve --layout

# Zusätzlich optionale Bild-, Schrift-, Audio-, Video-, Medien- und Textspur-Ressourcen laden
moli serve --layout --resource
```

Derselbe Endpunkt stellt alle drei Protokolle bereit: CDP, WebDriver Classic und WebDriver BiDi. Playwright kann sich direkt über CDP verbinden:

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const context = browser.contexts()[0];
const page = context.pages()[0] ?? await context.newPage();

await page.goto("https://example.com");
console.log(await page.locator("body").innerText());

await browser.close();
```

## Warum Moli?

Für Agenten-Workloads zählen vor allem drei Eigenschaften — und Moli vereint sie:

- **Vollständig** — echtes JavaScript, DOM, CSS, Netzwerk, Speicher, Layout, Screenshots und die gängigen Automatisierungsprotokolle, alles in einem einzigen Headless-Browser vereint.
- **Schnell** — die meisten Automatisierungsanfragen benötigen gar kein visuelles Rendering; strukturorientierte Vorgänge überspringen Layout und Zeichnen deshalb vollständig.
- **Ressourceneffizient** — Layout und Pixel entstehen nur bei Bedarf, wodurch Moli keinen vollständig gerenderten visuellen Zustand dauerhaft vorhalten und aktualisieren muss.

Die meisten Browser-Automatisierungen brauchen in Wirklichkeit die Struktur einer Seite, nicht eine fortlaufend gerenderte visuelle Welt. Moli behandelt natives DOM und Stilzustand als einzige verbindliche Datenquelle und löst Layout- oder Software-Rendering nur dort aus, wo eine Operation diese Berechnung tatsächlich braucht.

| Agentenanfrage | Verhalten von Moli |
| --- | --- |
| HTML/Markdown extrahieren, DOM abfragen, JS ausführen, Netzwerk/Speicher untersuchen | Liest den Zustand der Browser-Laufzeit direkt aus — löst weder Layout noch Zeichnen aus |
| Begrenzungsrahmen eines Elements lesen, Koordinaten testen, Koordinateneingaben senden | Führt eine Layoutberechnung aus und behält nur den neuesten eingefrorenen Layoutbaum |
| Screenshot aufnehmen oder Screencast aktualisieren | Baut aus dem aktuellen DOM/Stil neu auf, ersetzt den eingefrorenen Baum, rendert einen neuen Frame und verwirft ihn nach Gebrauch |

<p align="center">
  <a href="../assets/moli_ondemand_rendering_flow.svg">
    <img
      src="../assets/moli_ondemand_rendering_flow.svg"
      alt="So verarbeitet Moli eine Anfrage: standardmäßig DOM-orientiert; Layout und Zeichnung werden nur bei Bedarf neu aufgebaut"
      width="680"
    />
  </a>
</p>

Dabei bringt Moli weiterhin den vollen Funktionsumfang mit — V8, CSS, Layout, Textsatz, Hit-Testing, Software-Rendering und mehr. Der einzige Unterschied ist, *wann* diese visuelle Arbeit anfällt und *wie lange* ihr Ergebnis aufbewahrt wird. Dieses Kostenmodell passt besonders gut zu Crawling, Browser-Agenten, Retrieval-Pipelines, Evaluierungsumgebungen und Reinforcement-Learning-Workloads.

## Derzeit unterstützte Funktionen

- **Vollständige Web-Laufzeit** — Streaming-HTML-Parsing, natives DOM, V8 JavaScript, Module/Timer/Microtasks/Events, iframes und Worker, CSS-Kaskade, Fetch/XHR/WebSocket, Cookies, WebCrypto und profilspezifischer Speicher (localStorage, IndexedDB, OPFS).
- **Für Extraktion optimierte Ausgaben** — die CLI liefert HTML, Markdown, JSON, semantische Textbäume und framebewusste Serialisierung direkt als Ausgabe und unterstützt das Warten auf Selektoren, Skripte oder Antworten sowie Netzwerk-Tracing.
- **Ein einheitlicher Automatisierungs-Stack** — CDP, WebDriver Classic und WebDriver BiDi laufen über denselben Kernel und Scheduler. Eine separate Installation von ChromeDriver, geckodriver oder einem eigenen Browser ist nicht nötig.
- **Echte visuelle Funktionen bei Bedarf** — `--layout` aktiviert die vollständige Box-Konstruktion, Taffy-Layout, Parley-Textsatz, layoutgestützte Hit-Tests und Eingaben, Viewport-Screenshots sowie niedrigfrequente, CPU-gerenderte DevTools-Screencasts.
- **Fein steuerbare Betriebsoptionen** — Profile, Cookies, HTTP-Cache, Proxys, Ressourcengruppen, Verbindungslimits, Zeitüberschreitungen, Richtlinien für private Netzwerke, User-Agent-Überschreibungen, strukturierte Protokollierung und Netzwerkdiagnose stehen vollständig zur Verfügung.

## Die Beziehung zwischen Moli und Lexmount

Moli ist der quelloffene Headless-Browser von Lexmount. Lexmount Browser ist die verwaltete Cloud-Laufzeitumgebung und Steuerungsebene, die darauf aufbaut.

**Der quelloffene Headless-Browser lässt sich eigenständig nutzen und ist nicht von Lexmount Browser abhängig.**

## Kostensteuerung

Rechenintensive Browser-Operationen sind in Moli standardmäßig deaktiviert und müssen ausdrücklich aktiviert werden:

| Modus oder Option | Verhalten |
| --- | --- |
| Standard | `LayoutPolicy::Mock` — deterministische, formatkompatible Geometrie, kein echtes Layout und kein Zeichnen |
| `--layout` | `LayoutPolicy::OnDemand` — echtes Layout, Geometrie, Hit-Testing, Koordinateneingaben, Screenshots und Screencast |
| `--resource` | Alle optionalen visuellen und Medienressourcengruppen laden |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Eine bestimmte optionale Ressourcengruppe einzeln aktivieren |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Persistenz gezielt aktivieren, je nach Bedarf des Workloads |

Das Layoutergebnis ist ein bei Bedarf erzeugter Snapshot, kein dauerhaft gepflegter Zustand: Die erste Geometrieanfrage (Kaltstart) baut aus dem aktuellen DOM/Stil einen temporären Arbeitsbaum auf und friert dessen kanonische Geometrie in einen unveränderlichen, vom DOM unabhängigen `FrozenLayoutTree` ein — nur dieser jeweils neueste Baum wird vorgehalten. Normale Geometrieabfragen können ihn auch nach Seitenänderungen weiterverwenden; Screenshots und Screencasts dagegen bauen jedes Mal neu auf, ersetzen den eingefrorenen Baum und greifen nie auf alte Zeichenergebnisse zurück.

## Architektur

Moli ist ein eigenständiger Browser-Kernel, kein Chromium-Wrapper. Er ist in Rust geschrieben, folgt eigenen Ownership- und Lifecycle-Regeln und stützt sich auf folgende zentrale Abhängigkeiten:

- `libcurl` — Netzwerktransport und Laufzeit für parallele Anfragen
- `html5ever` — HTML-Parsing
- `rusty_v8` / V8 — JavaScript-Ausführung
- Servo/Stylo — Selektoren, Kaskade und berechnete Stile
- Taffy + Parley — Box- und Textlayout
- AnyRender/Vello CPU, `usvg` und das Rust-Bildökosystem — Software-Rendering

Dokument und Stil haben genau eine verbindliche Datenquelle: die Integration aus nativem DOM und Stylo. Jede echte Aktualisierung baut daraus einen temporären Arbeitsbaum auf, erzeugt und verbraucht bei Bedarf einen frischen Paint-Snapshot und friert die endgültige Box- und Fragmentgeometrie in einen kompakten `FrozenLayoutTree` ein. Anschließend verwirft sie Arbeitsbaum, Stilreferenzen, Layout-Caches, Diagnosedaten und Paint-Zustand wieder. Quellzuordnung und Hit-Test-Kandidaten werden bei Abfragen jeweils aus dem eingefrorenen Baum abgeleitet. Das System kennt weder einen inkrementell gepflegten Layoutbaum noch einen Damage-Graph, keine beibehaltene Displayliste, keinen GPU-Compositor und kein persistentes Fenster.

## Benchmarks

Die folgenden Messwerte zeigen Molis derzeitigen Funktionsumfang. Sie decken reale Websites, Automatisierungsclients, Prüfungen des Chromium-/WPT-Verhaltens und eine große nextest-Regressionssuite ab.

### Gemischter Crawling-Test im öffentlichen Web

Getestet wurden 192 öffentliche URLs großer chinesischer und internationaler Websites. Als Erfolg zählte ausschließlich eine Seite, die nach Ausführung von JavaScript inhaltlich verwertbare Ergebnisse lieferte — ein bloßer HTTP-200-Status, eine Verifizierungsseite, eine Anmeldesperre, eine leere Antwort oder eine reine App-Hülle galten nicht als Erfolg.

| Browser | Verwertbare Seiten | Erfolgsquote | Medianzeit | Median-RSS |
| --- | ---: | ---: | ---: | ---: |
| **Moli** | **103** | **53.6%** | **1.43 s** | **73 MiB** |
| Chrome Headless | 101 | 52.6% | 1.43 s | 773 MiB |
| Lightpanda | 85 | 44.3% | 0.97 s | 40 MiB |
| Obscura | 57 | 29.7% | 1.30 s | 39 MiB |

### Beispiel eines Agenten-Workloads

| Metrik | Moli | Chromium |
| --- | ---: | ---: |
| CDP bereit | 34.85 ms | 169.37 ms |
| Aktive Episodendauer p50 | 33.40 ms | 57.13 ms |
| PSS-Spitzenwert | 102.46 MiB | 348.82 MiB |
| Maximale Prozesse / Threads | 1 / 24 | 11 / 123 |

### WPT-Tests

In der aktuellen WPT-Auswahl zur Überprüfung von Molis Funktionsumfang als Agenten-Browser wurden bei einem vollständigen Lauf **1,612 Millionen Tests bestanden**.

### Molis Ergebnisse in Lexbench-Headless-Browser

Der vollständige Aufgabenbestand von [Lexbench-Headless-Browser](https://github.com/lexmount/Lexbench-Headless-Browser) umfasst 1.928 Aufgaben zu rohem CDP, 13 fest versionierten Automatisierungswerkzeugen wie Playwright, Puppeteer und Selenium sowie zur Semantik der Web-Plattform. Da Kitesurf nur als entfernter Endpunkt verfügbar ist, verwendet die folgende Grafik 1.308 vergleichbare Aufgaben. Für alle Browser gelten dieselben Regeln zur Aufgabenauswahl.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-five-engine-caliber-b-dark.jpg">
  <img alt="Erfolgsquote von fünf Headless-Browsern über 1.308 vergleichbare Aufgaben: Chrome 99,8 %, Moli 81,9 %, Kitesurf 62,1 %, Lightpanda 53,3 %, Obscura 44,9 %" src="../assets/lexbench-five-engine-caliber-b-light.jpg" width="100%">
</picture>

**Moli 0.1.1 bestand 1.071 Aufgaben und erreichte 81,88 %**. Kitesurf erreichte 62,08 %, Lightpanda 53,29 % und Obscura 44,88 %; Chrome kam als Referenz-Engine auf 99,85 %. Kitesurf lief mit k=1, nicht abgedeckte Aufgaben zählen als nicht bestanden, und die Reproduktionsbedingungen eines entfernten Dienstes unterscheiden sich von denen lokaler Binärdateien. Die vollständigen Ergebnisse stehen im [Fünf-Engine-Bericht](https://github.com/lexmount/Lexbench-Headless-Browser/blob/kitesurf-eval/docs/reports/five-engine-report-20260813.md) des Benchmarks.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-efficiency-map-dark.jpg">
  <img alt="Aufgaben-Erfolgsquote gegen den Median des Spitzenspeichers pro Aufgabe für die vier lokalen Engines: Chrome bei 99,9 % und 697 MiB, Moli bei 80,7 % und 92 MiB, Lightpanda bei 43,8 % und 34 MiB, Obscura bei 39,5 % und 39 MiB" src="../assets/lexbench-efficiency-map-light.jpg" width="100%">
</picture>

Kitesurf ist ein entfernter Dienst, dessen CPU, Arbeitsspeicher und Prozessanzahl nicht messbar sind. Der Ressourcenvergleich umfasst daher nur die vier lokalen Engines. Ein separater Lauf mit 557 Aufgaben berücksichtigt nur Arbeiten, die alle vier abgeschlossen haben. Molis Median pro Aufgabe lag bei **100,6 ms CPU-Zeit** und **92 MiB Spitzenspeicher**; Chrome erreichte **687 ms** beziehungsweise **697 MiB**. Damit lag Moli bei rund 15 % der CPU-Zeit und 13 % des Spitzenspeichers von Chrome. Methodik und vollständige Daten stehen in der [Ressourcenübersicht](https://github.com/lexmount/Lexbench-Headless-Browser/blob/main/docs/reports/resource-card-20260812.md) des Benchmarks.

## Projektumfang

Für die in der Dokumentation beschriebenen Agenten-Browser-Szenarien ist Moli bereits produktionsreif und wird laufend weiterentwickelt.

Zu den aktuell bewusst gesetzten Grenzen gehören:

- Kein GUI-Browser: Es gibt weder ein persistentes Fenster noch einen GPU-Compositor oder eine über mehrere Frames hinweg beibehaltene Zeichenarchitektur.
- Moli strebt keine pixelgenaue Übereinstimmung mit Chrome an und bietet keine originalgetreue Canvas-/WebGL-/Medienwiedergabe.
- Im `--layout`-Modus werden Software-Screenshots und rasterbasierte CDP-PDF-Erzeugung unterstützt, aber nicht sämtliche Screenshot- oder Druckmodi von Chrome.

Nicht unterstützte Protokollpfade liefern einen eindeutigen Fehler zurück — Moli täuscht nie vor, dass eine Browseraktion, ein Ereignis, eine Netzwerkbeobachtung oder ein visuelles Ergebnis stattgefunden hätte.

## Star-Verlauf

Stündlich aus der Stargazer-Timeline erzeugt von [lexmount/moli-metrics](https://github.com/lexmount/moli-metrics).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/lexmount/moli-metrics/main/assets/star-history-dark.svg">
  <img alt="Star-Verlauf von lexmount/moli" src="https://raw.githubusercontent.com/lexmount/moli-metrics/main/assets/star-history.svg" width="100%">
</picture>

## Lizenz

Sofern eine Datei oder ein Verzeichnis nichts anderes angibt, kann Moli wahlweise unter der [Apache License 2.0](../LICENSE-APACHE) oder der [MIT License](../LICENSE-MIT) genutzt werden. Separat lizenzierte Komponenten und Fixtures von Drittanbietern unterliegen weiterhin ihren jeweiligen Lizenzen und Hinweisen.
