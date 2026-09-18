// Protocolo com o native host (org.lyraos.downloads). Versão 1.

export const HOST_NAME = "org.lyraos.downloads";
export const PROTOCOL_VERSION = 1;

export type HostOp =
  | { op: "health" }
  | { op: "open_app" }
  | { op: "send_download"; url: string; suggested_filename?: string }
  | { op: "handoff"; url: string; suggested_filename?: string }
  | { op: "cancel_handoff" };

export interface HostResponse {
  v: number;
  request_id: string | null;
  ok: boolean;
  result?: any;
  error?: { code: string; message: string };
}

export type HostResult =
  | { kind: "ok"; result: any }
  | { kind: "error"; code: string; message: string }
  | { kind: "host_missing"; message: string }
  | { kind: "timeout" };

export type SendNative = (host: string, msg: object) => Promise<unknown>;

export function newRequestId(): string {
  return crypto.randomUUID();
}

/** Envia uma mensagem ao host com timeout. Distingue host ausente (não
 * instalado/registrado), erro estruturado e timeout (resultado incerto). */
export async function callHost(
  send: SendNative,
  requestId: string,
  op: HostOp,
  timeoutMs: number,
): Promise<HostResult> {
  const msg = { v: PROTOCOL_VERSION, request_id: requestId, ...op };
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<HostResult>((resolve) => {
    timer = setTimeout(() => resolve({ kind: "timeout" }), timeoutMs);
  });
  const call = send(HOST_NAME, msg).then(
    (raw): HostResult => {
      const r = raw as HostResponse;
      if (!r || typeof r !== "object") return { kind: "error", code: "invalid_response", message: "Resposta inválida do componente de integração." };
      if (r.request_id !== null && r.request_id !== requestId) {
        return { kind: "error", code: "invalid_response", message: "Resposta de outra requisição." };
      }
      if (r.ok) return { kind: "ok", result: r.result };
      return { kind: "error", code: r.error?.code ?? "unknown", message: r.error?.message ?? "Erro desconhecido." };
    },
    (e: unknown): HostResult => {
      const text = String((e as Error)?.message ?? e);
      if (/No such native application|not found|Access to the specified native messaging host is forbidden/i.test(text)) {
        return {
          kind: "host_missing",
          message: "O Lyra Downloads ou o componente de integração com o Firefox não está instalado ou registrado.",
        };
      }
      return { kind: "error", code: "host_failed", message: "O componente de integração encerrou inesperadamente." };
    },
  );
  try {
    return await Promise.race([call, timeout]);
  } finally {
    clearTimeout(timer);
  }
}
