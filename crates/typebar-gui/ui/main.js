// Frontend del spike de la GUI de typebar. JavaScript vanilla, sin frameworks
// ni bundlers. Habla con el backend Rust por IPC via el global window.__TAURI__
// (habilitado por withGlobalTauri en tauri.conf.json).
//
// Responsabilidades: mantener sincronizados el textarea de la fuente y el panel
// de preview (el HTML lo genera el nucleo, no este archivo), y disparar los
// dialogos nativos de abrir y guardar.

const invoke = window.__TAURI__.core.invoke;

const sourceEl = document.getElementById("source");
const previewEl = document.getElementById("preview");
const docPathEl = document.getElementById("doc-path");
const btnOpen = document.getElementById("btn-open");
const btnSave = document.getElementById("btn-save");
const btnSaveAs = document.getElementById("btn-save-as");

// Ruta del documento en disco, o null si todavia no se guardo.
let currentPath = null;

// Documento de bienvenida por defecto (markdown embebido).
const WELCOME = `# typebar

Editor de Markdown **WYSIWYG**, ahora tambien como app de escritorio.
Este es el *spike* inicial de la GUI en Tauri v2 sobre \`typebar-core\`.

## Como funciona

A la izquierda escribis el markdown crudo en monospace. A la derecha ves el
preview en vivo: el HTML lo genera el nucleo de typebar por IPC, no el
navegador.

### Probalo

- Escribi en el panel izquierdo y mira el preview actualizarse.
- Usa **Open** para cargar un \`.md\` desde disco.
- Usa **Save** para guardarlo.

> La estetica es tinta sobre papel, como el modo whitepaper de la terminal.
`;

// Extrae el contenido del <body> del documento HTML que devuelve el nucleo.
// Asi reusamos el render del core pero aplicamos la estetica del preview.
// DOMParser no ejecuta scripts, y innerHTML tampoco los corre al insertarlos.
function extractBody(fullHtml) {
  const doc = new DOMParser().parseFromString(fullHtml, "text/html");
  return doc.body ? doc.body.innerHTML : "";
}

// Pide al backend el HTML del markdown actual y lo pinta en el preview.
async function renderPreview() {
  const markdown = sourceEl.value;
  try {
    const html = await invoke("render_html", { markdown });
    previewEl.innerHTML = extractBody(html);
  } catch (err) {
    previewEl.textContent = "Error al renderizar: " + err;
  }
}

// Debounce del render para no llamar al backend en cada pulsacion.
let renderTimer = null;
function scheduleRender() {
  if (renderTimer !== null) {
    clearTimeout(renderTimer);
  }
  renderTimer = setTimeout(renderPreview, 200);
}

// Actualiza el rotulo de la ruta del documento en la barra superior.
function setDocPath(path) {
  currentPath = path;
  docPathEl.textContent = path || "documento sin guardar";
  docPathEl.title = path || "";
}

// Reemplaza el contenido de la fuente y refresca el preview de inmediato.
function setSource(text) {
  sourceEl.value = text;
  renderPreview();
}

// Abrir: dialogo nativo, lectura por el backend y volcado a la fuente.
async function openFile() {
  try {
    const path = await invoke("pick_open_path");
    if (!path) {
      return; // el usuario cancelo
    }
    const contents = await invoke("load_file", { path });
    setSource(contents);
    setDocPath(path);
  } catch (err) {
    alert("No se pudo abrir el archivo: " + err);
  }
}

// Guarda en `path` el contenido actual de la fuente.
async function writeTo(path) {
  await invoke("save_file", { path, contents: sourceEl.value });
  setDocPath(path);
}

// Save: reusa la ruta actual; si no hay, cae en "Save as".
async function saveFile() {
  try {
    if (!currentPath) {
      await saveFileAs();
      return;
    }
    await writeTo(currentPath);
  } catch (err) {
    alert("No se pudo guardar: " + err);
  }
}

// Save as: siempre pide una ruta nueva por el dialogo nativo.
async function saveFileAs() {
  try {
    const path = await invoke("pick_save_path");
    if (!path) {
      return; // el usuario cancelo
    }
    await writeTo(path);
  } catch (err) {
    alert("No se pudo guardar: " + err);
  }
}

// Cableado de eventos.
sourceEl.addEventListener("input", scheduleRender);
btnOpen.addEventListener("click", openFile);
btnSave.addEventListener("click", saveFile);
btnSaveAs.addEventListener("click", saveFileAs);

// Estado inicial: documento de bienvenida.
setSource(WELCOME);
