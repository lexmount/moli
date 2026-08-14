<p align="center">
  <img
    src="assets/moli-browser-banner.jpg"
    alt="Moli Browser — La structure d'abord. Les pixels à la demande. Un navigateur open source pour les agents d'IA."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.de.md">Deutsch</a> |
  <strong>Français</strong> |
  <a href="README.es.md">Español</a>
</p>

Moli est un navigateur headless prêt pour la production et destiné aux agents d'IA. Sa conception fondée sur une mise en page et un rendu à la demande associe un environnement d'exécution de navigateur complet à une faible empreinte sur les ressources.

Par défaut, il exécute du code JavaScript réel, maintient un véritable DOM et fournit de véritables API de navigateur. Il ne calcule la mise en page et ne rend les pixels que lorsqu'ils sont réellement nécessaires.

Utilisez-le via la CLI, CDP, WebDriver Classic ou WebDriver BiDi.

## Démarrage rapide

Donnez cette instruction à votre agent de programmation IA :

```text
Installe les skills sous https://github.com/lexmount/moli/tree/main/skills, suis leurs instructions pour télécharger et installer le dernier binaire Moli précompilé, puis utilise moli-webfetch pour récupérer https://example.com et montre-moi le résultat.
```

## Démonstration

<p align="center">
  <a href="assets/moli-game.jpg">
    <img
      src="assets/moli-game.jpg"
      alt="Un jeu HTML5 rendu par Moli et inspecté avec Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Un jeu HTML5 rendu par Moli et inspecté en direct avec Chrome DevTools.</sub>
</p>

<p align="center">
  <a href="assets/moli-devtools-rust-lang.jpg">
    <img
      src="assets/moli-devtools-rust-lang.jpg"
      alt="Le site rust-lang.org rendu par Moli et inspecté avec Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Le site rust-lang.org rendu par Moli, avec son DOM, son CSS et sa géométrie en direct dans Chrome DevTools.</sub>
</p>

## Utilisation de la CLI

### Extraire une page

Effectuez le rendu de la page au format Markdown avec la stratégie d'achèvement par défaut de Moli :

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

Ou renvoyez directement un arbre sémantique compact et adapté aux modèles :

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

Pour une sortie visuelle, activez la mise en page à la demande et générez directement une capture PNG de la fenêtre d'affichage ou un PDF paginé :

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

Exécutez `fetch --help` pour obtenir la liste complète des paramètres, notamment les formats de sortie, les attentes de chargement de page ou de réponse, les profils, les réglages du proxy, les politiques de ressources et les options de traçage.

### Démarrer le serveur d'automatisation

```bash
# Serveur d'automatisation de base pour les charges de travail privilégiant le DOM
moli serve

# Activer la géométrie réelle, les entrées par coordonnées et les fonctions de capture/screencast
moli serve --layout

# Récupérer aussi les ressources facultatives d'image, de police, d'audio, de vidéo, de média et de piste de texte
moli serve --layout --resource
```

Le même point de terminaison fournit les trois protocoles : CDP, WebDriver Classic et WebDriver BiDi. Playwright peut s'y connecter directement via CDP :

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

Trois qualités sont essentielles aux charges de travail des agents, et Moli les réunit :

- **Complet** — JavaScript, DOM, CSS, réseau, stockage, mise en page, captures d'écran et protocoles d'automatisation standard réels, tous intégrés dans un seul navigateur headless.
- **Rapide** — la plupart des requêtes d'automatisation n'ont aucun besoin de rendu visuel ; les opérations axées sur la structure ignorent donc entièrement la mise en page et le dessin.
- **Économe en ressources** — la mise en page et les pixels ne sont produits que lorsqu'ils sont nécessaires ; Moli n'a donc pas à maintenir et actualiser en permanence un état visuel entièrement rendu.

Ce dont la plupart des tâches d'automatisation ont réellement besoin, c'est de la structure de la page, pas d'un univers visuel rendu en continu. Moli traite le DOM natif et l'état des styles comme l'unique source de vérité, et ne déclenche la mise en page ou le dessin logiciel que pour les opérations qui exigent réellement ces calculs.

