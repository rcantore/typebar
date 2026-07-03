// Frontend del cuarto spike de la GUI de typebar: editor WYSIWYG por bloques
// estilo Typora. JavaScript vanilla, sin frameworks ni bundlers. Habla con el
// backend Rust por IPC via el global window.__TAURI__ (habilitado por
// withGlobalTauri en tauri.conf.json).
//
// Modelo (decidido, no negociable): el markdown es la UNICA fuente de verdad.
// Nada de contenteditable libre ni de convertir HTML de vuelta a markdown. El
// documento se muestra como una columna de bloques de nivel superior:
//   - Los bloques SIN foco se ven renderizados (HTML que genera el nucleo).
//   - El bloque CON foco se swapea in-place por una superficie editable
//     (<div contenteditable>) cuyo textContent ES el source markdown crudo, byte
//     a byte, pero PINTADO: los marcadores (`**`, `#`, `-`, `>`, backticks)
//     quedan visibles pero atenuados y el contenido se estiliza (bold en bold,
//     heading a su escala, code en mono). El estilado es solo pintura encima del
//     texto; el textContent nunca deja de ser el markdown exacto.
// Al sacar el foco (blur, Escape, click en otro bloque) se junta el documento
// entero, se re-parsea con el nucleo (render_blocks) y se repinta. Se re-parsea
// TODO a proposito: editar un bloque puede cambiar la segmentacion (p.ej. una
// linea en blanco parte un parrafo en dos) y asi el estado queda consistente.
//
// Esto es el "Nivel 1" de la TUI (los marcadores nunca se ocultan, solo se
// atenuan) aplicado al bloque activo, sobre el "Nivel 2" por bloque (el resto
// contraido/renderizado). Ese paralelismo es identidad del producto.
//
// El estilado de los tramos lo calcula el nucleo (`style_spans`): aca no hay
// nada de logica de markdown, solo pintura de spans y manejo del caret.

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
// Referencia al <div contenteditable> del bloque en edicion (la superficie
// editable estilizada). Vive solo mientras se edita.
let editorEl = null;
// Ruta del documento en disco, o null si todavia no se guardo.
let currentPath = null;
// Guarda para ignorar el blur que dispara quitar el editor del DOM al repintar.
let isPainting = false;
// True mientras hay una composicion IME activa (teclas muertas opcion-e + e,
// CJK, etc). Durante la composicion NO repintamos ni tocamos el DOM: hacerlo
// aborta la composicion. Repintamos al terminar (compositionend).
let isComposing = false;

// Snapshot del documento tal como esta en disco (ultimo load/save). Comparado
// contra fullDocument() da el estado "dirty" (cambios sin guardar). Comparar
// strings enteras es barato de sobra para el tamano de un documento de texto,
// asi que lo recalculamos hasta mientras se tipea.
let savedDocument = "";

// Columna objetivo para navegar con flechas entre bloques. Igual que en la TUI:
// al subir/bajar se conserva la columna aproximada. null cuando no se esta
// navegando verticalmente.
let goalColumn = null;

// Instruccion de donde dejar el caret en el proximo paint(). null = al final
// (comportamiento por defecto). Tipos:
//   { type: "within", within }              -> offset absoluto de texto
//   { type: "edge", edge, goalColumn }       -> "first"/"last" linea, en columna
//   { type: "snippet", fullText, offset }    -> mapear click renderizado a source
let pendingCaret = null;

// Token monotono para descartar repintados de spans obsoletos: cada pedido a
// style_spans es async, y entre el pedido y la respuesta el usuario pudo seguir
// tipeando. Solo aplicamos la respuesta si el token sigue vigente.
let repaintToken = 0;
// Timer del repintado con debounce corto (evita pedir spans en cada tecla).
let repaintTimer = null;
const REPAINT_DEBOUNCE_MS = 45;

// --- Undo por sesion de edicion -------------------------------------------
//
// Reconstruir el innerHTML del editor en cada repintado rompe el undo nativo de
// contenteditable (el navegador pierde su historial). Mantenemos un mini stack
// propio de snapshots { text, caret } por SESION de edicion (se resetea al
// entrar a un bloque). Coalescing simple: las rafagas de tipeo seguidas colapsan
// en un solo paso. El undo cross-block (deshacer a traves de la re-segmentacion)
// queda para mas adelante: aca el alcance es el bloque en edicion.
let undoStack = [];
let redoStack = [];
let lastRecordTime = 0;
const UNDO_COALESCE_MS = 500;

