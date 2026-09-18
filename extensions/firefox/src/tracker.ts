// Memória curta das requisições observadas pelo webRequest, para saber o
// método e os cabeçalhos da requisição que originou um download.

import type { RequestRecord } from "./rules.js";

const MAX_AGE_MS = 2 * 60 * 1000;
const MAX_ENTRIES = 500;

export class RequestTracker {
  private byUrl = new Map<string, RequestRecord>();
  private byRequestId = new Map<string, RequestRecord>();

  constructor(private now: () => number = Date.now) {}

  record(requestId: string, rec: Omit<RequestRecord, "time">): void {
    const full = { ...rec, time: this.now() };
    this.byRequestId.set(requestId, full);
    this.byUrl.set(rec.url, full);
    this.prune();
  }

  /** Um redirecionamento herda o registro original (se a origem foi um
   * POST, o destino também não é considerado reproduzível). */
  redirect(requestId: string, redirectUrl: string): void {
    const orig = this.byRequestId.get(requestId);
    if (!orig) return;
    const rec = { ...orig, url: redirectUrl, time: this.now() };
    this.byUrl.set(redirectUrl, rec);
  }

  find(url: string): RequestRecord | undefined {
    const rec = this.byUrl.get(url);
    if (!rec || this.now() - rec.time > MAX_AGE_MS) return undefined;
    return rec;
  }

  private prune(): void {
    const limit = this.now() - MAX_AGE_MS;
    for (const [k, v] of this.byUrl) if (v.time < limit) this.byUrl.delete(k);
    for (const [k, v] of this.byRequestId) if (v.time < limit) this.byRequestId.delete(k);
    while (this.byUrl.size > MAX_ENTRIES) {
      const first = this.byUrl.keys().next().value;
      if (first === undefined) break;
      this.byUrl.delete(first);
    }
  }
}

export function headerPresent(headers: { name: string }[] | undefined, name: string): boolean {
  const n = name.toLowerCase();
  return (headers ?? []).some((h) => h.name.toLowerCase() === n);
}
