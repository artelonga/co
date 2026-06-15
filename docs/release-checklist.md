# CO — Release Checklist (Wave-Based)

**Versão:** vivo, atualizado por release  
**URL:** <https://co.artelonga.com.br> (prod — único alvo obrigatório)  
**Ambientes e deploy:** ver [`docs/OPERATIONS.md` → "Environments & Deploy"](OPERATIONS.md)
(fonte única da verdade). Resumo: deploy **prod-direto**; não há UAT; staging
(`co-artelonga-staging`) é **preview manual opcional**, não um gate.  
**Cadência:** bi-semanal, quinta-feira 15:00 BRT (corte de PR quarta 23:59 BRT)

Este documento é o gate de cada release. Cada wave consolida múltiplos PRs em UMA tag git semver. A checklist roda antes do `scripts/release-commit.sh`.

## Como reportar

Mesmos canais que `feedback-checklist.md` — issues, e-mail, bate-papo.

---

## Antes de qualquer release — Gate de pré-flight

- [ ] Todos os PRs da wave atual mergeados em `main` antes do corte (quarta 23:59 BRT)
- [ ] `CHANGELOG-PENDING/` contém entrada para cada CO-N da wave
- [ ] **Gate de usabilidade prod (CO-421) verde** — bloqueia release se vermelho (ver abaixo)
- [ ] Drift check OpenAPI verde (CO-350) — bloqueia release se vermelha
- [ ] Migration validation verde para qualquer PR com migração nova (CO-376)
- [ ] `gh pr list --state open` mostra zero PRs com label `release-blocker`
- [ ] yuri@artelonga.com.br consegue logar em prod com senha (CO-377)

### Gate de usabilidade prod — CO-421 (substitui o smoke manual de UAT)

Sem staging (decisão 2026-06-12): o alvo é **prod direto**. Uma suite Playwright
**anônima e read-only** valida a usabilidade real de produção — o que o health 200
não pega. É **read-only por construção**: um interceptor de request aborta e falha a
suite se qualquer `POST/PUT/PATCH/DELETE` for emitido, então **nunca muta prod**.

Rode como gate pré-deploy (e novamente pós-deploy) — bloqueia a promoção se vermelho:

```bash
cd co-web && BASE_URL=https://co.artelonga.com.br \
  npx playwright test e2e/prod-usability.spec.ts \
  --project=desktop-chromium --workers=2
```

Cobre (tempo total < ~2 min; verificado verde em ~40s):

- [ ] Board do template (`/template`) carrega com tarefas tutorial visíveis (stat `tarefas` > 0 + card visível)
- [ ] Troca de tema aplica (`html[data-palette]` muda)
- [ ] Toggle pt/en muda os rótulos do botão de idioma
- [ ] Deep-link de entrada (`/template/projects/CO/1`) renderiza markdown no zoom (não cai no 404)
- [ ] Grafo/lente (stats de conteúdo / dashboard) abre

> CI / localhost: sem `BASE_URL` a mesma suite roda contra `http://localhost:3000`
> (um `co serve` local), então também serve de smoke em PR. O alvo prod só é
> exercido quando `BASE_URL=https://co.artelonga.com.br` é passado explicitamente.

## Roteiro por wave

### Wave 4 — v3.0 (Public Mobile Release)

**Tema:** Brain on any device, public.

Critérios de pronto (DoD por entregável):

#### Substrate gates

- [ ] **CO-379** _(histórico — staging é preview manual opcional, não gate; ver OPERATIONS.md)_ Staging Fly app `co-artelonga-staging` responde 200 em `/api/health` quando deployado à mão
- [ ] **CO-365** Backup backend trait + LocalFsBackend gravam snapshot diário em `/data/backups/`
- [ ] **CO-278-B** `X-RateLimit-Limit` + `X-RateLimit-Remaining` presentes em todas as respostas `/api/v1/*`
- [ ] **CO-360** `/gestao/resumo` renderiza 4 abas (Resumo / Conteúdo / Usuários / Atividades) a partir de um único endpoint
- [ ] **CO-378** `/2026-05-29/` não aparece em `top_pages` do `/gestao/resumo` (privacidade do noindex)

#### Sala (workspace) gates