// Documento de bienvenida por defecto (markdown embebido).
const WELCOME = `# typebar

Editor de Markdown **WYSIWYG por bloques**, ahora como app de escritorio.
Este es el cuarto *spike* de la GUI en Tauri v2 sobre \`typebar-core\`.

## Como funciona

El documento se ve como una columna de bloques renderizados. Al hacer click en
un bloque, se convierte en su *source* markdown crudo pero **estilizado**: los
marcadores quedan a la vista, atenuados, y escribis sobre el documento.

- Hace click en este bloque para editarlo.
- Usa **Open** para cargar un \`.md\` desde disco.
- Usa **Save** (o Cmd/Ctrl+S) para guardarlo.

> La estetica es tinta sobre papel, como el modo whitepaper de la terminal.
`;

// --- Helpers de documento -------------------------------------------------

// Source vigente del bloque `i`: si es el que se esta editando, lo toma del
// editor (textContent en vivo, que ES el markdown crudo); si no, del array.
function sourceOf(i) {
  if (i === editingIndex && editorEl) {
    return editorEl.textContent;
  }
  return blocks[i].source;
}

// Junta el documento completo a partir de los sources de todos los bloques, con
// el bloque en edicion reemplazado por el contenido actual del editor. La
// concatenacion reconstruye el documento tal como lo veria el disco.
function fullDocument() {
  let out = "";
  for (let i = 0; i < blocks.length; i++) {
    out += sourceOf(i);
  }
  return out;
}

// Offset (en unidades de String JS) donde arranca el bloque `i` en el documento
// que resultaria de confirmar la edicion actual. Sirve para reubicar el foco
// despues de re-segmentar.
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
// ese bloque. Un offset que cae justo en el limite entra al ultimo bloque como
// su posicion final.
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

// --- Caret sobre la superficie editable -----------------------------------
//
// En un contenteditable el caret se maneja con Selection/Range, no con
// selectionStart. Trabajamos siempre con un OFFSET de texto absoluto (en
// unidades de String JS, iguales a las UTF-16 que devuelve el nucleo): el largo
// del textContent desde el inicio del editor hasta el caret. Asi el offset
// sobrevive a reconstruir el innerHTML con los spans estilizados.

// Offset de texto del caret (colapsado o extremo final de la seleccion) dentro
// de `root`. null si no hay seleccion dentro del editor. Usa un Range desde el
// inicio del editor hasta el caret y mide su longitud de texto, que es robusto
// ante el arbol de spans (el texto de los `\n` cuenta como un char, gracias a
// white-space: pre-wrap).
function getCaretOffset(root) {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) {
    return null;
  }
  const range = sel.getRangeAt(0);
  if (!root.contains(range.endContainer)) {
    return null;
  }
  const pre = range.cloneRange();
  pre.selectNodeContents(root);
  pre.setEnd(range.endContainer, range.endOffset);
  return pre.toString().length;
}

// Coloca el caret colapsado en el offset de texto `offset` dentro de `root`.
// Recorre los nodos de texto sumando largos hasta encontrar el que contiene el
// offset. Si se pasa del final, cae al final del editor.
function setCaretOffset(root, offset) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let remaining = offset;
  let node = null;
  let n;
  while ((n = walker.nextNode())) {
    const len = n.textContent.length;
    if (remaining <= len) {
      node = n;
      break;
    }
    remaining -= len;
  }
  const sel = window.getSelection();
  const range = document.createRange();
  if (node) {
    range.setStart(node, remaining);
    range.collapse(true);
  } else {
    // Sin nodos de texto (editor vacio) u offset pasado del final: al final.
    range.selectNodeContents(root);
    range.collapse(false);
  }
  sel.removeAllRanges();
  sel.addRange(range);
}

// --- Render / pintado -----------------------------------------------------

