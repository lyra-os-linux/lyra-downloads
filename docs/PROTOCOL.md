# Protocolos

## 1. IPC local (interface/native host ↔ backend)

- Transporte: socket Unix `$XDG_RUNTIME_DIR/lyra-downloads/backend.sock`.
- Uma requisição JSON por linha; uma resposta por requisição; limite de 1 MiB.
- Só conexões do mesmo UID.

Requisição:

```json
{"v": 1, "request_id": "c123-1", "op": "add_download", "url": "https://…", "source": "interface"}
```

Resposta:

```json
{"v": 1, "request_id": "c123-1", "ok": true, "result": {"task_id": "…", "duplicate": false, "filename": "a.iso"}}
{"v": 1, "request_id": "c123-1", "ok": false, "error": {"code": "invalid_destination", "message": "…"}}
```

Operações (`op`): `health`, `list_tasks`, `probe`, `add_download`, `pause`,
`resume`, `cancel`, `retry`, `retry_with_new_url`, `remove_from_history`,
`delete_file`, `move`, `change_connections`, `cancel_by_request_key`,
`pause_all`, `resume_all`, `get_settings`, `restart_engine`, `set_settings`,
`shutdown`. Qualquer outro valor é recusado com `invalid_request`.

Códigos de erro: `invalid_request`, `unsupported_version`, `not_found`,
`invalid_url`, `invalid_destination`, `invalid_state`, `engine_unavailable`,
`internal`.

**Idempotência**: `add_download.idempotency_key` é gravada na mesma transação
da tarefa. Reenviar a mesma chave devolve a mesma tarefa com
`"duplicate": true`; downloads intencionais posteriores da mesma URL usam
chaves diferentes e não são bloqueados.

Desde 0.1.1, `cancel_by_request_key` grava uma revogação mesmo se a tarefa
ainda não existir. Reenvios tardios dessa chave são recusados, inclusive após
reiniciar o backend. Uma resposta com `revoked: true` e `completed: false`
confirma que a tarefa está parada/ausente e não poderá ser criada por um
handoff atrasado; `completed: true` informa que o Lyra já concluiu a transferência.
Falha ao confirmar o estado no aria2 retorna erro, sem autorizar o navegador
a iniciar outra transferência.

O schema SQLite passa de 1 para 2, adicionando `cancelled_requests`; tarefas
e preferências são preservadas. Binários 0.1.0 recusam o schema novo. Um
rollback para 0.1.0 exige restaurar o banco anterior, com o backend parado.

## 2. Native Messaging (extensão ↔ `lyra-downloads-nativehost`)

- Nome do host: `org.lyraos.downloads`.
- Enquadramento oficial do Firefox: 4 bytes de tamanho (u32, ordem nativa) +
  JSON UTF-8. Limite: 1 MiB. stdout é exclusivo do protocolo; diagnósticos
  vão para stderr, sem URLs completas.
- Leituras parciais são remontadas; EOF entre mensagens encerra normalmente;
  mensagem truncada ou grande demais encerra a conexão; JSON inválido recebe
  erro estruturado e o laço continua.
- O host confere que foi chamado pelo ID de extensão permitido (argumento
  passado pelo Firefox) e só aceita `request_id` com `[A-Za-z0-9_-]{1,128}`.

| `op` | Efeito | Resposta de sucesso |
|---|---|---|
| `health` | inicia o backend se preciso e consulta o estado | `{"host_version", "backend": {...}}` |
| `open_app` | abre/mostra a janela | `{"status": "app_opened"}` |
| `send_download` (`url`, `suggested_filename?`) | abre o diálogo "Novo download" preenchido; **não cria tarefa** | `{"status": "dialog_opened"}` |
| `handoff` (`url`, `suggested_filename?`) | pede ao backend que aceite a tarefa de forma durável (chave `firefox:<request_id>`) | `{"status": "accepted", "task": {"task_id", "duplicate"}}` |
| `cancel_handoff` | revoga a chave e confirma a parada da tarefa, ou sua conclusão | `{"found", "cancelled", "revoked", "completed"}` |

Erros: `invalid_request`, `unsupported_version`, `invalid_url` (só http/https
sem credenciais embutidas), `app_unavailable`, `backend_error`, `too_large`,
`not_allowed`.

## 3. Repasse automático no Firefox (estados)

`downloads.onCreated` chega **depois** que o download começou; não é uma API
de bloqueio. O fluxo:

1. Decidir (pura, testada): captura ligada; fora da navegação privada;
   `http/https`; domínio/tipo conforme as regras; existe registro do
   `webRequest` para a URL (ou cadeia de redirecionamentos) com método `GET`,
   sem `Authorization` e — salvo permissão explícita — sem cookies. Na dúvida,
   o download fica no Firefox.
2. `downloads.pause(id)`. Se não for possível pausar, **não repassa**
   (evita duas transferências).
3. `handoff` com um `request_id` novo, timeout de 8 s; em timeout, um reenvio
   com o **mesmo** `request_id`.
4. Aceite → `downloads.cancel(id)` e `downloads.erase`. Se o cancelamento
   falhar, a cópia do Firefox fica pausada e o usuário é avisado.
5. Recusa de validação ou host ausente na primeira tentativa → `downloads.resume(id)`.
6. Timeout ou falha de transporte → `cancel_handoff`. Só retomar após
   `revoked: true, completed: false`. Resposta perdida, erro ou backend antigo
   deixam a cópia do Firefox pausada e oferecem “Recuperar no Firefox”. Se
   `completed: true`, cancelar apenas a cópia do Firefox.
7. Se `resume` falhar, o popup oferece "Reiniciar no Firefox": é um **novo**
   download (do zero), e a URL/ID ficam protegidos contra nova captura.

Estados exibidos: `enviando`, `repassado`, `mantido_no_firefox`,
`pausado_no_firefox`, `parado_no_firefox`, `reiniciado_no_firefox`, `repasse_incerto`. Cada
repasse é finalizado uma única vez (respostas atrasadas são ignoradas) e o
mesmo download nunca é processado duas vezes.

As entradas, o aceite e as proteções contra recaptura são persistidos em
`storage.session` antes dos efeitos externos. Ao recriar a página de fundo,
a extensão restaura a sessão antes de processar downloads ou pedidos do popup.
Uma entrada interrompida requer reconciliação; um aceite já persistido apenas
completa a limpeza no Firefox, sem cancelar a tarefa entregue ao Lyra.
