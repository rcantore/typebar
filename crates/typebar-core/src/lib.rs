//! Nucleo agnostico de UI del editor typebar.
//!
//! Aca vive toda la logica que no depende de como se pinta la pantalla ni de
//! como se leen las teclas: el documento y su edicion, el historial de
//! undo/redo, los motions y la seleccion, el analisis de markdown, la busqueda,
//! el matcher fuzzy, el descubrimiento de archivos, el export a HTML, la
//! geometria de texto Unicode y las cadenas de i18n.
//!
//! Lo consumen la TUI (`typebar`) y, a futuro, la GUI (`typebar-gui`): ambas
//! frentes comparten este mismo nucleo y solo aportan la capa de presentacion.

pub mod buffers;
pub mod document;
pub mod export;
pub mod files;
pub mod fuzzy;
pub mod i18n;
pub mod markdown;
pub mod search;
pub mod text;