// Nivel de heading (1..6) de un source de bloque, o 0 si no es un heading ATX.
// Sirve para escalar la superficie editable a la altura del heading, ya que el
// nucleo marca el texto del heading pero el tamano lo decide el frontend.
function headingLevel(source) {
  const m = /^(#{1,6})\s/.exec(source);
  return m ? m[1].length : 0;
}

// Un bloque es un code fence completo si arranca con ``` o ~~~. En ese caso toda
// la superficie va en mono (los fences son la excepcion a la tipografia serif).
function isFenceBlock(source) {
  return /^\s*(```|~~~)/.test(source);
}

// Ajusta la clase de escala de heading del editor segun su contenido actual, en
// vivo (si el usuario agrega/saca `#`, el tamano acompana sin esperar al commit).
function syncEditorScale(div) {
  for (let l = 1; l <= 6; l++) {
    div.classList.remove("block-editor-h" + l);
  }
  const level = headingLevel(div.textContent);
  if (level) {
    div.classList.add("block-editor-h" + level);
  }
}

// Reconstruye el interior del editor como secuencia de <span class="md-<kind>">
// + texto plano a partir de `text` (el source crudo) y `spans` (los tramos de
// estilo del nucleo, en offsets UTF-16 = indices de String JS). Los huecos entre
// tramos son texto plano. El resultado tiene el MISMO textContent que `text`
// (solo cambia la pintura), asi la invariante se mantiene.
function applySpans(div, text, spans) {
  const frag = document.createDocumentFragment();
  let pos = 0;
  for (const sp of spans) {
    // Blindaje: el nucleo entrega tramos ordenados y disjuntos, pero ante
    // cualquier anomalia no queremos duplicar ni cruzar texto.
    if (sp.start < pos || sp.end <= sp.start || sp.end > text.length) {
      continue;
    }
    if (sp.start > pos) {
      frag.appendChild(document.createTextNode(text.slice(pos, sp.start)));
    }
    const el = document.createElement("span");
    el.className = "md-" + sp.kind;
    el.textContent = text.slice(sp.start, sp.end);
    frag.appendChild(el);
    pos = sp.end;
  }
  if (pos < text.length) {
    frag.appendChild(document.createTextNode(text.slice(pos)));
  }
  div.textContent = "";
  div.appendChild(frag);
}

// Pide los spans del contenido actual del editor y repinta, preservando el caret
// por offset absoluto. Es el corazon del "Nivel 1" del bloque activo. Async
// (style_spans es IPC); descarta la respuesta si el editor cambio o el usuario
// siguio tipeando (token/textContent), o si hay una composicion IME en curso.
async function repaintEditor() {
  const div = editorEl;
  if (!div) {
    return;
  }
  if (isComposing) {
    return; // no tocar el DOM en medio de una composicion IME.
  }
  const text = div.textContent;
  const token = ++repaintToken;
  let spans;
  try {
    spans = await invoke("style_spans", { source: text });
  } catch {
    return;
  }
  // Descartar si algo cambio entre el pedido y la respuesta.
  if (token !== repaintToken || editorEl !== div || isComposing) {
    return;
  }
  if (div.textContent !== text) {
    return;
  }
  const caret = getCaretOffset(div);
  syncEditorScale(div);
  applySpans(div, text, spans);
  if (caret !== null) {
    setCaretOffset(div, caret);
  }
}

// Programa un repintado de spans con debounce corto (no en cada tecla).
function scheduleRepaint() {
  if (repaintTimer) {
    clearTimeout(repaintTimer);
  }
  repaintTimer = setTimeout(() => {
    repaintTimer = null;
    repaintEditor();
  }, REPAINT_DEBOUNCE_MS);
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

// Crea la superficie editable (<div contenteditable>) del bloque `i`, con su
// source markdown crudo como textContent, y la cablea. El pintado de spans se
// dispara desde paint() una vez insertada y con el foco puesto.
function makeEditor(i) {
  const div = document.createElement("div");
  div.className = "block-editor";
  div.setAttribute("contenteditable", "true");
  div.spellcheck = false;
  div.setAttribute("autocorrect", "off");
  div.setAttribute("autocapitalize", "off");
  // El textContent ES el source crudo, byte a byte (invariante sagrada).
  div.textContent = blocks[i].source;
  if (isFenceBlock(blocks[i].source)) {
    div.classList.add("block-editor-code");
  }
  syncEditorScale(div);
  div.addEventListener("beforeinput", onBeforeInput);
  div.addEventListener("input", onEditorInput);
  div.addEventListener("keydown", onEditorKeydown);
  div.addEventListener("blur", onBlur);
  div.addEventListener("paste", onPaste);
  div.addEventListener("drop", onDrop);
  div.addEventListener("dragover", (e) => e.preventDefault());
  div.addEventListener("compositionstart", () => {
    isComposing = true;
  });
  div.addEventListener("compositionend", () => {
    isComposing = false;
    // Al cerrar la composicion recien registramos undo y repintamos.
    onEditorInput();
  });
  editorEl = div;
  return div;
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
  // dispare el blur del editor anterior (la transicion la maneja startEdit).
  // Antes de entrar, calculamos donde cayo el click sobre el texto renderizado
  // para poder posicionar el caret en el punto equivalente del source.
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
  // Vaciar remueve el editor con foco y dispara su blur, que ignoramos por
  // isPainting para no re-confirmar en medio del repintado.
  docEl.innerHTML = "";
  editorEl = null;

  for (let i = 0; i < blocks.length; i++) {
    if (i === editingIndex) {
      docEl.appendChild(makeEditor(i));
    } else {
      docEl.appendChild(makeRendered(i));
    }
  }

  if (editorEl) {
    // Foco + caret sobre el texto plano (todavia sin spans), reset del undo de
    // la nueva sesion de edicion, y primer pintado de spans (async).
    applyPendingCaret(editorEl);
    undoReset(editorEl);
    repaintEditor();
  }
  isPainting = false;
}

// Coloca el caret en el editor recien pintado segun `pendingCaret` y lo consume.
// Sin instruccion, cae al final (comportamiento historico). Pone el foco.
function applyPendingCaret(div) {
  const pending = pendingCaret;
  pendingCaret = null;
  div.focus();
  const text = div.textContent;
  if (!pending) {
    setCaretOffset(div, text.length);
    return;
  }
  if (pending.type === "within") {
    setCaretOffset(div, Math.max(0, Math.min(pending.within, text.length)));
    return;
  }
  if (pending.type === "edge") {
    setCaretOffset(div, caretAtEdge(text, pending.edge, pending.goalColumn));
    return;
  }
  if (pending.type === "snippet") {
    const pos = caretFromSnippet(text, pending.fullText, pending.offset);
    setCaretOffset(div, pos === null ? text.length : pos);
    return;
  }
  setCaretOffset(div, text.length);
}

// Offset de texto para dejar el caret en la primera o ultima linea, en la
// columna objetivo `goalColumn` (recortada al largo de esa linea). "primera" y
// "ultima" se calculan sobre los "\n" del texto, NO sobre las lineas visuales
// que produce el word-wrap. Es una aproximacion aceptable para el spike.
function caretAtEdge(text, edge, goalColumn) {
  const col = goalColumn ?? 0;
  if (edge === "first") {
    const lineEnd = text.indexOf("\n");
    const len = lineEnd === -1 ? text.length : lineEnd;
    return Math.min(col, len);
  }
  // Ultima linea: arranca despues del ultimo "\n".
  const lineStart = text.lastIndexOf("\n") + 1;
  const len = text.length - lineStart;
  return lineStart + Math.min(col, len);
}

// Mapea un click sobre el HTML renderizado a una posicion en el `source`
// markdown. `fullText` es el textContent del bloque renderizado y `offset` el
// punto clickeado dentro de ese texto plano. Tomamos una ventana de contexto
// alrededor del click y la buscamos en el source; como el render quita sintaxis
// (##, **, backticks, marcadores de lista), el texto plano coincide con el
// source SOLO en la prosa. Por eso probamos ventanas cada vez mas chicas y, si
// ninguna matchea, devolvemos null para caer con gracia al final del bloque.
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
      return idx + (offset - start);
    }
  }
  return null;
}

// --- Undo / redo del bloque -----------------------------------------------

// Snapshot del estado de edicion: el texto crudo y el offset de caret.
function snapshot(div) {
  return { text: div.textContent, caret: getCaretOffset(div) ?? div.textContent.length };
}

// Reinicia el historial al entrar a un bloque: el estado inicial es la base.
function undoReset(div) {
  undoStack = [snapshot(div)];
  redoStack = [];
  lastRecordTime = 0;
}

// Registra el estado tras un input. Coalescing por pausa: si el ultimo registro
// fue hace poco (rafaga de tipeo), reemplaza el tope en vez de apilar, para que
// toda la rafaga sea un solo paso de undo. Cualquier registro invalida el redo.
function undoRecord(div) {
  const now = Date.now();
  const coalesce = now - lastRecordTime < UNDO_COALESCE_MS;
  lastRecordTime = now;
  const snap = snapshot(div);
  if (coalesce && undoStack.length >= 1) {
    undoStack[undoStack.length - 1] = snap;
  } else {
    undoStack.push(snap);
  }
  redoStack = [];
}

// Restaura un snapshot en el editor: texto plano + caret, y repinta los spans.
function restoreSnapshot(div, snap) {
  div.textContent = snap.text;
  setCaretOffset(div, snap.caret);
  refreshDirty();
  repaintEditor();
}

// Deshace un paso dentro del bloque (Cmd/Ctrl+Z).
function undo(div) {
  if (undoStack.length < 2) {
    return; // solo queda el estado base: nada que deshacer.
  }
  const current = undoStack.pop();
  redoStack.push(current);
  restoreSnapshot(div, undoStack[undoStack.length - 1]);
  lastRecordTime = 0; // el proximo input abre un paso nuevo, no coalesce.
}

// Rehace un paso dentro del bloque (Shift+Cmd/Ctrl+Z o Ctrl+Y).
function redo(div) {
  if (redoStack.length === 0) {
    return;
  }
  const snap = redoStack.pop();
  undoStack.push(snap);
  restoreSnapshot(div, snap);
  lastRecordTime = 0;
}

// --- Entrada de texto plano (contenteditable estricto) --------------------

// Inserta `text` como texto plano en el caret actual, reemplazando la seleccion
// si la hay. Manual (sin execCommand) para no meter nodos con formato: el caret
// queda despues del texto insertado. El repintado posterior normaliza el DOM.
function insertTextAtCaret(div, text) {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) {
    div.appendChild(document.createTextNode(text));
    return;
  }
  const range = sel.getRangeAt(0);
  range.deleteContents();
  const node = document.createTextNode(text);
  range.insertNode(node);
  range.setStartAfter(node);
  range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
}

