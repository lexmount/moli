<p align="center">
  <img
    src="../assets/moli-browser-banner.jpg"
    alt="Moli Browser — La structure d'abord. Les pixels à la demande. Un navigateur open source pour les agents d'IA."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="../README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.de.md">Deutsch</a> |
  <strong>Français</strong> |
  <a href="README.es.md">Español</a>
</p>

Moli est un navigateur headless conçu pour la production, pensé dès le départ pour les agents d'IA. Grâce à une architecture de mise en page et de rendu à la demande, il combine un moteur de navigateur complet avec une faible consommation de ressources.

Il permet à votre agent d'IA de récupérer et d'extraire le contenu de pages web, d'effectuer des recherches en ligne et d'automatiser des tâches dans le navigateur.

Vous pouvez utiliser Moli via la CLI, CDP, WebDriver Classic ou WebDriver BiDi.

Moli prend en charge Linux, macOS et Windows.

## Démarrage rapide

Donnez l'instruction suivante à votre agent IA :

```text
Installe les skills sous https://github.com/lexmount/moli/tree/main/skills,
suis leurs instructions pour télécharger et installer le dernier binaire Moli
précompilé, puis utilise moli-webfetch pour récupérer https://example.com et
montre-moi le résultat.
```

### Installation directe

Sous Linux ou macOS :

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/lexmount/moli/releases/latest/download/moli-installer.sh | sh
```

Sous Windows, exécutez cette commande dans PowerShell :

```powershell
irm https://github.com/lexmount/moli/releases/latest/download/moli-installer.ps1 | iex
```

## Démonstration

<p align="center">
  <a href="../assets/moli-game.jpg">
    <img
      src="../assets/moli-game.jpg"
      alt="Un jeu HTML5 rendu par Moli et inspecté avec Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Un jeu HTML5 rendu par Moli, inspecté en direct avec Chrome DevTools.</sub>
</p>

<p align="center">
  <a href="../assets/moli-devtools-rust-lang.jpg">
    <img
      src="../assets/moli-devtools-rust-lang.jpg"
      alt="Le site rust-lang.org rendu par Moli et inspecté avec Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Le site rust-lang.org rendu par Moli : DOM, CSS et géométrie visibles en direct dans Chrome DevTools.</sub>
</p>

## Utilisation en ligne de commande

### Extraire une page

Générez le rendu de la page en Markdown avec la stratégie de complétion par défaut de Moli :

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

Ou récupérez directement un arbre sémantique compact, optimisé pour les modèles de langage :

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

Pour une sortie visuelle, activez la mise en page à la demande afin de générer une capture PNG du viewport, une capture PNG du document complet ou un PDF paginé :

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump screenshot_full https://example.com > full-page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

Lancez `fetch --help` pour la liste complète des options : formats de sortie, conditions d'attente (chargement de page, réponse réseau), profils, configuration du proxy, politiques de ressources et options de traçage.

### Démarrer le serveur d'automatisation

```bash
# Serveur d'automatisation de base pour les charges de travail privilégiant le DOM
moli serve

# Activer la géométrie réelle, les entrées par coordonnées et les fonctions de capture/screencast
moli serve --layout

# Récupérer aussi les ressources facultatives d'image, de police, d'audio, de vidéo, de média et de piste de texte
moli serve --layout --resource
```

Ce même point d'accès expose les trois protocoles — CDP, WebDriver Classic et WebDriver BiDi. Playwright peut donc s'y connecter directement via CDP :

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const context = browser.contexts()[0];
const page = context.pages()[0] ?? await context.newPage();

await page.goto("https://example.com");
console.log(await page.locator("body").innerText());

await browser.close();
```

## Pourquoi Moli ?

Trois qualités comptent vraiment pour les charges de travail agentiques, et Moli les réunit toutes :

- **Complet** — JavaScript, DOM, CSS, réseau, stockage, mise en page, captures d'écran et véritables protocoles d'automatisation standard, le tout réuni dans un seul navigateur headless.
- **Rapide** — la plupart des requêtes d'automatisation n'ont besoin d'aucun rendu visuel : les opérations purement structurelles court-circuitent donc entièrement la mise en page et le dessin.
- **Économe en ressources** — la mise en page et les pixels ne sont calculés qu'en cas de besoin réel : Moli n'a donc jamais à maintenir en permanence un état visuel entièrement rendu.

