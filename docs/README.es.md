<p align="center">
  <img
    src="../assets/moli-browser-banner.jpg"
    alt="Moli Browser — La estructura primero. Píxeles bajo demanda. Un navegador de código abierto para agentes de IA."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="../README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.de.md">Deutsch</a> |
  <a href="README.fr.md">Français</a> |
  <strong>Español</strong>
</p>

Moli es un navegador headless para agentes de IA, listo para producción. Su diseño de layout y renderizado bajo demanda combina un entorno de ejecución de navegador completo con un consumo reducido de recursos.

Moli ayuda a tu agente de IA a obtener y extraer páginas web, buscar en la web y automatizar tareas de navegador.

Puedes usarlo desde la CLI, o mediante CDP, WebDriver Classic o WebDriver BiDi.

Moli es compatible con Linux, macOS y Windows.

## Inicio rápido

Dale esta instrucción a tu agente de IA:

```text
Instala los skills de https://github.com/lexmount/moli/tree/main/skills y sigue sus
instrucciones para descargar e instalar el binario precompilado más reciente de
Moli. Después usa moli-webfetch para obtener https://example.com y enséñame el
resultado.
```

### Instalación directa

En Linux o macOS:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/lexmount/moli/releases/latest/download/moli-installer.sh | sh
```

En Windows, ejecuta este comando en PowerShell:

```powershell
irm https://github.com/lexmount/moli/releases/latest/download/moli-installer.ps1 | iex
```

## Demostración

<p align="center">
  <a href="../assets/moli-game.jpg">
    <img
      src="../assets/moli-game.jpg"
      alt="Un juego HTML5 renderizado por Moli e inspeccionado con Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Un juego HTML5 renderizado por Moli e inspeccionado en vivo con Chrome DevTools.</sub>
</p>

<p align="center">
  <a href="../assets/moli-devtools-rust-lang.jpg">
    <img
      src="../assets/moli-devtools-rust-lang.jpg"
      alt="El sitio rust-lang.org renderizado por Moli e inspeccionado con Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>rust-lang.org renderizado por Moli, con su DOM, CSS y geometría disponibles en vivo desde Chrome DevTools.</sub>
</p>

## Uso de la CLI

### Extraer una página

Renderiza la página a Markdown con la estrategia de finalización predeterminada de Moli:

```bash
moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

También puedes pedir directamente un árbol semántico compacto, pensado para que un modelo lo procese sin esfuerzo:

```bash
moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

Para obtener una salida visual, activa el layout bajo demanda y genera una captura PNG del viewport, una captura PNG del documento completo o un PDF paginado:

```bash
moli fetch --layout --dump screenshot https://example.com > page.png
moli fetch --layout --dump screenshot_full https://example.com > full-page.png
moli fetch --layout --dump pdf https://example.com > page.pdf
```

Ejecuta `fetch --help` para ver la lista completa de parámetros: formatos de salida, esperas de carga de página o de respuesta, perfiles, configuración de proxy, políticas de recursos y opciones de trazado.

### Iniciar el servidor de automatización

```bash
# Servidor de automatización básico, pensado para cargas de trabajo centradas en el DOM
moli serve

# Activar geometría real, entradas por coordenadas y capacidades de captura/screencast
moli serve --layout

# Traer también recursos opcionales: imágenes, fuentes, audio, vídeo, multimedia y pistas de texto
moli serve --layout --resource
```

El mismo endpoint expone los tres protocolos —CDP, WebDriver Classic y WebDriver BiDi—, así que Playwright puede conectarse directamente por CDP:

```js
import { chromium } from "playwright";

const browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
const context = browser.contexts()[0];
const page = context.pages()[0] ?? await context.newPage();

await page.goto("https://example.com");
console.log(await page.locator("body").innerText());