// beforeinput: mantenemos el contenteditable como texto plano estricto y con "\n"
// literal en vez de <div>/<br>. Interceptamos los inputType que meten estructura
// o formato; el tipeo normal (insertText) pasa de largo y lo maneja el input.
function onBeforeInput(e) {
  const div = editorEl;
  if (!div) {
    return;
  }
  const t = e.inputType || "";
  // Enter: en contenteditable el navegador mete <div>/<br>. Lo hacemos nosotros
  // con un "\n" literal para que el textContent siga siendo el markdown exacto.
  if (t === "insertParagraph" || t === "insertLineBreak") {
    e.preventDefault();
    insertTextAtCaret(div, "\n");
    onEditorInput();
    return;
  }
  // Undo/redo nativos (gesto del sistema o del menu): los maneja nuestro stack.
  if (t === "historyUndo") {
    e.preventDefault();
    undo(div);
    return;
  }
  if (t === "historyRedo") {
    e.preventDefault();
    redo(div);
    return;
  }
  // Formato enriquecido (Cmd+B/I, etc): bloqueado, aca no hay rich text.
  if (t.startsWith("format")) {
    e.preventDefault();
    return;
  }
  // El pegado y el drop con formato los manejan onPaste/onDrop (que cancelan el
  // evento nativo antes de este beforeinput), asi que aca no hace falta tocarlos.
}