Ce dont la plupart des tâches d'automatisation ont réellement besoin, c'est de la structure de la page — pas d'un monde visuel rendu en continu. Moli considère le DOM natif et l'état des styles comme l'unique source de vérité, et ne déclenche la mise en page ou le rendu logiciel que lorsque l'opération l'exige vraiment.

| Requête de l'agent | Traitement par Moli |
| --- | --- |
| Extraire du HTML/Markdown, interroger le DOM, exécuter du JS, inspecter le réseau ou le stockage | Lit directement l'état du moteur du navigateur, sans déclencher ni mise en page ni rendu |
| Lire la boîte englobante d'un élément, tester des coordonnées, envoyer une entrée par coordonnées | Calcule la mise en page à la volée et ne conserve que le dernier arbre figé |
| Prendre une capture d'écran ou actualiser un screencast | Reconstruit à partir du DOM et des styles actuels, remplace l'arbre figé, produit une nouvelle image, puis la libère aussitôt après usage |

<p align="center">
  <a href="../assets/moli_ondemand_rendering_flow.svg">
    <img
      src="../assets/moli_ondemand_rendering_flow.svg"
      alt="Traitement d'une requête par Moli : priorité au DOM par défaut, mise en page et dessin reconstruits uniquement à la demande"
      width="680"
    />
  </a>
</p>

Moli embarque toujours toutes les briques nécessaires : V8, CSS, mise en page, composition de texte, hit-testing, rendu logiciel, et bien plus. Ce qui change, c'est le moment où ce travail visuel s'exécute, et la durée pendant laquelle ses résultats sont conservés. Ce modèle de coût convient particulièrement bien au crawling du Web, aux agents pilotant un navigateur, aux pipelines de recherche d'information, aux environnements d'évaluation et aux charges de travail d'apprentissage par renforcement.

## Fonctionnalités actuellement prises en charge

- **Environnement Web complet** — parsing HTML en streaming, DOM natif, JavaScript V8, modules/timers/microtâches/événements, iframes et workers, cascade CSS, Fetch/XHR/WebSocket, cookies, WebCrypto et stockage par profil (localStorage, IndexedDB, OPFS).
- **Sorties pensées pour l'extraction** — la CLI produit directement du HTML, du Markdown, du JSON, des arbres de texte sémantiques et des résultats sérialisés incluant les informations de frame, avec gestion des conditions d'attente (sélecteur, script, réponse) et traçage réseau.
- **Pile d'automatisation unifiée** — CDP, WebDriver Classic et WebDriver BiDi partagent le même noyau et le même ordonnanceur : pas besoin d'installer séparément ChromeDriver, geckodriver, ni même un navigateur.
- **Rendu visuel réel, à la demande** — l'option `--layout` active la construction complète des boîtes, la mise en page via Taffy, la composition de texte via Parley, le hit-testing et les entrées fondées sur la géométrie, ainsi que les captures du viewport et les screencasts DevTools, rendus côté CPU à basse fréquence.
- **Configuration fine et maîtrisée** — profils, cookies, cache HTTP, proxys, familles de ressources, limites de connexions, timeouts, politique réseau privé, User-Agent personnalisé, logs structurés et diagnostics réseau : tout est configurable.

## Moli et Lexmount

Moli est le navigateur headless open source de Lexmount. Lexmount Browser, lui, est l'environnement d'exécution cloud managé et le plan de contrôle construits autour de Moli.

**Le navigateur headless open source fonctionne de façon totalement autonome, sans dépendre de Lexmount Browser.**

## Maîtrise des coûts

Dans Moli, les opérations coûteuses du navigateur ne sont jamais activées par défaut — il faut explicitement les demander :

