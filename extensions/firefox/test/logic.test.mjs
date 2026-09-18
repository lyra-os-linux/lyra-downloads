// Testes da lógica pura da extensão (executados no Node contra o JS compilado).
import { test } from "node:test";
import assert from "node:assert/strict";

import { decide, extensionMatches, hostMatches, normalizeDomain, hostPatterns } from "../dist/rules.js";
import { RequestTracker } from "../dist/tracker.js";
import { callHost } from "../dist/protocol.js";
import { HandoffManager } from "../dist/handoff.js";

const OWN = "lyra-downloads@lyraos.com.br";
const settings = {
  autoCapture: true,
  scope: "listed",
  anyFileType: false,
  allowCookiesAll: false,
  showApp: true,
  rules: [{ domain: "example.org", allowCookies: false }, { domain: "cdn.test", allowCookies: true }],
  extensions: ["iso", "tar.gz"],
};
const item = (over = {}) => ({
  id: 1, url: "https://dl.example.org/a.iso?token=x", filename: "/home/u/Downloads/a.iso",
  incognito: false, state: "in_progress", ...over,
});
const rec = (over = {}) => ({
  url: "https://dl.example.org/a.iso?token=x", method: "GET", hasCookie: false,
  hasAuthorization: false, incognito: false, time: 0, ...over,
});

test("decisão: captura GET simples de domínio e tipo permitidos", () => {
  const d = decide(item(), settings, rec(), OWN);
  assert.equal(d.capture, true);
  assert.equal(d.url, "https://dl.example.org/a.iso?token=x", "URL assinada preservada");
});

test("decisão: tudo que é duvidoso fica no Firefox", () => {
  const cases = [
    [item(), { ...settings, autoCapture: false }, rec(), "desativada"],
    [item({ incognito: true }), settings, rec(), "privada"],
    [item({ url: "blob:https://example.org/uuid" }), settings, rec(), "http"],
    [item({ url: "data:application/octet-stream;base64,AA==" }), settings, rec(), "http"],
    [item({ url: "https://evil.test/a.iso" }), settings, rec(), "domínio"],
    [item({ filename: "/d/a.html" }), settings, rec(), "tipo"],
    [item(), settings, undefined, "informação"],
    [item(), settings, rec({ method: "POST" }), "POST"],
    [item(), settings, rec({ hasAuthorization: true }), "autenticação"],
    [item(), settings, rec({ hasCookie: true }), "cookies"],
    [item({ byExtensionId: OWN }), settings, rec(), "própria"],
    [item({ state: "complete" }), settings, rec(), "terminou"],
    [item({ url: "https://u:p@dl.example.org/a.iso" }), settings, rec(), "credenciais"],
  ];
  for (const [i, s, r, why] of cases) {
    const d = decide(i, s, r, OWN);
    assert.equal(d.capture, false, why);
    assert.match(d.reason, new RegExp(why, "i"));
  }
});

test("decisão: cookies aceitos só quando a regra permite", () => {
  const d = decide(item({ url: "https://x.cdn.test/a.iso" }), settings, rec({ url: "https://x.cdn.test/a.iso", hasCookie: true }), OWN);
  assert.equal(d.capture, true);
});

test("tipo de arquivo usa o nome definido pelo Firefox, inclusive extensões compostas", () => {
  assert.ok(extensionMatches("/tmp/download?id=3/pacote.tar.gz", ["tar.gz"]));
  assert.ok(!extensionMatches("/tmp/pacote.gz", ["tar.gz"]));
  assert.ok(extensionMatches("C:\\x\\A.ISO", ["iso"]));
});

test("domínios", () => {
  assert.ok(hostMatches("a.b.example.org", "example.org"));
  assert.ok(!hostMatches("badexample.org", "example.org"));
  assert.equal(normalizeDomain(" *.SourceForge.net. "), "sourceforge.net");
  assert.equal(normalizeDomain("http://x"), null);
  assert.deepEqual(hostPatterns([{ domain: "a.org", allowCookies: false }]), ["*://a.org/*", "*://*.a.org/*"]);
});

test("rastreador herda método em redirecionamentos e expira registros", () => {
  let now = 0;
  const t = new RequestTracker(() => now);
  t.record("r1", { url: "https://a.org/get", method: "POST", hasCookie: false, hasAuthorization: false, incognito: false });
  t.redirect("r1", "https://mirror.a.org/file.iso");
  assert.equal(t.find("https://mirror.a.org/file.iso").method, "POST");
  now = 3 * 60 * 1000;
  assert.equal(t.find("https://mirror.a.org/file.iso"), undefined);
});

test("protocolo: host ausente, erro, timeout e resposta de outra requisição", async () => {
  const missing = await callHost(() => Promise.reject(new Error("No such native application org.lyraos.downloads")), "r", { op: "health" }, 100);
  assert.equal(missing.kind, "host_missing");
  const err = await callHost(async () => ({ v: 1, request_id: "r", ok: false, error: { code: "invalid_url", message: "x" } }), "r", { op: "health" }, 100);
  assert.deepEqual([err.kind, err.code], ["error", "invalid_url"]);
  const slow = await callHost(() => new Promise(() => {}), "r", { op: "health" }, 20);
  assert.equal(slow.kind, "timeout");
  const other = await callHost(async () => ({ v: 1, request_id: "zzz", ok: true, result: {} }), "r", { op: "health" }, 100);
  assert.equal(other.kind, "error");
});