await browser.close();
```

## Por qué elegir Moli

En las cargas de trabajo de agentes hay tres cualidades que importan especialmente, y Moli las reúne todas:

- **Completo** — JavaScript, DOM, CSS, red, almacenamiento, layout, capturas de pantalla y los protocolos de automatización estándar de verdad, todo en un único navegador headless.
- **Rápido** — la mayoría de las peticiones de automatización no necesitan renderizado visual, así que las operaciones centradas en la estructura se saltan por completo el layout y el pintado.
- **Eficiente en recursos** — el layout y los píxeles solo se calculan cuando hacen falta: Moli no tiene que mantener ni actualizar constantemente un estado visual completamente renderizado.

Lo que la mayoría de las tareas de automatización de navegador necesitan de verdad es la estructura de la página, no un mundo visual renderizándose sin parar. Moli trata el DOM nativo y el estado de los estilos como única fuente de verdad, y solo dispara el layout o el pintado por software cuando la operación realmente lo requiere.

| Petición del agente | Qué hace Moli |
| --- | --- |
| Extraer HTML/Markdown, consultar el DOM, ejecutar JS, inspeccionar la red o el almacenamiento | Lee el estado del runtime del navegador directamente, sin disparar layout ni pintado |
| Leer el bounding box de un elemento, comprobar coordenadas, enviar entradas por coordenadas | Calcula el layout y se queda solo con el árbol de layout congelado más reciente |
| Capturar una pantalla o actualizar un screencast | Reconstruye a partir del DOM y los estilos actuales, sustituye el árbol congelado, renderiza un frame nuevo y lo descarta después de usarlo |

<p align="center">
  <a href="../assets/moli_ondemand_rendering_flow.svg">
    <img
      src="../assets/moli_ondemand_rendering_flow.svg"
      alt="Cómo procesa Moli una solicitud: prioriza el DOM de forma predeterminada y solo reconstruye la disposición y el dibujo bajo demanda"
      width="680"
    />
  </a>
</p>

Moli sigue teniendo todas las piezas necesarias: V8, CSS, layout, composición de texto, hit-testing, pintado por software y mucho más. La diferencia está en *cuándo* se ejecuta el trabajo visual y *durante cuánto tiempo* se conservan sus resultados. Este modelo de costes encaja especialmente bien con web scraping, agentes que usan un navegador, pipelines de recuperación de información, entornos de evaluación y cargas de trabajo de reinforcement learning.

## Capacidades disponibles actualmente

- **Runtime web completo** — parsing de HTML en streaming, DOM nativo, JavaScript con V8, módulos/timers/microtasks/eventos, iframes y workers, cascada CSS, Fetch/XHR/WebSocket, cookies, WebCrypto y almacenamiento aislado por perfil (localStorage, IndexedDB, OPFS).
- **Salidas pensadas para extracción** — la CLI genera directamente HTML, Markdown, JSON, árboles de texto semánticos y resultados serializados con información de frames, y admite esperas por selector/script/respuesta además de trazado de red.
- **Stack de automatización unificado** — CDP, WebDriver Classic y WebDriver BiDi comparten el mismo núcleo y el mismo scheduler. No hace falta instalar por separado ChromeDriver, geckodriver ni el propio navegador.
- **Capacidades visuales reales, bajo demanda** — con `--layout` se activan la construcción completa de cajas, el layout con Taffy, la composición de texto con Parley, hit-testing e inputs basados en layout, capturas del viewport y screencasts de DevTools renderizados por CPU a baja frecuencia.
- **Opciones operativas configurables** — perfiles, cookies, caché HTTP, proxies, familias de recursos, límites de conexión, timeouts, políticas de red privada, override de User-Agent, logs estructurados y diagnóstico de red.

## Relación entre Moli y Lexmount

Moli es el navegador headless de código abierto de Lexmount; Lexmount Browser es el runtime gestionado en la nube y el plano de control que se construyen alrededor de él.

**El navegador headless de código abierto funciona de forma totalmente independiente: no necesitas Lexmount Browser para usarlo.**

## Control de costes

En Moli, las operaciones caras del navegador hay que activarlas explícitamente: nunca vienen habilitadas por defecto.

| Modo u opción | Comportamiento |
| --- | --- |
| Por defecto | `LayoutPolicy::Mock`: devuelve geometría determinista, compatible con el formato esperado, sin layout ni pintado reales |
| `--layout` | `LayoutPolicy::OnDemand`: layout real, geometría, hit-testing, inputs por coordenadas, capturas de pantalla y screencast |
| `--resource` | Descarga todas las familias opcionales de recursos visuales y multimedia |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Activa por separado una familia concreta de recursos opcionales |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Activa de forma selectiva la persistencia que necesite tu carga de trabajo |

El resultado del layout es una instantánea tomada bajo demanda, no un estado que se mantiene todo el tiempo: la primera petición de geometría (cold start) construye un árbol de trabajo temporal a partir del DOM y los estilos actuales, congela su geometría canónica en un `FrozenLayoutTree` inmutable e independiente del DOM, y se queda solo con ese árbol, el más reciente. Las lecturas de geometría normales pueden reutilizarlo aunque la página haya cambiado entretanto; las capturas de pantalla y los screencasts, en cambio, siempre reconstruyen y sustituyen el árbol congelado, y nunca reutilizan resultados de pintado antiguos.

## Arquitectura

Moli es un motor de navegador independiente, no un wrapper de Chromium. Está construido en Rust, con sus propias reglas de ownership y ciclo de vida, y entre sus dependencias principales están:

- `libcurl` — transporte de red y runtime para múltiples requests
- `html5ever` — parsing de HTML
- `rusty_v8` / V8 — ejecución de JavaScript
- Servo/Stylo — selectores, cascada y estilos calculados
- Taffy + Parley — layout de cajas y de texto
- AnyRender/Vello CPU, `usvg` y el ecosistema de imágenes de Rust — renderizado por software

El documento y los estilos tienen una única fuente de verdad: la integración del DOM nativo con Stylo. Cada actualización real crea un árbol de trabajo temporal, genera y consume bajo demanda una nueva instantánea de pintado, congela la geometría final de cajas y fragmentos en un `FrozenLayoutTree` compacto, y después descarta el árbol de trabajo, las referencias de estilo, las cachés de layout, los diagnósticos y el estado de pintado. Los índices de origen y los candidatos de hit-test se derivan del árbol congelado en el momento de la consulta. No hay ningún árbol de layout mantenido de forma incremental, ni damage graph, ni display list retenida, ni compositor de GPU, ni ventana persistente.

## Datos de las pruebas

Las siguientes mediciones muestran las capacidades actuales de Moli. Cubren sitios web reales, clientes de automatización, verificaciones del comportamiento de Chromium/WPT y una batería grande de regresión con nextest.

### Prueba de rastreo mixto de la web pública

La prueba cubre 192 URLs públicas de sitios importantes de China y del resto del mundo. Para contar como éxito, una página tiene que generar contenido realmente útil después de ejecutar JavaScript: un simple 200 OK, una página de verificación, un muro de login, una respuesta vacía o una interfaz de aplicación que solo muestre su esqueleto básico no cuentan como resultado válido.

| Navegador | Páginas útiles | Tasa de éxito | Tiempo mediano | RSS mediana |
| --- | ---: | ---: | ---: | ---: |
| **Moli** | **103** | **53.6%** | **1.43 s** | **73 MiB** |
| Chrome Headless | 101 | 52.6% | 1.43 s | 773 MiB |
| Lightpanda | 85 | 44.3% | 0.97 s | 40 MiB |
| Obscura | 57 | 29.7% | 1.30 s | 39 MiB |

### Ejemplo de carga de trabajo de un agente

| Métrica | Moli | Chromium |
| --- | ---: | ---: |
| CDP listo | 34.85 ms | 169.37 ms |
| Tiempo activo del episodio (p50) | 33.40 ms | 57.13 ms |
| PSS máximo | 102.46 MiB | 348.82 MiB |
| Procesos / hilos máximos | 1 / 24 | 11 / 123 |

### Pruebas WPT

En la selección WPT actual que valida el alcance de Moli como navegador para agentes, una ejecución completa superó **1.612.000 pruebas**.

### Rendimiento de Moli en Lexbench-Headless-Browser

El conjunto completo de [Lexbench-Headless-Browser](https://github.com/lexmount/Lexbench-Headless-Browser) contiene 1.928 tareas que cubren CDP directo, 13 herramientas de automatización con versiones fijadas, entre ellas Playwright, Puppeteer y Selenium, y la semántica de la plataforma web. Para incluir a Kitesurf, que solo está disponible como endpoint remoto, el gráfico utiliza 1.308 tareas comparables. Todos los navegadores siguen las mismas reglas de selección.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-five-engine-caliber-b-dark.jpg">
  <img alt="Tasa de éxito de cinco navegadores headless sobre 1.308 tareas comparables: Chrome 99,8 %, Moli 81,9 %, Kitesurf 62,1 %, Lightpanda 53,3 %, Obscura 44,9 %" src="../assets/lexbench-five-engine-caliber-b-light.jpg" width="100%">
</picture>

**Moli 0.1.1 superó 1.071 tareas, con una tasa de éxito del 81,88 %**, por encima de Kitesurf con un 62,08 %, Lightpanda con un 53,29 % y Obscura con un 44,88 %; Chrome, usado como referencia, alcanzó el 99,85 %. Kitesurf se ejecutó con k=1, las tareas no cubiertas cuentan como no superadas y las condiciones de reproducción de un servicio remoto difieren de las de los binarios locales. Los resultados completos están en el [informe de cinco motores](https://github.com/lexmount/Lexbench-Headless-Browser/blob/kitesurf-eval/docs/reports/five-engine-report-20260813.md) del benchmark.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/lexbench-efficiency-map-dark.jpg">
  <img alt="Tasa de éxito frente a la mediana de memoria pico por tarea para los cuatro motores locales: Chrome con 99,9 % y 697 MiB, Moli con 80,7 % y 92 MiB, Lightpanda con 43,8 % y 34 MiB, Obscura con 39,5 % y 39 MiB" src="../assets/lexbench-efficiency-map-light.jpg" width="100%">
</picture>

Kitesurf es un servicio remoto, por lo que no se pueden medir su CPU, memoria ni número de procesos. La comparación de recursos solo cubre los cuatro motores locales. Una ejecución independiente de 557 tareas cuenta únicamente el trabajo que completaron los cuatro. La mediana de Moli fue de **100,6 ms de CPU** y **92 MiB de memoria pico** por tarea; Chrome registró **687 ms** y **697 MiB**, respectivamente. Moli utilizó alrededor del 15 % del tiempo de CPU y del 13 % de la memoria pico de Chrome. La metodología y los datos completos están en la [ficha de recursos](https://github.com/lexmount/Lexbench-Headless-Browser/blob/main/docs/reports/resource-card-20260812.md) del benchmark.

## Alcance del proyecto

Dentro de los escenarios de navegador para agentes que cubre la documentación, Moli ya está listo para producción y sigue en desarrollo activo.

Estos son los límites que se mantienen a propósito:

- No es un navegador con interfaz gráfica, no tiene ventana persistente ni compositor de GPU, y tampoco implementa una arquitectura de pintado retenido multi-frame.
- No busca un renderizado pixel-perfect idéntico al de Chrome, ni ofrece reproducción de Canvas, WebGL o contenido multimedia de alta fidelidad.
- El modo `--layout` soporta capturas de pantalla por software y generación de PDF rasterizado vía CDP, pero no implementa todos los modos de captura o impresión de Chrome.

Los caminos de protocolo no soportados devuelven un error explícito: Moli nunca finge que ha ocurrido una acción del navegador, un evento, una observación de red o un resultado visual.

## Historial de stars

Generado cada hora a partir de la cronología de stars por [lexmount/moli-metrics](https://github.com/lexmount/moli-metrics).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/lexmount/moli-metrics/main/assets/star-history-dark.svg">
  <img alt="Historial de stars de lexmount/moli" src="https://raw.githubusercontent.com/lexmount/moli-metrics/main/assets/star-history.svg" width="100%">
</picture>

## Licencia

Salvo que se indique lo contrario en un archivo o directorio concreto, Moli puede usarse, a elección de quien lo use, bajo la [Licencia Apache 2.0](../LICENSE-APACHE) o la [Licencia MIT](../LICENSE-MIT). Los componentes y fixtures de terceros con licencia propia siguen sujetos a sus respectivas licencias y avisos.