| Mode ou option | Comportement |
| --- | --- |
| Par défaut | `LayoutPolicy::Mock` — géométrie déterministe au format compatible, sans véritable mise en page ni rendu |
| `--layout` | `LayoutPolicy::OnDemand` — véritables mise en page, géométrie, hit-testing, entrées par coordonnées, captures d'écran et screencast |
| `--resource` | Récupère l'ensemble des familles de ressources optionnelles (visuelles et multimédias) |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Active une famille spécifique de ressources optionnelles |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Active, au cas par cas, la persistance dont la charge de travail a besoin |

Le résultat de la mise en page est un instantané produit à la demande, pas un état maintenu en continu : la première requête de géométrie (« à froid ») construit un arbre de travail temporaire à partir du DOM et des styles courants, fige sa géométrie canonique dans un `FrozenLayoutTree` immuable et indépendant du DOM, puis ne conserve que ce dernier. Les lectures de géométrie ultérieures peuvent réutiliser cet arbre figé, même si la page a changé entre-temps. Les captures d'écran et les screencasts, eux, reconstruisent et remplacent systématiquement l'arbre figé, sans jamais réutiliser un ancien résultat de rendu.

## Architecture

Moli est un noyau de navigateur autonome — pas une surcouche posée sur Chromium. Écrit en Rust, il définit ses propres règles de propriété mémoire et de cycle de vie. Ses principales dépendances :

- `libcurl` — transport réseau et runtime multi-requêtes
- `html5ever` — parsing HTML
- `rusty_v8` / V8 — exécution JavaScript
- Servo/Stylo — sélecteurs, cascade et styles calculés
- Taffy + Parley — mise en page des boîtes et du texte
- AnyRender/Vello CPU, `usvg` et l'écosystème d'images Rust — rendu logiciel

Le document et les styles n'ont qu'une seule source de vérité : le DOM natif, intégré à Stylo. Chaque véritable rafraîchissement crée un arbre de travail temporaire, produit et consomme au besoin un nouvel instantané de rendu, fige la géométrie finale des boîtes et des fragments dans un `FrozenLayoutTree` compact, puis élimine l'arbre de travail, les références de style, les caches de mise en page, les diagnostics et l'état de rendu. Les associations aux sources et les candidats au hit-testing sont dérivés de l'arbre figé au moment des requêtes. Il n'y a ici ni arbre de mise en page maintenu de façon incrémentale, ni graphe de dommages, ni liste d'affichage persistante, ni compositeur GPU, ni fenêtre persistante.

## Données de test

Les deux jeux de mesures ci-dessous illustrent l'étendue actuelle des capacités de Moli. Les tests couvrent des sites réels, de vrais clients d'automatisation, des vérifications ciblées du comportement Chromium/WPT, ainsi qu'une large suite de régression nextest.

### Test d'exploration mixte du Web public

Le test porte sur 192 URL publiques issues de grands sites chinois et internationaux. Pour compter comme réussie, une page doit produire un contenu réellement exploitable après exécution du JavaScript : un simple code HTTP 200, une page de vérification, un mur de connexion, une réponse vide ou une coquille d'application vide ne suffisent pas.

| Navigateur | Pages utiles | Taux de réussite | Temps médian | RSS médiane |
| --- | ---: | ---: | ---: | ---: |
| **Moli** | **103** | **53.6%** | **1.43 s** | **73 MiB** |
| Chrome Headless | 101 | 52.6% | 1.43 s | 773 MiB |
| Lightpanda | 85 | 44.3% | 0.97 s | 40 MiB |
| Obscura | 57 | 29.7% | 1.30 s | 39 MiB |

### Exemple de charge de travail d'un agent

| Mesure | Moli | Chromium |
| --- | ---: | ---: |
| CDP prêt | 34.85 ms | 169.37 ms |
| Durée active d'un épisode p50 | 33.40 ms | 57.13 ms |
| PSS maximal | 102.46 MiB | 348.82 MiB |
| Nombre maximal de processus / threads | 1 / 24 | 11 / 123 |

### Lexbench-Headless-Browser (compatibilité des piles de drivers)

