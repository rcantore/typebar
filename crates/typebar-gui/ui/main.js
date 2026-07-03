// Frontend del segundo spike de la GUI de typebar: editor WYSIWYG por bloques
// estilo Typora. JavaScript vanilla, sin frameworks ni bundlers. Habla con el
// backend Rust por IPC via el global window.__TAURI__ (habilitado por
// withGlobalTauri en tauri.conf.json).
//
// Modelo (decidido, no negociable): el markdown es la UNICA fuente de verdad.
// Nada de contenteditable libre ni de convertir HTML de vuelta a markdown. El
// documento se muestra como una columna de bloques de nivel superior:
//   - Los bloques SIN foco se ven renderizados (HTML que genera el nucleo).
//   - El bloque CON foco se swapea in-place por un <textarea> con su source
//     markdown crudo, en monospace y autoajustado en altura.
// Al sacar el foco (blur, Escape, click en otro bloque) se junta el documento
// entero, se re-parsea con el nucleo (render_blocks) y se repinta. Se re-parsea
// TODO a proposito: editar un bloque puede cambiar la segmentacion (p.ej. una
// linea en blanco parte un parrafo en dos) y asi el estado queda consistente.
//
// Es el mismo concepto del "Nivel 2" de la TUI (la linea activa se ve cruda y
// el resto contraido/renderizado), subido de linea a bloque. Ese paralelismo es
// identidad del producto.
//
// El estado en JS es minimo: el array de bloques que devolvio el nucleo mas el
// indice del bloque en edicion. Nada de logica de markdown en este archivo.

const invoke = window.__TAURI__.core.invoke;

const docEl = document.getElementById("doc");
const docPathEl = document.getElementById("doc-path");
const btnOpen = document.getElementById("btn-open");
const btnSave = document.getElementById("btn-save");
const btnSaveAs = document.getElementById("btn-save-as");

// --- Estado ---------------------------------------------------------------

// Array de bloques { source, html } tal como los devolvio el nucleo.
let blocks = [];
// Indice del bloque en edicion, o null si ninguno tiene el foco.
let editingIndex = null;
// Referencia al <textarea> del bloque en edicion (vive solo mientras se edita).
let textareaEl = null;
// Ruta del documento en disco, o null si todavia no se guardo.
let currentPath = null;
// Guarda para ignorar el blur que dispara quitar el textarea del DOM al repintar.
let isPainting = false;

// Documento de bienvenida por defecto (markdown embebido).
const WELCOME = `# typebar

Editor de Markdown **WYSIWYG por bloques**, ahora como app de escritorio.
Este es el segundo *spike* de la GUI en Tauri v2 sobre \`typebar-core\`.

## Como funciona

El documento se ve como una columna de bloques renderizados. Al hacer click en
un bloque, se convierte en su *source* markdown crudo para editarlo. Al salir,
vuelve a renderizarse.

- Hace click en este bloque para editarlo.
- Usa **Open** para cargar un \`.md\` desde disco.
- Usa **Save** (o Cmd/Ctrl+S) para guardarlo.

> La estetica es tinta sobre papel, como el modo whitepaper de la terminal.
`;

// --- Helpers de documento -------------------------------------------------

// Source vigente del bloque `i`: si es el que se esta editando, lo toma del
// textarea (valor en vivo); si no, del array.
function sourceOf(i) {
  if (i === editingIndex && textareaEl) {
    return textareaEl.value;
  }
  return blocks[i].source;
}

// Junta el documento completo a partir de los sources de todos los bloques,
// con el bloque en edicion reemplazado por el contenido actual del textarea.
// La concatenacion reconstruye el documento tal como lo veria el disco.
function fullDocument() {
  let out = "";
  for (let i = 0; i < blocks.length; i++) {
    out += sourceOf(i);
  }
  return out;
}

// Offset en bytes-de-string (chars JS) donde arranca el bloque `i` en el
// documento que resultaria de confirmar la edicion actual. Sirve para reubicar
// el foco despues de re-segmentar.
function docOffsetOfBlockStart(i) {
  let off = 0;
  for (let k = 0; k < i; k++) {
    off += sourceOf(k).length;
  }
  return off;
}

// Indice del bloque (en el array `blocks` actual) que contiene `offset`.
function blockIndexAtOffset(offset) {
  let off = 0;
  for (let k = 0; k < blocks.length; k++) {
    const len = blocks[k].source.length;
    if (offset < off + len) {
      return k;
    }
    off += len;
  }
  return Math.max(0, blocks.length - 1);
}

// Un documento vacio se muestra como un unico bloque editable en blanco, para
// poder empezar a escribir enseguida.
function ensureNonEmpty() {
  if (blocks.length === 0) {
    blocks = [{ source: "", html: "" }];
    editingIndex = 0;
  }
}

// Re-parsea `markdown` con el nucleo y adopta la nueva segmentacion como estado.
async function setDocumentFromMarkdown(markdown) {
  blocks = await invoke("render_blocks", { markdown });
  editingIndex = null;
  ensureNonEmpty();
  paint();
}

// --- Render / pintado -----------------------------------------------------

// Ajusta la altura del textarea a su contenido (sin barra de scroll interna).
function autosize(ta) {
  ta.style.height = "auto";
  ta.style.height = ta.scrollHeight + "px";
}

// Crea el <textarea> con el source crudo del bloque `i` y lo cablea.
function makeTextarea(i) {
  const ta = document.createElement("textarea");
  ta.className = "block-source";
  ta.value = blocks[i].source;
  ta.spellcheck = false;
  ta.setAttribute("autocomplete", "off");
  ta.setAttribute("autocapitalize", "off");
  ta.addEventListener("input", () => autosize(ta));
  ta.addEventListener("blur", onBlur);
  ta.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      commitEdit();
    }
  });
  textareaEl = ta;
  return ta;
}

