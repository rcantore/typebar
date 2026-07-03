// Genera el contexto de Tauri en tiempo de compilacion: valida tauri.conf.json
// y las capabilities, y embebe los estaticos de ui/ dentro del binario.
fn main() {
    tauri_build::build();
}