| Requête de l'agent | Traitement par Moli |
| --- | --- |
| Extraire du HTML/Markdown, interroger le DOM, exécuter du JS, inspecter le réseau ou le stockage | Lit directement l'état de l'environnement d'exécution du navigateur — ne déclenche ni mise en page ni dessin |
| Lire la boîte englobante d'un élément, tester des coordonnées, envoyer une entrée par coordonnées | Exécute un calcul de mise en page et ne conserve que le dernier instantané de géométrie |
| Prendre une capture d'écran ou actualiser un screencast | Reconstruit à partir du DOM et des styles actuels, rend une nouvelle image, puis la supprime après usage |

<p align="center">
  <a href="assets/moli_ondemand_rendering_flow.svg">
    <img
      src="assets/moli_ondemand_rendering_flow.svg"
      alt="Traitement d'une requête par Moli : priorité au DOM par défaut, mise en page et dessin reconstruits uniquement à la demande"
      width="680"
    />
  </a>
</p>

Moli continue d'intégrer toutes les capacités nécessaires : V8, CSS, mise en page, composition du texte, tests de pointage, dessin logiciel, et bien plus encore. La seule différence porte sur le moment où le travail visuel s'exécute et la durée pendant laquelle ses résultats sont conservés. Ce modèle de coût convient particulièrement à l'exploration du Web, aux agents utilisant un navigateur, aux pipelines de recherche d'information, aux environnements d'évaluation et aux charges de travail d'apprentissage par renforcement.

## Fonctionnalités actuellement prises en charge

- **Environnement Web complet** — analyse HTML en flux continu, DOM natif, JavaScript V8, modules/minuteurs/microtâches/événements, iframes et workers, cascade CSS, Fetch/XHR/WebSocket, cookies, WebCrypto et stockage propre à chaque profil (localStorage, IndexedDB, OPFS).
- **Sorties optimisées pour l'extraction** — la CLI produit directement du HTML, du Markdown, du JSON, des arbres de texte sémantiques et des résultats sérialisés contenant les informations de frame, avec des attentes de sélecteur/script/réponse et le traçage réseau.
- **Programme d'automatisation unifié** — CDP, WebDriver Classic et WebDriver BiDi partagent le même noyau et le même ordonnanceur. Aucune installation distincte de ChromeDriver, geckodriver ou du navigateur lui-même n'est nécessaire.
- **Fonctions visuelles réelles à la demande** — l'option `--layout` active la construction complète des boîtes, la mise en page Taffy, la composition du texte Parley, les tests de pointage/entrées fondés sur la mise en page, les captures de la fenêtre d'affichage et les screencasts DevTools rendus à basse fréquence par le processeur.
- **Options d'exploitation contrôlables** — profils, cookies, cache HTTP, proxys, familles de ressources, limites de connexions, délais d'expiration, politique de réseau privé, remplacement du User-Agent, journalisation structurée et diagnostics réseau sont tous disponibles.

## La relation entre Moli et Lexmount

Moli est le navigateur headless open source de Lexmount ; Lexmount Browser est l'environnement d'exécution cloud géré et le plan de contrôle construits autour de celui-ci.

**Le navigateur headless open source est entièrement utilisable seul, sans dépendre de Lexmount Browser.**

## Maîtrise des coûts

Dans Moli, les opérations coûteuses du navigateur doivent être activées explicitement et ne le sont jamais par défaut :

| Mode ou option | Comportement |
| --- | --- |
| Par défaut | `LayoutPolicy::Mock` — géométrie déterministe dans un format compatible, sans véritable mise en page ni dessin |
| `--layout` | `LayoutPolicy::OnDemand` — véritables mise en page, géométrie, tests de pointage, entrées par coordonnées, captures d'écran et screencast |
| `--resource` | Récupère toutes les familles facultatives de ressources visuelles ou multimédias |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Active une famille précise de ressources facultatives |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Active sélectivement la persistance nécessaire à la charge de travail |

