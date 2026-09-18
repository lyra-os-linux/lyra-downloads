// Repasse de um download do Firefox para o Lyra Downloads.
//
// downloads.onCreated chega DEPOIS de o download começar; não há como
// impedi-lo preventivamente. Por isso o fluxo é:
//   1. pausar o download no Firefox (se não for possível pausar, NÃO
//      repassa: haveria duas transferências);
//   2. pedir ao host que o Lyra aceite a tarefa de forma durável;
//   3. só com o aceite confirmado, cancelar e apagar o download do Firefox;
//   4. em recusa, falha de IPC ou timeout, retomar o download no Firefox.
// Um download cancelado nunca é "retomado": reiniciar pelo Firefox é uma
// ação explícita, tratada como recomeço, e protegida contra recaptura.

import type { DownloadInfo } from "./rules.js";
import { basename } from "./rules.js";
import type { HostOp, HostResult } from "./protocol.js";

export type HandoffState =
  | "enviando"
  | "repassado"
  | "mantido_no_firefox"
  | "pausado_no_firefox"
  | "parado_no_firefox"
  | "reiniciado_no_firefox";

export interface HandoffEntry {
  requestId: string;
  downloadId: number;
  url: string;
  filename: string;
  state: HandoffState;
  message: string;
  time: number;
  canRestart: boolean;
}

export interface Deps {
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  cancel(id: number): Promise<void>;
  erase(id: number): Promise<void>;
  startDownload(url: string): Promise<number>;
  call(requestId: string, op: HostOp, timeoutMs: number): Promise<HostResult>;
  newId(): string;
  now(): number;
  changed(entries: HandoffEntry[]): void;
}

export const HANDOFF_TIMEOUT_MS = 8000;
const RESTART_GUARD_MS = 60_000;
const MAX_ENTRIES = 20;

export class HandoffManager {
  entries: HandoffEntry[] = [];
  private ignoreIds = new Set<number>();
  private ignoreUrls = new Map<string, number>();
  private inFlight = new Set<number>();

  constructor(private deps: Deps) {}

  /** Downloads reiniciados pela própria extensão não são capturados de novo. */
  isGuarded(item: DownloadInfo): boolean {
    if (this.ignoreIds.has(item.id) || this.inFlight.has(item.id)) return true;
    const until = this.ignoreUrls.get(item.url);
    if (until !== undefined) {
      if (this.deps.now() < until) return true;
      this.ignoreUrls.delete(item.url);
    }
    return false;
  }

  private add(entry: HandoffEntry): void {
    this.entries.unshift(entry);
    this.entries.length = Math.min(this.entries.length, MAX_ENTRIES);
    this.deps.changed(this.entries);
  }

  /** Finaliza uma entrada uma única vez (respostas repetidas/atrasadas são ignoradas). */
  private finish(entry: HandoffEntry, state: HandoffState, message: string, canRestart = false): void {
    if (entry.state !== "enviando") return;
    entry.state = state;
    entry.message = message;
    entry.canRestart = canRestart;
    this.deps.changed(this.entries);
  }

  async handoff(item: DownloadInfo): Promise<HandoffEntry> {
    const entry: HandoffEntry = {
      requestId: this.deps.newId(),
      downloadId: item.id,
      url: item.url,
      filename: basename(item.filename),
      state: "enviando",
      message: "Enviando ao Lyra Downloads…",
      time: this.deps.now(),
      canRestart: false,
    };
    this.inFlight.add(item.id);
    this.add(entry);
    try {
      await this.run(entry);
    } finally {
      this.inFlight.delete(item.id);
      // O download (cancelado ou não) nunca deve ser reprocessado.
      this.ignoreIds.add(item.id);
    }
    return entry;
  }

  private async run(entry: HandoffEntry): Promise<void> {
    try {
      await this.deps.pause(entry.downloadId);
    } catch {
      this.finish(
        entry,
        "mantido_no_firefox",
        "O Firefox não permitiu pausar este download (ou ele já terminou); ele continua no Firefox para evitar duas transferências.",
      );
      return;
    }

    const op: HostOp = { op: "handoff", url: entry.url, suggested_filename: entry.filename };
    let res = await this.deps.call(entry.requestId, op, HANDOFF_TIMEOUT_MS);
    if (res.kind === "timeout") {
      // Reenvio com o MESMO request_id: o backend é idempotente por ele.
      res = await this.deps.call(entry.requestId, op, HANDOFF_TIMEOUT_MS);
    }

    if (res.kind === "ok" && res.result?.status === "accepted") {
      try {
        await this.deps.cancel(entry.downloadId);
      } catch {
        this.finish(
          entry,
          "pausado_no_firefox",
          "O Lyra Downloads assumiu o download, mas o Firefox não permitiu cancelar a cópia dele, que ficou pausada. Remova-a pelo painel de downloads do Firefox.",
        );
        return;
      }
      try {
        await this.deps.erase(entry.downloadId);
      } catch {
        // Apagar da lista do Firefox é cosmético.
      }
      this.finish(entry, "repassado", "Enviado ao Lyra Downloads.");
      return;
    }

    let reason: string;
    if (res.kind === "timeout") {
      // Resultado incerto: pede ao backend que desista da tarefa desta
      // requisição (se ela chegou a ser criada) antes de devolver ao Firefox.
      await this.deps.call(entry.requestId, { op: "cancel_handoff" }, 3000);
      reason = "O Lyra Downloads não respondeu a tempo.";
    } else if (res.kind === "host_missing") {
      reason = res.message;
    } else if (res.kind === "error") {
      reason = `O Lyra Downloads recusou o download: ${res.message}`;
    } else {
      reason = "Resposta inesperada do Lyra Downloads.";
    }

    try {
      await this.deps.resume(entry.downloadId);
      this.finish(entry, "mantido_no_firefox", `${reason} O download continuou no Firefox.`);
    } catch {
      this.finish(
        entry,
        "parado_no_firefox",
        `${reason} O Firefox não conseguiu retomar o download pausado. Use “Reiniciar no Firefox” para começar de novo (a transferência recomeça do zero).`,
        true,
      );
    }
  }

  /** Recomeça o download pelo Firefox (ação explícita do usuário). */
  async restartInFirefox(requestId: string): Promise<boolean> {
    const entry = this.entries.find((e) => e.requestId === requestId);
    if (!entry || !entry.canRestart) return false;
    this.ignoreUrls.set(entry.url, this.deps.now() + RESTART_GUARD_MS);
    const id = await this.deps.startDownload(entry.url);
    this.ignoreIds.add(id);
    entry.state = "reiniciado_no_firefox";
    entry.message = "Download reiniciado no Firefox (do zero).";
    entry.canRestart = false;
    this.deps.changed(this.entries);
    return true;
  }
}
