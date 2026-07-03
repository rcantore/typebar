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
const listen = window.__TAURI__.event.listen;

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

// Snapshot del documento tal como esta en disco (ultimo load/save). Comparado
// contra fullDocument() da el estado "dirty" (cambios sin guardar). Comparar
// strings enteras es barato de sobra para el tamano de un documento de texto,
// asi que lo recalculamos hasta mientras se tipea.
let savedDocument = "";

// Columna objetivo para navegar con flechas entre bloques (punto 1). Igual que
// en la TUI: al subir/bajar se conserva la columna aproximada. null cuando no
// se esta navegando verticalmente.
let goalColumn = null;

// Instruccion de donde dejar el caret en el proximo paint(). null = al final
// (comportamiento por defecto). Tipos:
//   { type: "within", within }              -> offset absoluto dentro del value
//   { type: "edge", edge, goalColumn }       -> "first"/"last" linea, en columna
//   { type: "snippet", fullText, offset }    -> mapear click renderizado a source
let pendingCaret = null;

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

// Dado un array de bloques y un offset absoluto en el documento que forman,
// devuelve el indice del bloque que lo contiene y el offset relativo dentro de
// ese bloque. Generaliza `blockIndexAtOffset` para reubicar el caret tras
// re-segmentar (puntos 1 y 3). Un offset que cae justo en el limite entra al
// ultimo bloque como su posicion final.
function locateOffset(blocksArr, offset) {
  let off = 0;
  for (let k = 0; k < blocksArr.length; k++) {
    const len = blocksArr[k].source.length;
    if (offset < off + len) {
      return { index: k, within: offset - off };
    }
    off += len;
  }
  const last = Math.max(0, blocksArr.length - 1);
  const within = blocksArr.length ? blocksArr[last].source.length : 0;
  return { index: last, within };
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
  ta.addEventListener("input", () => {
    autosize(ta);
    refreshDirty();
    // El gesto de "linea en blanco" puede confirmar y partir el parrafo.
    maybeSplitOnBlankLine(ta);
  });
  ta.addEventListener("blur", onBlur);
  ta.addEventListener("keydown", onTextareaKeydown);
  textareaEl = ta;
  return ta;
}