Le résultat de la mise en page est un instantané produit à la demande, et non un état maintenu en continu : la première demande de géométrie (à froid) construit une mise en page complète à partir du DOM et des styles actuels, puis ne conserve que le dernier `LayoutPassOutput`. Par la suite, les lectures de géométrie ordinaires peuvent réutiliser cet instantané même si la page a changé. Les captures d'écran et les screencasts, eux, sont reconstruits à chaque fois et ne réutilisent jamais d'anciens résultats.

## Architecture

Moli est un noyau de navigateur autonome, et non une surcouche de Chromium. Développé en Rust, il possède ses propres règles de propriété et de cycle de vie. Ses principales dépendances sont :

- `libcurl` — transport réseau et environnement d'exécution multirequête
- `html5ever` — analyse HTML
- `rusty_v8` / V8 — exécution de JavaScript
- Servo/Stylo — sélecteurs, cascade et styles calculés
- Taffy + Parley — mise en page des boîtes et du texte
- AnyRender/Vello CPU, `usvg` et l'écosystème d'images Rust — rendu logiciel

Le document et les styles n'ont qu'une seule source de vérité : le DOM natif et son intégration à Stylo. Chaque véritable actualisation reconstruit la mise en page à partir de cette source, convertit le résultat en données immuables indépendantes du DOM, puis élimine l'état temporaire produit pendant cette passe de mise en page et de dessin. L'ensemble du système ne comporte ni arbre de mise en page incrémental, ni graphe de dommages, ni liste d'affichage persistante, ni compositeur GPU, ni fenêtre persistante.

## Données de test

Les deux ensembles de mesures ci-dessous montrent le périmètre actuel des capacités de Moli. Les tests couvrent des sites réels, de véritables clients d'automatisation, des vérifications ciblées du comportement Chromium/WPT et une vaste suite de régression nextest.

### Test d'exploration mixte du Web public

Le test porte sur 192 URL publiques de grands sites chinois et internationaux. Pour être considérée comme réussie, une page doit produire un contenu réellement utile après l'exécution de JavaScript : un code HTTP 200, une page de vérification, un mur de connexion, une réponse vide ou une interface d'application réduite à sa coquille ne suffisent pas.

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

Dans la sélection WPT actuelle utilisée pour valider le périmètre fonctionnel du navigateur pour agents de Moli, une exécution complète a enregistré **1 612 000 tests réussis**.

## Périmètre du projet

Dans les scénarios de navigateur pour agents définis par la documentation, Moli est déjà prêt pour la production et continue de faire l'objet d'un développement actif.

Les limites actuellement conservées de manière intentionnelle comprennent :

- Aucun navigateur avec interface graphique, aucune fenêtre persistante, aucun compositeur GPU et aucune architecture persistante de dessin multiframe.
- Moli ne cherche pas à reproduire Chrome pixel par pixel et ne propose pas de lecture haute fidélité de Canvas, WebGL ou des médias.
- Seule une partie des fonctionnalités de CDP, WebDriver Classic et WebDriver BiDi est couverte, et non une implémentation entièrement compatible de ces protocoles.
- Le mode `--layout` prend en charge les captures d'écran logicielles et la génération de PDF CDP rastérisés, mais pas tous les modes de capture ou d'impression de Chrome.
- Le chargement des ressources, l'actualisation de la géométrie et le coût du rendu visuel restent des choix de politique à définir explicitement, et ne sont pas activés en permanence par défaut.

Les chemins de protocole non pris en charge renvoient une erreur explicite : Moli ne prétend jamais qu'une action du navigateur, un événement, une observation réseau ou un résultat visuel a eu lieu.

Les mainteneurs peuvent publier une version binaire balisée depuis GitHub Actions en suivant le [guide de publication](RELEASING.md).

## Licence

Sauf indication contraire dans un fichier ou un répertoire, Moli peut être utilisé, au choix, sous [licence Apache 2.0](LICENSE-APACHE) ou sous [licence MIT](LICENSE-MIT). Les composants et les fixtures tiers sous licence distincte restent soumis à leurs propres licences et mentions.