- [ ] **CO-352** `/u/comunicacao/sala?template=mbya` carrega canvas espacial
- [ ] **CO-352** btn-add coloca termo no canvas (drag de fora)
- [ ] **CO-352** btn-link conecta dois termos com seta tipada
- [ ] **CO-352** btn-compor abre modal de compor + grava via API
- [ ] **CO-354** btn-sugerir aceita anônimo (rate-limited 5/min/IP)
- [ ] **CO-354** Owner em `/u/comunicacao/review` aprova/rejeita sugestão
- [ ] **CO-354** Status do entry vai draft → reviewed → published
- [ ] **CO-355** Dropdown de template lista mbya-basics + yoruba-basics + blank
- [ ] **CO-355** Carregar template seeda nós no canvas
- [ ] **CO-353** Dois navegadores na mesma sala veem cursor um do outro
- [ ] **CO-353** Drop de termo broadcasta para outro cliente < 300ms
- [ ] **CO-353** Reconexão automática em queda de WebSocket

#### Mobile gates

- [ ] **CO-356** Drag de card entre colunas funciona em iPhone Safari + Android Chrome
- [ ] **CO-357** Lighthouse PWA score ≥ 90 em prod
- [ ] **CO-357** Botão "Instalar app" aparece após `beforeinstallprompt`
- [ ] **CO-357** SW cacheia `/lib/*.js` + `/api/v1/universes/*/entries` para leitura offline
- [ ] **CO-358** Sidebar vira drawer com swipe-from-left em viewport ≤ 640px
- [ ] **CO-358** Breadcrumbs colapsam para "← Voltar" em ≤ 480px
- [ ] **CO-358** Kanban vira lista vertical em ≤ 640px
- [ ] **CO-358** Hit areas ≥ 44px (HIG)
- [ ] **CO-359** CI matrix verde para Pixel 7 + iPhone 14 + iPad Pro

#### Identity + contract gates

- [ ] **CO-377** Token de prod aceito (curl test) — _cross-env com staging só se staging estiver deployado à mão_
- [ ] **CO-374** Playwright cobre: A → A/B → A/B/C recursão; promoção de A/B a root B; funil discover→register; rotas gerais
- [ ] **CO-375** Probe de contrato (OpenAPI drift) verde — roda contra localhost/prod, não exige staging
- [ ] **CO-376** PR com migração nova passa por snapshot+migrate+smoke antes de mergeavel

---

### Pós-release (15:05 BRT quinta)

- [ ] `git tag v3.0.0` empurrado para origin
- [ ] `gh release create v3.0.0` com release notes geradas de `CHANGELOG.md`
- [ ] Retrospectiva CO-369 corre + commita `docs/scrum/sprints/sprint-N.md`
- [ ] CO-372 calendar atualiza para próxima sprint
- [ ] Anúncio em `/sobre` (próxima sprint)
- [ ] Post no blog `artelonga.com.br/blog/`
- [ ] Convite público enviado para waitlist

---

## Roteiro por funcionalidade — gates contínuos (todo release)

### 1. Primeiro contato anônimo

- [ ] Página carrega em < 3s na primeira visita (3G simulado: < 8s)
- [ ] Idioma padrão pt-BR
- [ ] Tema padrão Modern
- [ ] "Aprenda CO" mostra tutorial em pt-BR
- [ ] Drag de tarefa funciona sem login (CO-356 garante touch)
- [ ] Criar tarefa via "+ Nova Tarefa" funciona
- [ ] Refresh mantém estado (anônimo)
- [ ] Após 100 entradas: modal "Crie uma conta"
- [ ] **0 mixed-content warnings no console** (CO-362)
- [ ] **0 entries de probe na feedback inbox** (CO-339)

### 2. Cadastro e login

- [ ] Magic-code chega via e-mail (ou log do servidor)
- [ ] Verificação de código avança signup-source lead para `in_progress` (CO-370)
- [ ] Password login funciona para admin (yuri)
- [ ] Google OAuth funciona em prod
- [ ] Logout limpa cookie + volta para anônimo

### 3. Universos

- [ ] Sidebar lista universos do usuário (CO-280 IA)
- [ ] Criar universo via `+`
- [ ] Universo novo é privado por padrão
- [ ] Tornar público / privado funciona
- [ ] Duplicar universo
- [ ] Sub-universo (`parent_key`) renderiza com breadcrumb (CO-280)
- [ ] Promoção sub-universo → root funciona (CO-347 pattern)
- [ ] Cross-universe wikilink `[[key::path]]` resolve (CO-363)

### 4. Conteúdo

- [ ] Conteúdo no `co` universo aparece (CO-346 fixed empty board)
- [ ] Conteúdo em sub-universos (`mbya`, `yoruba`) renderiza
- [ ] Sister-repo content (yuri, neuro, odysseus, claude-code) sincronizado (CO-337/CO-347/CO-364)
- [ ] Frontmatter `private: true` esconde entry de listagem anônima
- [ ] Markdown renderiza com http→https rewrite (CO-362)

### 5. Sala (Wave 4)

