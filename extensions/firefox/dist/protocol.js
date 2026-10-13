// Protocolo com o native host (org.lyraos.downloads). Versão 1.
export const HOST_NAME = "org.lyraos.downloads";
export const PROTOCOL_VERSION = 1;
export function newRequestId() {
    return crypto.randomUUID();
}
/** Envia uma mensagem ao host com timeout. Distingue host ausente (não
 * instalado/registrado), erro estruturado e timeout (resultado incerto). */
export async function callHost(send, requestId, op, timeoutMs) {
    const msg = { v: PROTOCOL_VERSION, request_id: requestId, ...op };
    let timer;
    const timeout = new Promise((resolve) => {
        timer = setTimeout(() => resolve({ kind: "timeout" }), timeoutMs);
    });
    const call = send(HOST_NAME, msg).then((raw) => {
        const r = raw;
        if (!r || typeof r !== "object")
            return { kind: "error", code: "invalid_response", message: "Resposta inválida do componente de integração." };
        if (r.request_id !== null && r.request_id !== requestId) {
            return { kind: "error", code: "invalid_response", message: "Resposta de outra requisição." };
        }
        if (r.ok)
            return { kind: "ok", result: r.result };
        return { kind: "error", code: r.error?.code ?? "unknown", message: r.error?.message ?? "Erro desconhecido." };
    }, (e) => {
        const text = String(e?.message ?? e);
        if (/No such native application|not found|Access to the specified native messaging host is forbidden/i.test(text)) {
            return {
                kind: "host_missing",
                message: "O Lyra Downloads ou o componente de integração com o Firefox não está instalado ou registrado.",
            };
        }
        return { kind: "error", code: "host_failed", message: "O componente de integração encerrou inesperadamente." };
    });
    try {
        return await Promise.race([call, timeout]);
    }
    finally {
        clearTimeout(timer);
    }
}