// paste: insertamos SOLO text/plain, nunca el HTML del portapapeles.
function onPaste(e) {
  e.preventDefault();
  const div = editorEl;
  if (!div) {
    return;
  }
  const text = e.clipboardData ? e.clipboardData.getData("text/plain") : "";
  if (text) {
    insertTextAtCaret(div, text);
    onEditorInput();
  }
}

// drop: idem, solo texto plano en el caret (o al final).
function onDrop(e) {
  e.preventDefault();
  const div = editorEl;
  if (!div) {
    return;
  }
  const text = e.dataTransfer ? e.dataTransfer.getData("text/plain") : "";
  if (text) {
    insertTextAtCaret(div, text);
    onEditorInput();
  }
}

// input: en cada cambio del editor recalculamos dirty, registramos undo,
// programamos el repintado de spans y evaluamos el gesto de "linea en blanco".
// Durante una composicion IME NO hacemos nada (repintar abortaria la composicion);
// el compositionend vuelve a llamar aca una vez cerrada.
function onEditorInput() {
  if (isComposing) {
    return;
  }
  const div = editorEl;
  if (!div) {
    return;
  }
  refreshDirty();
  undoRecord(div);
  scheduleRepaint();
  maybeSplitOnBlankLine(div);
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

// Blur del editor: confirma, salvo que el blur venga de repintar la columna.
function onBlur() {
  if (isPainting) {
    return;
  }
  commitEdit();
}

// Entra a editar el bloque `i`. Si ya se estaba editando otro, lo confirma
// primero y reubica el foco al bloque que quedo en la misma posicion (la
// re-segmentacion puede haber cambiado los indices). `clickMap`, si viene, lleva
// { fullText, offset } para mapear el punto clickeado del render al source; si
// no, el caret cae al final. Nota: si el bloque YA estaba en edicion, el texto
// visible ES el source, asi que el caret del click directo lo pone el navegador
// nativamente (exacto) y no hace falta mapear.
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

// Teclas dentro de la superficie editable del bloque en edicion.
function onEditorKeydown(e) {
  const div = e.currentTarget;
  // Undo / redo del bloque (Cmd/Ctrl+Z, Shift+... para redo; tambien Ctrl+Y).
  if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "z") {
    e.preventDefault();
    if (e.shiftKey) {
      redo(div);
    } else {
      undo(div);
    }
    return;
  }
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "y") {
    e.preventDefault();
    redo(div);
    return;
  }
  if (e.key === "Escape") {
    e.preventDefault();
    goalColumn = null;
    commitEdit();
    return;
  }
  if (e.key === "ArrowUp" || e.key === "ArrowDown") {
    const dir = e.key === "ArrowUp" ? -1 : 1;
    const pos = getCaretOffset(div);
    if (pos === null) {
      return;
    }
    const text = div.textContent;
    const before = text.slice(0, pos);
    // "primera/ultima linea" se calcula sobre los "\n" del texto, no sobre las
    // filas visuales del word-wrap (aceptable para el spike).
    const onFirstLine = !before.includes("\n");
    const onLastLine = !text.slice(pos).includes("\n");
    const atEdge = dir < 0 ? onFirstLine : onLastLine;
    if (!atEdge) {
      // Movimiento vertical normal dentro del editor: WebKit maneja su propia
      // columna; reiniciamos la nuestra para que la proxima cadena arranque fresca.
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
// bloque donde cayo el caret, sin pasar por el mouse ni Escape.
//
// Fences: una linea en blanco DENTRO de un code fence abierto no parte el bloque
// (el nucleo lo mantiene entero). Lo detectamos sin logica de markdown: si tras
// re-segmentar el texto entero sigue siendo UN solo bloque identico, no hubo
// split y seguimos editando sin repintar (sin parpadeo).
async function maybeSplitOnBlankLine(div) {
  const pos = getCaretOffset(div);
  if (pos === null) {
    return;
  }
  const value = div.textContent;
  // Patron del doble Enter: el caret viene precedido por dos "\n" (linea en
  // blanco). Cubre el final de parrafo y tambien un corte en el medio.
  if (pos < 2 || value[pos - 1] !== "\n" || value[pos - 2] !== "\n") {
    return;
  }
  // Offset absoluto del caret en el documento completo (con el editor vivo).
  const absOffset = docOffsetOfBlockStart(editingIndex) + pos;
  const newBlocks = await invoke("render_blocks", { markdown: fullDocument() });
  const loc = locateOffset(newBlocks, absOffset);
  // Si el bloque que contiene el caret es identico al texto actual, NO hubo
  // corte estructural (linea en blanco final a la espera del proximo parrafo, o
  // fence abierto): seguimos editando el mismo editor sin repintar.
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
// IPC si el titulo efectivamente cambio.
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
  docPathEl.textContent = path || "sin archivo";
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