[Lexbench-Headless-Browser](https://github.com/lexmount/Lexbench-Headless-Browser) mesure la surface d'exécution dont dépendent réellement les écosystèmes d'automatisation : 1 928 tâches sur 13 piles de drivers réelles (CDP brut, Playwright, Puppeteer, Selenium via le WebDriver natif de Moli, chromedp, rod, chromiumoxide, ferrum, pydoll et d'autres), plus la justesse sémantique de la plateforme web, avec une vérification d'identité en deux étapes afin qu'aucun résultat de candidat ne puisse venir silencieusement de Chrome.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-four-engine-overview-dark.png">
  <img alt="Taux de réussite de quatre navigateurs headless sur 1 928 tâches : Chrome 99,9 %, Moli 80,7 %, Lightpanda 43,8 %, Obscura 39,5 %" src="../assets/lexbench-four-engine-overview-light.png" width="100%">
</picture>

Run `four_engine_full_20260812` · seed `official20260709` · k=3 ·
`--score-mode independent --chrome-baseline best_effort`:

| Moteur | Binaire | Tâches réussies | Sémantique L2 |
| --- | --- | ---: | ---: |
| Chrome for Testing | 151.0.7922.47 `3b0be9872ea9` | 1,926 / 1,928 (99.90%) | 192 / 192 |
| **Moli** | **0.1.1 `74e08f8d3eb6`** | **1,556 / 1,928 (80.71%)** | **183 / 192** |
| Lightpanda | 1.0.0-dev.321 `70f5ab69b0ce` | 845 / 1,928 (43.83%) | 132 / 192 |
| Obscura | 0.1.11 `42c7eac0f635` | 762 / 1,928 (39.52%) | 84 / 192 |

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-efficiency-map-dark.png">
  <img alt="Taux de réussite en fonction de la mémoire de pointe médiane par tâche : Chrome à 99,9 % et 697 MiB, Moli à 80,7 % et 92 MiB, Lightpanda à 43,8 % et 34 MiB, Obscura à 39,5 % et 39 MiB" src="../assets/lexbench-efficiency-map-light.png" width="100%">
</picture>

La mesure de ressources calibrée par A/B (même bench, `jobs=1 k=5`, CPU cgroup-v2 et PSS de l'arbre de processus sur l'intersection des tâches réussies par les quatre moteurs, effet d'observation sous 0,9 % par moteur) place **Moli à une médiane de 100,6 ms de CPU et 92 MiB de mémoire de pointe par tâche contre 687 ms et 697 MiB pour Chrome** : environ 1/7 du CPU et 1/7,5 de la mémoire tout en réussissant 80,7 % de la surface de tâches. Les rapports complets se trouvent dans le dépôt du bench, sous [`docs/reports/`](https://github.com/lexmount/Lexbench-Headless-Browser/tree/main/docs/reports).

Sur la sélection WPT actuelle, utilisée pour valider le périmètre fonctionnel du navigateur pour agents de Moli, une exécution complète a totalisé **1 612 000 tests réussis**.

## Périmètre du projet

Sur les scénarios de navigateur pour agents couverts par la documentation, Moli est déjà prêt pour la production, et continue de faire l'objet d'un développement actif.

Voici les limites actuellement conservées de façon délibérée :

- Pas de navigateur avec interface graphique, pas de fenêtre persistante, pas de compositeur GPU, pas d'architecture de rendu persistante multi-frame.
- Moli ne vise pas une fidélité pixel-perfect avec Chrome, et ne propose pas de rendu haute fidélité pour Canvas, WebGL ou les médias.
- Le mode `--layout` gère les captures d'écran logicielles et la génération de PDF CDP rastérisés, mais pas l'ensemble des modes de capture ou d'impression de Chrome.

Les chemins de protocole non pris en charge renvoient toujours une erreur explicite : Moli ne fait jamais semblant qu'une action du navigateur, un événement, une observation réseau ou un résultat visuel a eu lieu.

## Licence

Sauf mention contraire dans un fichier ou un répertoire donné, Moli est disponible, au choix, sous [licence Apache 2.0](../LICENSE-APACHE) ou sous [licence MIT](../LICENSE-MIT). Les composants et fixtures tiers sous licence distincte restent soumis à leurs propres licences et mentions.
