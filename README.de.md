<p align="center">
  <img
    src="assets/moli-browser-banner.jpg"
    alt="Moli Browser — Struktur zuerst. Pixel bei Bedarf. Open-Source-Browser für KI-Agenten."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <strong>Deutsch</strong> |
  <a href="README.fr.md">Français</a> |
  <a href="README.es.md">Español</a>
</p>

Moli ist ein produktionsreifer Headless-Browser für KI-Agenten. Sein Design mit Layout und Rendering nach Bedarf verbindet eine vollständige Browser-Laufzeit mit einem geringen Ressourcenbedarf.

Sie führt standardmäßig echtes JavaScript aus, verwaltet ein echtes DOM und stellt echte Browser-APIs bereit. Layoutberechnungen oder das Rendern von Pixeln erfolgen jedoch nur, wenn sie tatsächlich benötigt werden.

Moli kann über die CLI, CDP, WebDriver Classic oder WebDriver BiDi genutzt werden.

## Schnellstart

Gib deinem KI-Coding-Agenten diese Anweisung:

```text
Installiere die skills unter https://github.com/lexmount/moli/tree/main/skills, folge ihren Anweisungen zum Herunterladen und Installieren des neuesten vorkompilierten Moli-Binarys, rufe anschließend mit moli-webfetch die Seite https://example.com ab und zeige mir das Ergebnis.
```

## Demo

<p align="center">
  <a href="assets/moli-game.jpg">
    <img
      src="assets/moli-game.jpg"
      alt="Ein von Moli gerendertes und mit Chrome DevTools untersuchtes HTML5-Spiel"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Ein von Moli gerendertes und live mit Chrome DevTools untersuchtes HTML5-Spiel.</sub>
</p>

<p align="center">
  <a href="assets/moli-devtools-rust-lang.jpg">
    <img
      src="assets/moli-devtools-rust-lang.jpg"
      alt="Die von Moli gerenderte und mit Chrome DevTools untersuchte Website rust-lang.org"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Die von Moli gerenderte Website rust-lang.org, deren Live-DOM, CSS und Geometrie in Chrome DevTools verfügbar sind.</sub>
</p>

## CLI-Verwendung

### Eine Seite extrahieren

Die Seite mit Molis standardmäßiger Abschlussstrategie als Markdown rendern:

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

Alternativ direkt einen kompakten, modellfreundlichen semantischen Baum zurückgeben:

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

Für eine visuelle Ausgabe kann das On-Demand-Layout aktiviert und entweder ein PNG-Screenshot des Viewports oder ein mehrseitiges PDF direkt erzeugt werden:

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

`fetch --help` zeigt die vollständige Parameterliste, darunter Ausgabeformate, Wartebedingungen für das Laden von Seiten und für Antworten, Profile, Proxy-Einstellungen, Ressourcenrichtlinien und Tracing-Optionen.

### Den Automatisierungsserver starten

