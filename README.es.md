<p align="center">
  <img
    src="assets/moli-browser-banner.jpg"
    alt="Moli Browser — La estructura primero. Píxeles bajo demanda. Un navegador de código abierto para agentes de IA."
    width="1086"
  />
</p>

<h1 align="center">Moli</h1>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ja.md">日本語</a> |
  <a href="README.de.md">Deutsch</a> |
  <a href="README.fr.md">Français</a> |
  <strong>Español</strong>
</p>

Moli es un navegador sin interfaz gráfica para agentes de IA, diseñado en torno al concepto de «renderizado bajo demanda» y listo para su uso en producción.

De forma predeterminada, ejecuta JavaScript real, mantiene un DOM real y ofrece API de navegador reales. Solo calcula la disposición o renderiza píxeles cuando son realmente necesarios.

Puede utilizarse mediante la CLI, CDP, WebDriver Classic o WebDriver BiDi.

## Demostración

<p align="center">
  <a href="assets/moli-game.jpg">
    <img
      src="assets/moli-game.jpg"
      alt="Un juego HTML5 renderizado por Moli e inspeccionado con Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>Un juego HTML5 renderizado por Moli e inspeccionado en directo con Chrome DevTools.</sub>
</p>

<p align="center">
  <a href="assets/moli-devtools-rust-lang.jpg">
    <img
      src="assets/moli-devtools-rust-lang.jpg"
      alt="El sitio rust-lang.org renderizado por Moli e inspeccionado con Chrome DevTools"
      width="1200"
    />
  </a>
</p>

<p align="center">
  <sub>El sitio rust-lang.org renderizado por Moli, con su DOM, CSS y geometría en directo disponibles en Chrome DevTools.</sub>
</p>

## Inicio rápido

Compile desde la raíz del espacio de trabajo:

```bash
cargo build --release -p moli
```

### Extraer una página

Renderice la página como Markdown con la estrategia de finalización predeterminada de Moli:

```bash
./target/release/moli fetch \
  --dump markdown \
  --wait-until done \
  https://example.com
```

También puede devolver directamente un árbol semántico compacto y fácil de procesar por un modelo:

```bash
./target/release/moli fetch \
  --dump semantic_tree_text \
  --wait-selector body \
  https://example.com
```

Ejecute `fetch --help` para consultar la lista completa de parámetros, incluidos los formatos de salida, las esperas de carga de página o de respuesta, los perfiles, la configuración del proxy, las políticas de recursos y las opciones de trazado.

### Iniciar el servidor de automatización

```bash
# Servidor de automatización básico para cargas de trabajo que priorizan el DOM
./target/release/moli serve

# Activar geometría real, entradas por coordenadas y funciones de captura/screencast
./target/release/moli serve --layout

# Obtener también recursos opcionales de imágenes, fuentes, audio, vídeo, multimedia y pistas de texto
./target/release/moli serve --layout --resource
```

El mismo punto de conexión ofrece los tres protocolos: CDP, WebDriver Classic y WebDriver BiDi. Playwright puede conectarse directamente mediante CDP:

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

Para las cargas de trabajo de los agentes hay tres cualidades especialmente importantes, y Moli las reúne:

- **Completo** — JavaScript, DOM, CSS, red, almacenamiento, disposición, capturas de pantalla y protocolos de automatización estándar reales, todo integrado en un único navegador sin interfaz gráfica.
- **Rápido** — la mayoría de las solicitudes de automatización no necesitan ningún renderizado visual, por lo que las operaciones que priorizan la estructura omiten por completo la disposición y el dibujo.
- **Eficiente en el uso de recursos** — la disposición y los píxeles solo se generan cuando hacen falta; Moli no necesita mantener y actualizar continuamente un estado visual completamente renderizado.

Lo que la mayoría de las tareas de automatización de navegadores necesitan realmente es la estructura de la página, no un mundo visual renderizado de forma continua. Moli trata el DOM nativo y el estado de los estilos como la única fuente de verdad, y solo activa la disposición o el dibujo por software para las operaciones que realmente requieren esos cálculos.