// A partir de un mousedown sobre un bloque renderizado, calcula { fullText,
// offset }: el texto plano del bloque y el offset del caracter clickeado dentro
// de ese texto. Usa caretRangeFromPoint (WebKit) con caretPositionFromPoint como
// alternativa. Si el navegador no ofrece ninguno o el click no cae sobre un nodo
// de texto, devuelve null y la edicion abre con el caret al final (como antes).
function clickMapFromEvent(el, e) {
  let node = null;
  let nodeOffset = 0;
  if (document.caretRangeFromPoint) {
    const range = document.caretRangeFromPoint(e.clientX, e.clientY);
    if (range) {
      node = range.startContainer;
      nodeOffset = range.startOffset;
    }
  } else if (document.caretPositionFromPoint) {
    const cp = document.caretPositionFromPoint(e.clientX, e.clientY);
    if (cp) {
      node = cp.offsetNode;
      nodeOffset = cp.offset;
    }
  }
  if (!node || node.nodeType !== Node.TEXT_NODE) {
    return null;
  }
  // Offset global dentro del texto plano del bloque: sumamos el largo de los
  // nodos de texto previos hasta el nodo clickeado.
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let offset = 0;
  let n;
  let found = false;
  while ((n = walker.nextNode())) {
    if (n === node) {
      offset += nodeOffset;
      found = true;
      break;
    }
    offset += n.textContent.length;
  }
  if (!found) {
    return null;
  }
  return { fullText: el.textContent, offset };
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
  // Antes de entrar, calculamos donde cayo el click sobre el texto renderizado
  // para poder posicionar el caret en el punto equivalente del source (punto 2).
  el.addEventListener("mousedown", (e) => {
    e.preventDefault();
    startEdit(i, clickMapFromEvent(el, e));
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
    applyPendingCaret(textareaEl);
  }
  isPainting = false;
}

// Pone el foco en el textarea con el caret al final.
function focusAtEnd(ta) {
  ta.focus();
  const end = ta.value.length;
  ta.setSelectionRange(end, end);
}

// Coloca el caret en el textarea recien pintado segun `pendingCaret` y lo
// consume. Sin instruccion, cae al final (comportamiento historico).
function applyPendingCaret(ta) {
  const pending = pendingCaret;
  pendingCaret = null;
  if (!pending) {
    focusAtEnd(ta);
    return;
  }
  ta.focus();
  if (pending.type === "within") {
    const pos = Math.max(0, Math.min(pending.within, ta.value.length));
    ta.setSelectionRange(pos, pos);
    return;
  }
  if (pending.type === "edge") {
    const pos = caretAtEdge(ta.value, pending.edge, pending.goalColumn);
    ta.setSelectionRange(pos, pos);
    return;
  }
  if (pending.type === "snippet") {
    const pos = caretFromSnippet(ta.value, pending.fullText, pending.offset);
    if (pos === null) {
      focusAtEnd(ta);
    } else {
      ta.setSelectionRange(pos, pos);
    }
    return;
  }
  focusAtEnd(ta);
}

// Offset en `value` para dejar el caret en la primera o ultima linea, en la
// columna objetivo `goalColumn` (recortada al largo de esa linea). "primera" y
// "ultima" se calculan sobre los "\n" del value, NO sobre las lineas visuales
// que produce el word-wrap. Es una aproximacion aceptable para el spike: al
// saltar entre bloques cortos coincide, y en parrafos largos wrapeados el salto
// cae en el extremo logico de la linea, no en la fila visual exacta.
function caretAtEdge(value, edge, goalColumn) {
  const col = goalColumn ?? 0;
  if (edge === "first") {
    const lineEnd = value.indexOf("\n");
    const len = lineEnd === -1 ? value.length : lineEnd;
    return Math.min(col, len);
  }
  // Ultima linea: arranca despues del ultimo "\n".
  const lineStart = value.lastIndexOf("\n") + 1;
  const len = value.length - lineStart;
  return lineStart + Math.min(col, len);
}

// Mapea un click sobre el HTML renderizado a una posicion en el `source`
// markdown (punto 2). `fullText` es el textContent del bloque renderizado y
// `offset` el punto clickeado dentro de ese texto plano. Tomamos una ventana de
// contexto alrededor del click y la buscamos en el source; como el render quita
// sintaxis (##, **, backticks, marcadores de lista), el texto plano coincide con
// el source SOLO en la prosa. Por eso probamos ventanas cada vez mas chicas y,
// si ninguna matchea (el click cayo sobre sintaxis transformada, p.ej. el texto
// visible de un link cuya URL no esta en el render), devolvemos null para caer
// con gracia al final del bloque. Limite conocido: si la ventana aparece mas de
// una vez en el source, tomamos la primera ocurrencia (aceptable para el spike).
function caretFromSnippet(source, fullText, offset) {
  for (const radius of [8, 5, 3]) {
    const start = Math.max(0, offset - radius);
    const end = Math.min(fullText.length, offset + radius);
    const snippet = fullText.slice(start, end);
    if (snippet.length < 3) {
      continue;
    }
    const idx = source.indexOf(snippet);
    if (idx !== -1) {
      // Reconstruimos la posicion exacta sumando cuanto adentro del snippet
      // habia caido el click.
      return idx + (offset - start);
    }
  }
  return null;
}

// --- Transiciones de edicion ---------------------------------------------

// Confirma la edicion actual: junta el documento, lo re-segmenta con el nucleo
// y vuelve a renderizar todo.
async function commitEdit() {
  if (editingIndex === null) {
    return;
  }
  await setDocumentFromMarkdown(fullDocument());
  refreshDirty();
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
// re-segmentacion puede haber cambiado los indices). `clickMap`, si viene, lleva
// { fullText, offset } para mapear el punto clickeado del render al source
// (punto 2); si no, el caret cae al final como antes.
async function startEdit(i, clickMap) {
  // Un click arranca navegacion desde cero: se olvida la columna objetivo.
  goalColumn = null;
  if (clickMap) {
    pendingCaret = {
      type: "snippet",
      fullText: clickMap.fullText,
      offset: clickMap.offset,
    };
  }
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

// Teclas dentro del textarea del bloque en edicion.
function onTextareaKeydown(e) {
  const ta = e.target;
  if (e.key === "Escape") {
    e.preventDefault();
    goalColumn = null;
    commitEdit();
    return;
  }
  if (e.key === "ArrowUp" || e.key === "ArrowDown") {
    const dir = e.key === "ArrowUp" ? -1 : 1;
    const pos = ta.selectionStart;
    const value = ta.value;
    const before = value.slice(0, pos);
    // "primera/ultima linea" se calcula sobre los "\n" del value, no sobre las
    // filas visuales del word-wrap (aceptable para el spike).
    const onFirstLine = !before.includes("\n");
    const onLastLine = !value.slice(pos).includes("\n");
    const atEdge = dir < 0 ? onFirstLine : onLastLine;
    if (!atEdge) {
      // Movimiento vertical normal dentro del textarea: WebKit maneja su propia
      // columna; reiniciamos la nuestra para que la proxima cadena de saltos
      // arranque fresca.
      goalColumn = null;
      return;
    }
    e.preventDefault();
    const col = pos - (before.lastIndexOf("\n") + 1);
    // Conservamos la columna objetivo entre saltos consecutivos (como la TUI).
    if (goalColumn === null) {
      goalColumn = col;
    }
    navigateToAdjacentBlock(dir);
    return;
  }
  // Cualquier otra tecla (tipear, flechas horizontales) rompe la cadena de
  // navegacion vertical.
  goalColumn = null;
}

// Confirma el bloque en edicion y abre el adyacente (`dir` = -1 arriba, +1
// abajo), dejando el caret en la columna objetivo sobre la linea de contacto
// (ultima del anterior al subir, primera del siguiente al bajar). En el primer
// o ultimo bloque no hace nada. Re-segmenta como `startEdit` porque editar pudo
// cambiar la particion; reubica el indice por offset absoluto.
async function navigateToAdjacentBlock(dir) {
  const target = editingIndex + dir;
  if (target < 0 || target >= blocks.length) {
    return; // extremos: nos quedamos donde estamos.
  }
  const targetOffset = docOffsetOfBlockStart(target);
  blocks = await invoke("render_blocks", { markdown: fullDocument() });
  editingIndex = blockIndexAtOffset(targetOffset);
  ensureNonEmpty();
  pendingCaret = {
    type: "edge",
    edge: dir < 0 ? "last" : "first",
    goalColumn,
  };
  paint();
}

// Detecta el gesto universal de "nuevo parrafo": el caret quedo en una linea en
// blanco recien formada (doble Enter). Confirma, re-segmenta y sigue editando el
// bloque donde cayo el caret, sin pasar por el mouse ni Escape (punto 3).
//
// Fences: una linea en blanco DENTRO de un code fence abierto no parte el bloque
// (el nucleo lo mantiene entero). Lo detectamos sin logica de markdown: si tras
// re-segmentar el value entero sigue siendo UN solo bloque identico, no hubo
// split y seguimos editando sin repintar (sin parpadeo).
async function maybeSplitOnBlankLine(ta) {
  const pos = ta.selectionStart;
  const value = ta.value;
  // Patron del doble Enter: el caret viene precedido por dos "\n" (linea en
  // blanco). Cubre el final de parrafo y tambien un corte en el medio.
  if (pos < 2 || value[pos - 1] !== "\n" || value[pos - 2] !== "\n") {
    return;
  }
  // Offset absoluto del caret en el documento completo (con el textarea vivo).
  const absOffset = docOffsetOfBlockStart(editingIndex) + pos;
  const newBlocks = await invoke("render_blocks", { markdown: fullDocument() });
  const loc = locateOffset(newBlocks, absOffset);
  // Si el bloque que contiene el caret es identico al value actual, NO hubo
  // corte estructural (linea en blanco final a la espera del proximo parrafo, o
  // fence abierto): seguimos editando el mismo textarea sin repintar.
  if (newBlocks[loc.index] && newBlocks[loc.index].source === value) {
    return;
  }
  // Hubo split: adoptamos la nueva segmentacion y seguimos editando el bloque
  // donde quedo el caret, en la posicion correspondiente.
  blocks = newBlocks;
  editingIndex = loc.index;
  ensureNonEmpty();
  pendingCaret = { type: "within", within: loc.within };
  paint();
  refreshDirty();
}

// Click en el espacio vacio bajo el ultimo bloque: agrega un parrafo nuevo al
// final y lo pone en edicion.
async function appendBlockAndEdit() {
  goalColumn = null;
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

// --- Estado del documento (dirty + titulo) --------------------------------

// Nombre de archivo a partir de una ruta absoluta (separadores unix o windows).
function basename(path) {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

// Documento con cambios sin guardar respecto del ultimo load/save.
function isDirty() {
  return fullDocument() !== savedDocument;
}

// Ultimo estado dirty informado al backend y ultimo titulo pintado, para no
// repetir IPC en cada tecla.
let lastDirtyReported = null;
let lastTitleRendered = null;

// Rearma el titulo de la ventana segun el estado: "nombre.md" con archivo,
// "typebar" sin archivo, con "● " adelante si hay cambios sin guardar. Solo hace
// IPC si el titulo efectivamente cambio (cambia por dirty o por nombre de
// archivo), para no parpadear ni llamar de mas.
function updateTitle() {
  const name = currentPath ? basename(currentPath) : "typebar";
  const title = (isDirty() ? "● " : "") + name;
  if (title === lastTitleRendered) {
    return;
  }
  lastTitleRendered = title;
  invoke("update_title", { title }).catch(() => {});
}

// Recalcula el estado dirty y refresca el titulo. Comparar dos strings es
// barato, asi que esto puede llamarse hasta en cada tecla; el estado dirty solo
// se informa al backend cuando cambia (lo usa el guard de cierre).
function refreshDirty() {
  const dirty = isDirty();
  if (dirty !== lastDirtyReported) {
    lastDirtyReported = dirty;
    invoke("set_dirty", { dirty }).catch(() => {});
  }
  updateTitle();
}

// Marca el documento como guardado: toma como base el contenido `contents` (lo
// que quedo en disco) y refresca el estado.
function markSaved(contents) {
  savedDocument = contents;
  refreshDirty();
}

// --- Archivo --------------------------------------------------------------

// Actualiza el rotulo de la ruta del documento en la barra superior.
function setDocPath(path) {
  currentPath = path;
  docPathEl.textContent = path || "documento sin guardar";
  docPathEl.title = path || "";
}

// Si hay cambios sin guardar, pregunta por dialogo nativo si descartarlos.
// Devuelve true si se puede seguir (no habia cambios o el usuario descarto).
async function confirmDiscardIfDirty(message) {
  if (!isDirty()) {
    return true;
  }
  try {
    return await invoke("confirm_discard", { message });
  } catch (err) {
    // Ante un fallo del dialogo, preferimos no perder trabajo: cancelamos.
    return false;
  }
}

// Abrir: si hay cambios sin guardar pregunta primero (guard de Open); luego
// dialogo nativo, lectura por el backend y carga como bloques.
async function openFile() {
  try {
    const ok = await confirmDiscardIfDirty(
      "Vas a abrir otro archivo y perderas los cambios sin guardar. ¿Descartarlos?",
    );
    if (!ok) {
      return; // el usuario prefirio no perder los cambios
    }
    const path = await invoke("pick_open_path");
    if (!path) {
      return; // el usuario cancelo
    }
    const contents = await invoke("load_file", { path });
    await setDocumentFromMarkdown(contents);
    setDocPath(path);
    markSaved(contents);
  } catch (err) {
    alert("No se pudo abrir el archivo: " + err);
  }
}

// Guarda en `path` el documento completo (incluye el bloque en edicion, si hay).
async function writeTo(path) {
  const contents = fullDocument();
  await invoke("save_file", { path, contents });
  setDocPath(path);
  markSaved(contents);
}

// Save: reusa la ruta actual; si no hay, cae en "Save as". Devuelve true si el
// documento quedo guardado (lo usa el flujo "Guardar y cerrar").
async function saveFile() {
  try {
    if (!currentPath) {
      return await saveFileAs();
    }
    await writeTo(currentPath);
    return true;
  } catch (err) {
    alert("No se pudo guardar: " + err);
    return false;
  }
}

// Save as: siempre pide una ruta nueva por el dialogo nativo. Devuelve true si
// se guardo, false si el usuario cancelo o hubo error.
async function saveFileAs() {
  try {
    const path = await invoke("pick_save_path");
    if (!path) {
      return false; // el usuario cancelo
    }
    await writeTo(path);
    return true;
  } catch (err) {
    alert("No se pudo guardar: " + err);
    return false;
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

// Flujo "Guardar y cerrar": el guard de cierre en Rust emite este evento cuando
// el usuario elige "Guardar" en el dialogo de salida. Guardamos y, solo si lo
// logramos, terminamos de cerrar la ventana (close_window). Si el guardado se
// cancela, la ventana queda abierta con el trabajo intacto.
listen("typebar://save-and-close", async () => {
  const saved = await saveFile();
  if (saved) {
    invoke("close_window").catch(() => {});
  }
});

// Estado inicial: documento de bienvenida. Fijamos la base "guardada" en el
// mismo contenido para arrancar sin cambios pendientes (titulo "typebar").
(async () => {
  await setDocumentFromMarkdown(WELCOME);
  markSaved(WELCOME);
})();