- [ ] `/u/comunicacao/sala` abre canvas
- [ ] Buttons compor / link / sugerir / publicar / review / fit / salas funcionam
- [ ] Multi-user presence com cursores (CO-353)
- [ ] Estado per-user persiste

### 6. Funil de conversão

- [ ] Visit com UTM tracking corretamente (CO-340)
- [ ] CTA clica → goal event registrado
- [ ] Form de lead cria lead row (CO-370)
- [ ] Signup também cria lead row (source=signup, CO-370)
- [ ] Email é join key
- [ ] Funnel report mostra 8 steps com drop-off (CO-371, post-Wave 5)
- [ ] Privacidade: paths `noindex` não aparecem em `top_pages` (CO-378)

### 7. Pagamento (Wave 5, v3.1)

- [ ] CTA pós-register abre checkout Hostinger
- [ ] Webhook valida assinatura
- [ ] Tier do user flipa para `paid` após sucesso
- [ ] Atividades log registra `billing.payment_succeeded`

### 8. Admin (gestao)

- [ ] `/gestao/resumo` carrega em < 1s
- [ ] 4 abas (Resumo / Conteúdo / Usuários / Atividades) renderizam
- [ ] Privacidade: paths `noindex` agrupados em `🔒 (private)` (CO-378)
- [ ] `?include_private=true` requer admin auth
- [ ] Atividades log (CO-361) mostra eventos recentes
- [ ] Schema versions visível: "DB schema vN / app vX.Y.Z" (ex.: DB schema v88 / app v3.15.0 — número de schema vem de `co-web/src/storage/migrations/`)

### 9. Backup + restore

- [ ] Snapshot diário em `/data/backups/` (CO-365 LocalFsBackend)
- [ ] Retenção 30 dias por default
- [ ] `POST /api/v1/admin/backup/snapshot` força snapshot manual
- [ ] Backend pluggable: env `CO_BACKUP_BACKEND` aceita `local|s3|r2|fly|gcs|disabled`

### 10. Segurança + privacidade

- [ ] TLS válido em prod
- [ ] HSTS habilitado
- [ ] `X-RateLimit-*` headers em todas as `/api/v1/*` (CO-278-B)
- [ ] 61 requests/min anônimo → 429 (CO-278-B)
- [ ] User-Agent vazio → 400 (CO-278-B)
- [ ] Webhook signatures verificadas (CO-366)
- [ ] Paths privados redactados em analytics (CO-378)
- [ ] Atividades log tem SENSITIVE_KEYS redactor (CO-361)

---

## Lista de waves futuras (forward-looking)

### Wave 5 — v3.1.0 (Monetização + KB)
- CO-366 conversão/pagamento Hostinger
- CO-367 universal content → KB sync
- CO-371 funnel report
- CO-372 sprint calendar com DoD

### Wave 6 — v3.2.0 (Security epic)
- CO-145 encrypted assets
- CO-86 `.co` file format
- CO-87 composable protocol stack
- CO-110 filesystem-as-web

### Wave 7 — v3.3.0 (Sync + offline)
- CO-61 sync protocol v1
- CO-62 quilombo sync adapter
- CO-128 4-way conflict UI
- CO-58 PWA offline (estendendo CO-357)

### Wave 8 — v3.4.0 (Scale)
- CO-76 scalability infrastructure
- CO-78 job queue + worker pool
- CO-79 caching layer
- CO-80 per-tier rate limits
- CO-101 load test scaffolding
- CO-285/286 Fly cost optimization

### Wave 9 — v3.5.0 (Universe types epic)
- CO-63 universe manifest + plugin system
- CO-70 manifest format spec
- CO-89 git-backed universes
- CO-93 universe types (public-static / private-static / private-dynamic)

### Wave 10 — v3.6.0 (Native + advanced)
- CO-344 Capacitor iOS/Android
- CO-211 v2 (OpenAPI evolution)
- CO-264 (advanced)

### Ongoing (sem version tag)
- CO-227 server decomposition
- CO-228 type safety
- CO-170 universe + project hygiene
- CO-144 dados/ panel
- CO-281/285/286 Fly cost rightsize
- CO-283 cross-universe graph canvas (post-CO-345)
- CO-284 pluggable infrastructure
- CO-298 `co serve --staging` mode

---

## Critérios de "shippable" — quando pode marcar uma wave como done

1. Todos os DoD items da wave marcados ✅
2. Gate de usabilidade prod (CO-421) verde no pré-deploy E pós-deploy
3. Drift check OpenAPI verde
4. Backup snapshot < 24h existe
5. Migration validation verde para qualquer PR com migração nova
6. Operator (yuri) confirmou via `/sobre` que está pronto
7. Cron de release-commit fires automático na quinta 15:00 BRT, ou manual via `scripts/release-commit.sh`