| Solicitud del agente | Qué hace Moli |
| --- | --- |
| Extraer HTML/Markdown, consultar el DOM, ejecutar JS, inspeccionar la red o el almacenamiento | Lee directamente el estado del entorno de ejecución del navegador, sin activar la disposición ni el dibujo |
| Leer el rectángulo delimitador de un elemento, comprobar unas coordenadas, enviar entradas por coordenadas | Ejecuta un cálculo de disposición y conserva únicamente la instantánea de geometría más reciente |
| Capturar una pantalla o actualizar un screencast | Reconstruye desde el DOM y los estilos actuales, renderiza un fotograma nuevo y lo descarta después de usarlo |

<p align="center">
  <a href="assets/moli_ondemand_rendering_flow.svg">
    <img
      src="assets/moli_ondemand_rendering_flow.svg"
      alt="Cómo procesa Moli una solicitud: prioriza el DOM de forma predeterminada y solo reconstruye la disposición y el dibujo bajo demanda"
      width="680"
    />
  </a>
</p>

Moli sigue incluyendo todas las capacidades necesarias: V8, CSS, disposición, composición de texto, detección por coordenadas, dibujo por software y mucho más. La única diferencia está en *cuándo* se ejecuta el trabajo visual y *durante cuánto tiempo* se conservan sus resultados. Este modelo de costes resulta especialmente adecuado para el rastreo web, los agentes que utilizan navegadores, los procesos de recuperación de información, los entornos de evaluación y las cargas de aprendizaje por refuerzo.

## Capacidades disponibles actualmente

- **Entorno de ejecución web completo** — análisis de HTML en flujo continuo, DOM nativo, JavaScript V8, módulos/temporizadores/microtareas/eventos, iframes y workers, cascada CSS, Fetch/XHR/WebSocket, cookies, WebCrypto y almacenamiento aislado por perfil (localStorage, IndexedDB, OPFS).
- **Salidas optimizadas para la extracción** — la CLI produce directamente HTML, Markdown, JSON, árboles de texto semánticos y resultados serializados con información de los frames, y admite esperas por selector/script/respuesta y trazado de red.
- **Programa de automatización unificado** — CDP, WebDriver Classic y WebDriver BiDi comparten el mismo núcleo y planificador. No es necesario instalar por separado ChromeDriver, geckodriver ni el propio navegador.
- **Capacidades visuales reales bajo demanda** — al añadir `--layout` se habilitan la construcción completa de cajas, la disposición Taffy, la composición de texto Parley, la detección y las entradas basadas en la disposición, las capturas del viewport y los screencasts de DevTools renderizados por CPU a baja frecuencia.
- **Opciones operativas controlables** — perfiles, cookies, caché HTTP, proxys, familias de recursos, límites de conexiones, tiempos de espera, políticas de red privada, sustitución del User-Agent, registros estructurados y diagnóstico de red.

## Relación entre Moli y Lexmount

Moli es el navegador sin interfaz gráfica de código abierto de Lexmount; Lexmount Browser es el entorno de ejecución gestionado en la nube y el plano de control construidos a su alrededor.

**El navegador sin interfaz gráfica de código abierto puede utilizarse por completo de forma independiente, sin depender de Lexmount Browser.**

## Control de costes

En Moli, las operaciones costosas del navegador deben activarse explícitamente y nunca están habilitadas de forma predeterminada:

| Modo u opción | Comportamiento |
| --- | --- |
| Predeterminado | `LayoutPolicy::Mock` — devuelve geometría determinista y compatible con el formato esperado, sin disposición ni dibujo reales |
| `--layout` | `LayoutPolicy::OnDemand` — ofrece disposición real, geometría, detección por coordenadas, entradas por coordenadas, capturas de pantalla y screencast |
| `--resource` | Obtiene todas las familias opcionales de recursos visuales o multimedia |
| `--image`, `--font`, `--audio`, `--video`, `--media`, `--text-track` | Activa por separado una familia concreta de recursos opcionales |
| `--profile-dir`, `--http-cache-dir`, `--cookie-file` | Activa selectivamente la persistencia que necesita la carga de trabajo |

