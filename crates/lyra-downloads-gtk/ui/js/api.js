// Ponte com o Rust (comandos Tauri e eventos).

const tauri = window.__TAURI__;

/** Chama um comando; erros chegam como texto já traduzido. */
export async function call(cmd, args = {}) {
  try {
    return await tauri.core.invoke(cmd, args);
  } catch (error) {
    throw new Error(typeof error === "string" ? error : error?.message ?? String(error));
  }
}

/** Operação do protocolo do serviço (`{ op: "pause", id }` etc.). */
export const backend = (op) => call("backend_call", { op });

/** Escuta um evento do Rust; devolve a função que cancela a escuta. */
export function on(event, handler) {
  const pending = tauri.event.listen(event, (e) => handler(e.payload));
  return () => pending.then((unlisten) => unlisten());
}

/** Seletor de pasta nativo; devolve o caminho ou null. */
export async function pickFolder(title, defaultPath) {
  return tauri.core.invoke("plugin:dialog|open", { options: { directory: true, multiple: false, title, defaultPath } });
}

export const win = tauri.window.getCurrentWindow();
