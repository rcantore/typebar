// Binario finito de la GUI: oculta la consola en Windows (en release) y delega
// todo el arranque en la lib, siguiendo la convencion de Tauri.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    typebar_gui_lib::run();
}