// ---- repasse -----------------------------------------------------------

function harness(script) {
  const log = [];
  const calls = [];
  let n = 0;
  const deps = {
    pause: async (id) => { log.push(`pause ${id}`); if (script.pauseFails) throw new Error("nope"); },
    resume: async (id) => { log.push(`resume ${id}`); if (script.resumeFails) throw new Error("nope"); },
    cancel: async (id) => { log.push(`cancel ${id}`); if (script.cancelFails) throw new Error("nope"); },
    erase: async (id) => { log.push(`erase ${id}`); },
    startDownload: async (url) => { log.push(`start ${url}`); return 99; },
    call: async (rid, op) => {
      calls.push([rid, op.op]);
      const r = script.responses.shift();
      return typeof r === "function" ? r(rid, op) : r;
    },
    newId: () => `req-${++n}`,
    now: () => 1000,
    changed: () => {},
  };
  return { m: new HandoffManager(deps), log, calls };
}
const accepted = { kind: "ok", result: { status: "accepted", task: { task_id: "t1" } } };

test("repasse aceito: pausa, confirma, só então cancela e apaga", async () => {
  const h = harness({ responses: [accepted] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "repassado");
  assert.deepEqual(h.log, ["pause 1", "cancel 1", "erase 1"]);
  assert.ok(h.m.isGuarded(item()), "mesmo download não é reprocessado");
});

test("recusa do backend: download retomado no Firefox, nada cancelado", async () => {
  const h = harness({ responses: [{ kind: "error", code: "invalid_url", message: "inválido" }] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "mantido_no_firefox");
  assert.deepEqual(h.log, ["pause 1", "resume 1"]);
});

test("host ausente: retoma no Firefox e explica", async () => {
  const h = harness({ responses: [{ kind: "host_missing", message: "não instalado" }] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "mantido_no_firefox");
  assert.match(e.message, /não instalado/);
});

test("timeout seguido de aceite no reenvio com o MESMO request_id", async () => {
  const h = harness({ responses: [{ kind: "timeout" }, accepted] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "repassado");
  assert.equal(h.calls[0][0], h.calls[1][0], "reenvio usa o mesmo request_id (idempotente)");
});

test("timeout duplo: desiste no backend (cancel_handoff) e retoma no Firefox", async () => {
  const h = harness({ responses: [{ kind: "timeout" }, { kind: "timeout" }, { kind: "ok", result: {} }] });
  const e = await h.m.handoff(item());
  assert.deepEqual(h.calls.map((c) => c[1]), ["handoff", "handoff", "cancel_handoff"]);
  assert.equal(e.state, "mantido_no_firefox");
  assert.ok(!h.log.includes("cancel 1"));
});

test("download que não pausa não é repassado (evita duas transferências)", async () => {
  const h = harness({ pauseFails: true, responses: [] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "mantido_no_firefox");
  assert.equal(h.calls.length, 0, "nada enviado ao Lyra");
});

test("falha ao cancelar após aceite: cópia fica pausada e o usuário é avisado", async () => {
  const h = harness({ cancelFails: true, responses: [accepted] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "pausado_no_firefox");
});

test("falha ao retomar: oferece reinício explícito, protegido contra recaptura", async () => {
  const h = harness({ resumeFails: true, responses: [{ kind: "error", code: "backend_error", message: "x" }] });
  const e = await h.m.handoff(item());
  assert.equal(e.state, "parado_no_firefox");
  assert.equal(e.canRestart, true);
  assert.ok(await h.m.restartInFirefox(e.requestId));
  assert.equal(e.state, "reiniciado_no_firefox");
  // O download reiniciado (novo id, mesma URL) não é capturado de novo.
  assert.ok(h.m.isGuarded(item({ id: 99 })));
  assert.ok(h.m.isGuarded(item({ id: 123 })), "URL protegida mesmo antes de saber o novo id");
  assert.equal(await h.m.restartInFirefox(e.requestId), false, "reinício não se repete");
});

test("resposta repetida/atrasada não altera um repasse já finalizado", async () => {
  const h = harness({ responses: [accepted] });
  const e = await h.m.handoff(item());
  // Simula uma finalização tardia (ex.: segunda resposta): o estado não muda.
  h.m["finish"](e, "mantido_no_firefox", "tarde demais");
  assert.equal(e.state, "repassado");
});

test("modo gerenciador padrão: todos os sites e qualquer tipo, mantendo as salvaguardas", () => {
  const all = { ...settings, scope: "all", anyFileType: true };
  const other = { url: "https://qualquer.site/baixar?id=9", filename: "/d/relatorio.pdf" };
  assert.equal(decide(item(other), all, rec({ url: other.url }), OWN).capture, true);
  assert.equal(decide(item(other), all, rec({ url: other.url, hasCookie: true }), OWN).capture, false, "cookies: fica no Firefox por padrão");
  assert.equal(decide(item(other), { ...all, allowCookiesAll: true }, rec({ url: other.url, hasCookie: true }), OWN).capture, true);
  assert.equal(decide(item({ ...other, incognito: true }), all, rec({ url: other.url }), OWN).capture, false);
  assert.equal(decide(item(other), all, rec({ url: other.url, method: "POST" }), OWN).capture, false);
  assert.equal(decide(item(other), { ...all, autoCapture: false }, rec({ url: other.url }), OWN).capture, false, "desligado por padrão");
});
