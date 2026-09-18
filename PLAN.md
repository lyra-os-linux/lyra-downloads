# Lyra Downloads — decisões, validação e pendências

Documento curto para continuidade. Arquitetura: `docs/ARCHITECTURE.md`.

## Decisões

- **Workspace Rust** com 6 crates (core, aria2, ipc, backend, gtk, nativehost)
  + `tools/testserver` (só testes). Versões escolhidas pelo que o Leap 16 tem:
  gtk4-rs 0.11 / libadwaita-rs 0.9 (features `v4_12`/`v1_5`), rusqlite 0.32
  **sem** `bundled` (libsqlite3 do sistema), reqwest 0.12 com rustls + raízes
  do sistema (`ring`, sem aws-lc/cmake), rust ≥ 1.92 (MSRV das dependências).
- **Backend separado** (`lyra-downloads-backend`), instância única por `flock`,
  iniciado com `setsid` pela interface ou pelo native host.
- **aria2 exclusivo**: porta aleatória em loopback, segredo em arquivo `0600`,
  `--pause=true` na inicialização (o backend decide o que retomar), GID
  determinístico derivado do `Task.id` com fallback conferido empiricamente
  (aria2 1.37 responde "No URI to download." para GID repetido),
  `min-split-size=1M` (padrão do aria2 é 20M), `max-tries=5`.
- **Idempotência** por chave gravada na mesma transação da tarefa
  (`request_keys`); repasses do Firefox usam `firefox:<request_id>`.
- **Firefox**: MV3 com `background.scripts` (event page), `strict_min_version`
  140 (necessário para `data_collection_permissions`; é o ESR do Leap 16).
  `downloads`, `webRequest` e acesso a sites são permissões **opcionais**,
  pedidas só ao ligar a captura. ID definitivo
  `lyra-downloads@lyraos.com.br`.
- **Lyra como gerenciador padrão** (pedido do mantenedor em 2026-09-18):
  interruptor no popup e nas configurações da extensão; escopo "todos os
  sites" ou lista; "qualquer tipo" ou lista; opção de capturar com cookies;
  continua **desligado por padrão** e com as salvaguardas do escopo original.
- **Downloads que o Firefox não consegue pausar não são repassados** (evita
  duas transferências completas).
- **i18n**: gettext com mensagens-fonte em pt-BR; extrator próprio
  (`scripts/update-pot.py`) porque o xgettext 0.22 não entende Rust.
- **Ícone** provisório no estilo dos apps Lyra (não é logo aprovado).
- **Empacotamento** segue o padrão do Sulafat (tar.zst, `cargo_vendor`
  manual, `%cargo_build`), com dois pacotes: `lyra-downloads` e
  `lyra-downloads-firefox-integration` (x86_64, `Requires` da mesma versão).

## O que foi verificado nesta máquina (Lyra OS / Leap 16.1, x86_64)

| Verificação | Resultado |
|---|---|
| `cargo fmt --check`, `cargo clippy -D warnings` | passou |
| `cargo test --workspace` — 32 unitários + 10 de integração/e2e com aria2c 1.37.0 real e servidor HTTP local | passou |
| Integração: bytes/SHA-256 (confere, não confere, ausente), ranges paralelos, sem tamanho, sem ranges, redirecionamento, 403/404/conexão recusada, destino inválido, colisão de nomes, idempotência, pausa/retomada, encerramento e reinício do backend, pausa do usuário respeitada, cancelar × remover × excluir, mensagens IPC inválidas | passou |
| E2E native host: host em grupo próprio inicia backend, repasse idempotente, grupo do host morto e download termina; host com ID desconhecido recusado | passou |
| E2E queda: `SIGKILL` do backend (aria2 órfão encerrado após verificação em `/proc`) e depois de backend+aria2; conclusão íntegra, 1 tarefa | passou |
| Extensão: `tsc`, 17 testes de lógica (decisão, rastreador, protocolo, todos os caminhos de repasse, modo padrão), `web-ext lint` (0 erros; 1 aviso sobre Firefox Android, fora do escopo) | passou |
| `desktop-file-validate`, `appstreamcli validate`, `appstream-util validate-relax` | passou |
| `rpmbuild -ba` local com as macros reais do `cargo-packaging` 1.2 e `cargo-auditable` do Leap, build offline com vendor + `%check` completo | passou (`--nodeps`, pois as BuildRequires de empacotamento não puderam ser instaladas sem root) |
| rpmlint 2.7 do Leap (em venv temporário) | só erros esperados de build local (sem assinatura/Packager/changelog) + avisos de man page e Source sem URL; `checkbashisms` substituído por stub (checagem de bashisms **não** verificada) |
| Interface: executada na sessão GNOME/Wayland, download real, renderização offscreen conferida (layout largo, estados, erro) | verificado manualmente |
| Firefox 140 ESR real (perfil temporário via `web-ext run`) com host registrado para o usuário | carregado; testes de clique feitos pelo mantenedor |

## Não verificado / pendências

- [ ] Build no **OBS** (remoto) e `osc build`: não executados (sem projeto,
      conta ou credenciais definidos). Nada foi publicado.
- [ ] Alvo **Tumbleweed** e outras arquiteturas: não validados.
- [ ] Fluxo `obs_scm` (`_service.obs_scm.template`/`gen-service.sh`): não
      testado; depende de uma tag de lançamento.
- [x] ID definitivo da extensão: `lyra-downloads@lyraos.com.br`.
- [ ] **XPI assinado** pela Mozilla.
- [ ] Firefox Flatpak/Snap: não suportado.
- [ ] Teste de interface automatizado (janela estreita, leitores de tela):
      só verificação manual/offscreen.
- [ ] Man pages (aviso do rpmlint).
- [ ] Confirmar com o mantenedor: titular do copyright no spec ("Rodrigo
      Brito") e e-mail do `.changes` (rodrigo@lyraos.com.br), copiados do
      padrão do Sulafat.
- [ ] `npm audit` aponta vulnerabilidades em dependências de desenvolvimento
      do `web-ext`; nenhuma vai para o pacote da extensão.

## Diagnóstico de desenvolvimento

`LYRA_DOWNLOADS_RENDER_TO=/tmp/janela.png lyra-downloads` salva uma imagem da
janela após 3 s (útil onde o compositor bloqueia capturas de tela).