// Crea el bloque renderizado (solo lectura) del bloque `i`. El HTML lo genero
// el nucleo; aca solo lo insertamos y cableamos el click para entrar a editar.
function makeRendered(i) {
  const el = document.createElement("div");
  el.className = "block";
  el.innerHTML = blocks[i].html || "";
  if (!blocks[i].html) {
    // Bloque sin render (p.ej. solo espacios en blanco): lo dejamos clickeable.
    el.classList.add("block-empty");
  }
  // mousedown con preventDefault: entra a editar sin robar el foco de forma que
  // dispare el blur del textarea anterior (la transicion la maneja startEdit).
  el.addEventListener("mousedown", (e) => {
    e.preventDefault();
    startEdit(i);
  });
  // Evita que un enlace del render navegue: aca solo se edita el source.
  el.addEventListener("click", (e) => e.preventDefault());
  return el;
}

// Repinta la columna entera desde `blocks` y `editingIndex`.
function paint() {
  isPainting = true;
  // Vaciar remueve el textarea con foco y dispara su blur, que ignoramos por
  // isPainting para no re-confirmar en medio del repintado.
  docEl.innerHTML = "";
  textareaEl = null;

  for (let i = 0; i < blocks.length; i++) {
    if (i === editingIndex) {
      docEl.appendChild(makeTextarea(i));
    } else {
      docEl.appendChild(makeRendered(i));
    }
  }

  if (textareaEl) {
    autosize(textareaEl);
    focusAtEnd(textareaEl);
  }
  isPainting = false;
}

// Pone el foco en el textarea con el caret al final.
function focusAtEnd(ta) {
  ta.focus();
  const end = ta.value.length;
  ta.setSelectionRange(end, end);
}

// --- Transiciones de edicion ---------------------------------------------

// Confirma la edicion actual: junta el documento, lo re-segmenta con el nucleo
// y vuelve a renderizar todo.
async function commitEdit() {
  if (editingIndex === null) {
    return;
  }
  await setDocumentFromMarkdown(fullDocument());
}

// Blur del textarea: confirma, salvo que el blur venga de repintar.
function onBlur() {
  if (isPainting) {
    return;
  }
  commitEdit();
}

// Entra a editar el bloque `i`. Si ya se estaba editando otro, lo confirma
// primero y reubica el foco al bloque que quedo en la misma posicion (la
// re-segmentacion puede haber cambiado los indices).
async function startEdit(i) {
  if (editingIndex === i) {
    return;
  }
  if (editingIndex === null) {
    editingIndex = i;
    paint();
    return;
  }
  // Offset donde arranca el bloque objetivo en el documento ya confirmado.
  const targetOffset = docOffsetOfBlockStart(i);
  blocks = await invoke("render_blocks", { markdown: fullDocument() });
  editingIndex = blockIndexAtOffset(targetOffset);
  ensureNonEmpty();
  paint();
}

// Click en el espacio vacio bajo el ultimo bloque: agrega un parrafo nuevo al
// final y lo pone en edicion.
async function appendBlockAndEdit() {
  if (editingIndex !== null) {
    blocks = await invoke("render_blocks", { markdown: fullDocument() });
    editingIndex = null;
    ensureNonEmpty();
  }
  const last = blocks[blocks.length - 1];
  if (last && last.source.trim() === "") {
    // Ya hay un bloque vacio al final: editamos ese en vez de agregar otro.
    editingIndex = blocks.length - 1;
    paint();
    return;
  }
  if (last && !last.source.endsWith("\n\n")) {
    // Garantiza una linea en blanco de separacion para que el texto nuevo sea
    // un parrafo aparte y no se pegue al bloque anterior.
    last.source = last.source.replace(/\n*$/, "") + "\n\n";
  }
  blocks.push({ source: "", html: "" });
  editingIndex = blocks.length - 1;
  paint();
}

// Click en la zona vacia del contenedor (no sobre un bloque) -> bloque nuevo.
docEl.addEventListener("mousedown", (e) => {
  if (e.target === docEl) {
    e.preventDefault();
    appendBlockAndEdit();
  }
});

// --- Archivo --------------------------------------------------------------

// Actualiza el rotulo de la ruta del documento en la barra superior.
function setDocPath(path) {
  currentPath = path;
  docPathEl.textContent = path || "documento sin guardar";
  docPathEl.title = path || "";
}

// Abrir: dialogo nativo, lectura por el backend y carga como bloques.
async function openFile() {
  try {
    const path = await invoke("pick_open_path");
    if (!path) {
      return; // el usuario cancelo
    }
    const contents = await invoke("load_file", { path });
    await setDocumentFromMarkdown(contents);
    setDocPath(path);
  } catch (err) {
    alert("No se pudo abrir el archivo: " + err);
  }
}

// Guarda en `path` el documento completo (incluye el bloque en edicion, si hay).
async function writeTo(path) {
  await invoke("save_file", { path, contents: fullDocument() });
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

// --- Cableado -------------------------------------------------------------

btnOpen.addEventListener("click", openFile);
btnSave.addEventListener("click", saveFile);
btnSaveAs.addEventListener("click", saveFileAs);

// Cmd/Ctrl+S guarda sin importar donde este el foco.
document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
    e.preventDefault();
    saveFile();
  }
});

// Estado inicial: documento de bienvenida.
setDocumentFromMarkdown(WELCOME);