```bash
# Einfacher Automatisierungsserver für DOM-orientierte Workloads
moli serve

# Echte Geometrie, Koordinateneingaben sowie Screenshot-/Screencast-Funktionen aktivieren
moli serve --layout

# Zusätzlich optionale Bild-, Schrift-, Audio-, Video-, Medien- und Textspurressourcen abrufen
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

Für Agenten-Workloads sind drei Eigenschaften besonders wichtig, und Moli vereint sie:

- **Vollständig** — echtes JavaScript, DOM, CSS, Netzwerk, Speicher, Layout, Screenshots und Standard-Automatisierungsprotokolle, alles in einem einzigen Headless-Browser integriert.
- **Schnell** — die meisten Automatisierungsanfragen benötigen überhaupt kein visuelles Rendering; strukturorientierte Vorgänge überspringen Layout und Zeichnen deshalb vollständig.
- **Ressourceneffizient** — Layout und Pixel entstehen nur bei Bedarf. Moli muss daher keinen vollständig gerenderten visuellen Zustand dauerhaft pflegen und aktualisieren.

Was die meisten Browserautomatisierungen wirklich benötigen, ist die Seitenstruktur und keine kontinuierlich gerenderte visuelle Welt. Moli behandelt das native DOM und den Stilzustand als einzige maßgebliche Datenquelle und löst Layout oder Software-Zeichnung nur bei Vorgängen aus, die diese Berechnungen tatsächlich benötigen.

| Agentenanfrage | Verhalten von Moli |
| --- | --- |
| HTML/Markdown extrahieren, DOM abfragen, JS ausführen, Netzwerk/Speicher untersuchen | Liest den Zustand der Browser-Laufzeit direkt aus — löst weder Layout noch Zeichnen aus |
| Begrenzungsrahmen eines Elements lesen, Koordinaten testen, Koordinateneingaben senden | Führt eine Layoutberechnung aus und behält nur den neuesten Geometrie-Snapshot |
| Screenshot aufnehmen oder Screencast aktualisieren | Baut aus dem aktuellen DOM/Stil neu auf, rendert einen neuen Frame und verwirft ihn nach Gebrauch |

<p align="center">
  <a href="assets/moli_ondemand_rendering_flow.svg">
    <img
      src="assets/moli_ondemand_rendering_flow.svg"
      alt="So verarbeitet Moli eine Anfrage: standardmäßig DOM-orientiert; Layout und Zeichnung werden nur bei Bedarf neu aufgebaut"
      width="680"
    />
  </a>
</p>

Moli enthält weiterhin den vollständigen Funktionsumfang mit V8, CSS, Layout, Textsatz, Treffertests, Software-Zeichnung und mehr. Der einzige Unterschied liegt darin, *wann* visuelle Arbeit ausgeführt und *wie lange* ihr Ergebnis vorgehalten wird. Dieses Kostenmodell eignet sich besonders für Crawling, Browser-Agenten, Retrieval-Pipelines, Evaluierungsumgebungen und Reinforcement-Learning-Workloads.

## Derzeit unterstützte Funktionen

- **Vollständige Web-Laufzeit** — Streaming-HTML-Parsing, natives DOM, V8 JavaScript, Module/Timer/Microtasks/Events, iframes und Worker, CSS-Kaskade, Fetch/XHR/WebSocket, Cookies, WebCrypto und profilspezifischer Speicher (localStorage, IndexedDB, OPFS).
- **Für Extraktion optimierte Ausgaben** — die CLI gibt HTML, Markdown, JSON, semantische Textbäume und framebewusste Serialisierung direkt aus und unterstützt das Warten auf Selektoren/Skripte/Antworten sowie Netzwerk-Tracing.
- **Einheitliches Automatisierungsprogramm** — CDP, WebDriver Classic und WebDriver BiDi verwenden denselben Kernel und Scheduler. Eine separate Installation von ChromeDriver, geckodriver oder einem Browser ist nicht erforderlich.
- **Echte visuelle Funktionen bei Bedarf** — `--layout` aktiviert die vollständige Box-Konstruktion, Taffy-Layout, Parley-Textsatz, layoutgestützte Treffertests/Eingaben, Viewport-Screenshots und niedrigfrequente, CPU-gerenderte DevTools-Screencasts.
- **Kontrollierbare Betriebsoptionen** — Profile, Cookies, HTTP-Cache, Proxys, Ressourcengruppen, Verbindungslimits, Zeitüberschreitungen, Richtlinien für private Netzwerke, User-Agent-Überschreibungen, strukturierte Protokollierung und Netzwerkdiagnose sind vollständig verfügbar.

## Die Beziehung zwischen Moli und Lexmount

Moli ist der Open-Source-Headless-Browser von Lexmount; Lexmount Browser ist die darum herum aufgebaute verwaltete Cloud-Laufzeit und Steuerungsebene.

**Der Open-Source-Headless-Browser selbst ist vollständig nutzbar und nicht von Lexmount Browser abhängig.**

## Kostensteuerung

Aufwendige Browseroperationen müssen in Moli ausdrücklich aktiviert werden und sind standardmäßig ausgeschaltet:

| Modus oder Option | Verhalten |
| --- | --- |
| Standard | `LayoutPolicy::Mock` — deterministische, formatkompatible Geometrie, kein echtes Layout und kein Zeichnen |
| `--layout` | `LayoutPolicy::OnDemand` — echtes Layout, Geometrie, Treffertests, Koordinateneingaben, Screenshots und Screencast |
| `--resource` | Alle optionalen visuellen und Medienressourcengruppen abrufen |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Eine bestimmte optionale Ressourcengruppe aktivieren |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Persistenz je nach Bedarf des Workloads gezielt aktivieren |

Das Layoutergebnis ist ein bei Bedarf erstellter Snapshot und kein dauerhaft gepflegter Zustand: Die erste Geometrieanfrage (Kaltstart) baut ein vollständiges Layout aus dem aktuellen DOM/Stil auf und behält nur die neueste `LayoutPassOutput`. Danach können normale Geometrieabfragen diesen Snapshot selbst dann wiederverwenden, wenn sich die Seite geändert hat. Screenshots und Screencasts werden dagegen jedes Mal neu aufgebaut und verwenden keine alten Ergebnisse.

## Architektur

Moli ist ein eigenständiger Browser-Kernel und kein Chromium-Wrapper. Er ist in Rust entwickelt, folgt eigenen Regeln für Besitz und Lebenszyklus und verwendet als zentrale Abhängigkeiten:

- `libcurl` — Netzwerktransport und Laufzeit für mehrere Anfragen
- `html5ever` — HTML-Parsing
- `rusty_v8` / V8 — JavaScript-Ausführung
- Servo/Stylo — Selektoren, Kaskade und berechnete Stile
- Taffy + Parley — Box- und Textlayout
- AnyRender/Vello CPU, `usvg` und das Rust-Bildökosystem — Software-Rendering

Dokument und Stil haben genau eine maßgebliche Datenquelle: die Integration von nativem DOM und Stylo. Jede echte Aktualisierung baut das Layout daraus neu auf, überführt das Ergebnis in DOM-neutrale, unveränderliche Daten und verwirft anschließend den temporären Zustand aus diesem Layout- und Zeichendurchlauf. Das gesamte System besitzt keinen inkrementellen Layoutbaum, keinen Damage-Graph, keine beibehaltene Displayliste, keinen GPU-Compositor und kein persistentes Fenster.

## Testdaten

Die folgenden zwei Messreihen zeigen Molis derzeitigen Funktionsumfang. Die Tests decken reale Websites, reale Automatisierungsclients, gezielte Prüfungen des Chromium-/WPT-Verhaltens und eine große nextest-Regressionssuite ab.

### Gemischter Crawl-Test des öffentlichen Webs

Getestet wurden 192 öffentliche URLs großer chinesischer und internationaler Websites. Als Erfolg galt nur eine Seite, die nach Ausführung von JavaScript inhaltlich verwertbare Ergebnisse lieferte — ein HTTP-200-Status, eine Verifizierungsseite, eine Anmeldesperre, eine leere Antwort oder eine reine App-Hülle wurden nicht als Erfolg gewertet.

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

In der aktuellen WPT-Auswahl zur Überprüfung von Molis Agenten-Browser-Funktionsumfang verzeichnete ein vollständiger Testlauf **1,612 Millionen bestandene Tests**.

## Projektumfang

In den in der Dokumentation definierten Agenten-Browser-Szenarien ist Moli bereits produktionsreif und wird weiterhin kontinuierlich entwickelt.

Zu den derzeit bewusst beibehaltenen Grenzen gehören:

- Kein GUI-Browser, persistentes Fenster oder GPU-Compositor und keine beibehaltene Mehrfach-Frame-Zeichenarchitektur.
- Moli strebt keine pixelgenaue Übereinstimmung mit Chrome an und bietet keine originalgetreue Canvas-/WebGL-/Medienwiedergabe.
- Nur ein ausgewählter Teil der Funktionen von CDP, WebDriver Classic und WebDriver BiDi wird abgedeckt; eine vollständige Protokollkompatibilität ist nicht implementiert.
- Im `--layout`-Modus werden Software-Screenshots und rasterbasierte CDP-PDF-Erzeugung unterstützt, jedoch nicht sämtliche Screenshot- oder Druckmodi von Chrome.
- Ressourcenladen, Aktualität der Geometrie und Kosten des visuellen Renderings bleiben ausdrücklich festzulegende Richtlinienoptionen und sind nicht dauerhaft standardmäßig aktiv.

Nicht unterstützte Protokollpfade geben einen eindeutigen Fehler zurück — Moli täuscht nie vor, dass eine Browseraktion, ein Ereignis, eine Netzwerkbeobachtung oder ein visuelles Ergebnis stattgefunden hat.

Maintainer können mithilfe von GitHub Actions ein getaggtes Binär-Release veröffentlichen, indem sie der [Release-Anleitung](RELEASING.md) folgen.

## Lizenz

Sofern eine Datei oder ein Verzeichnis nichts anderes angibt, kann Moli wahlweise unter der [Apache License 2.0](LICENSE-APACHE) oder der [MIT License](LICENSE-MIT) genutzt werden. Separat lizenzierte Komponenten und Fixtures von Drittanbietern unterliegen weiterhin ihren jeweiligen Lizenzen und Hinweisen.