El resultado de la disposición es una instantánea tomada bajo demanda, no un estado que se mantenga continuamente: la primera solicitud de geometría (inicio en frío) construye una disposición completa a partir del DOM y de los estilos actuales, y solo conserva el `LayoutPassOutput` más reciente. A partir de ese momento, las lecturas de geometría ordinarias pueden reutilizar esa instantánea aunque la página haya cambiado; las capturas de pantalla y los screencasts, en cambio, se reconstruyen siempre y nunca reutilizan resultados antiguos.

## Arquitectura

Moli es un núcleo de navegador independiente, no una envoltura de Chromium. Está construido con Rust, tiene sus propias reglas de propiedad y ciclo de vida, y sus dependencias principales incluyen:

- `libcurl` — transporte de red y entorno de ejecución para varias solicitudes
- `html5ever` — análisis de HTML
- `rusty_v8` / V8 — ejecución de JavaScript
- Servo/Stylo — selectores, cascada y estilos calculados
- Taffy + Parley — disposición de cajas y texto
- AnyRender/Vello CPU, `usvg` y el ecosistema de imágenes de Rust — renderizado por software

El documento y los estilos tienen una única fuente de verdad: la integración del DOM nativo con Stylo. Cada actualización real reconstruye la disposición a partir de esa fuente, convierte el resultado en datos inmutables e independientes del DOM y después descarta el estado temporal producido durante esa pasada de disposición y dibujo. En todo el sistema no hay ningún árbol de disposición incremental, grafo de daños, lista de visualización retenida, compositor de GPU ni ventana persistente.

## Datos de las pruebas

Los dos conjuntos de mediciones siguientes muestran el alcance actual de las capacidades de Moli. Las pruebas abarcan sitios web reales, clientes de automatización reales, verificaciones específicas del comportamiento de Chromium/WPT y una gran batería de regresión con nextest.

### Prueba de rastreo mixto de la web pública

La prueba abarca 192 URL públicas de sitios importantes de China y del resto del mundo. Para considerarse correcta, una página debe generar contenido realmente útil después de ejecutar JavaScript: una respuesta HTTP 200, una página de verificación, un muro de inicio de sesión, una respuesta vacía o una interfaz de aplicación que solo contenga su estructura básica no cuentan como resultado satisfactorio.

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
| Tiempo activo del episodio p50 | 33.40 ms | 57.13 ms |
| PSS máximo | 102.46 MiB | 348.82 MiB |
| Máximo de procesos / hilos | 1 / 24 | 11 / 123 |

En la selección actual de pruebas WPT utilizada para validar el alcance funcional de Moli como navegador para agentes, una ejecución completa registró **1 612 000 pruebas superadas**.

## Alcance del proyecto

Dentro de los escenarios de navegador para agentes definidos en la documentación, Moli ya está listo para su uso en producción y continúa en desarrollo activo.

Los límites que se mantienen de forma intencionada incluyen:

- No ofrece un navegador con interfaz gráfica, una ventana persistente ni un compositor de GPU, y tampoco implementa una arquitectura de dibujo multifotograma retenida.
- No busca una representación idéntica píxel a píxel a la de Chrome ni ofrece reproducción de Canvas, WebGL o contenidos multimedia de alta fidelidad.
- Solo cubre parte de las funciones de CDP, WebDriver Classic y WebDriver BiDi, no una implementación con compatibilidad total de los protocolos.
- El modo `--layout` admite capturas de pantalla por software y generación rasterizada de PDF mediante CDP, pero no implementa todos los modos de captura o impresión de Chrome.
- La carga de recursos, la vigencia de la geometría y el coste del renderizado visual siguen siendo opciones de política que deben configurarse explícitamente, y no permanecen activadas de forma predeterminada.

Las rutas de protocolo no compatibles devuelven un error explícito: Moli nunca simula que se haya producido una acción del navegador, un evento, una observación de red o un resultado visual.

Los mantenedores pueden seguir la [guía de publicación](RELEASING.md) para publicar una versión binaria etiquetada mediante GitHub Actions.

## Licencia

Salvo que un archivo o directorio indique lo contrario, Moli puede utilizarse, a elección del usuario, bajo la [Licencia Apache 2.0](LICENSE-APACHE) o la [Licencia MIT](LICENSE-MIT). Los componentes y fixtures de terceros con licencias independientes siguen sujetos a sus respectivas licencias y avisos.
