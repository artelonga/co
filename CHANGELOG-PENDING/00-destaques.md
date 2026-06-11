- **`co updates`** — release notes no terminal. `-n 3` últimas, `--all` histórico desde 0.1.0. Offline, embutido no binário.
- **UI por lentes** — 8 lentes (kanban, tabela, calendário, timeline, gantt, dashboard, grafo, documento) sobre um registry único; formulários derivam do schema. CO-387/396 plugam sem tocar despacho.
- **Auditoria de segurança no CI** — todo PR escaneado antes do merge; achados em `security_findings` (severidade, CWE, patch sugerido); crítico bloqueia release.
- **WELCOME.md** — onboarding completo + a invariante do pipeline (*localhost → aprovar → mesclar*) em git e jj.

### Detalhes

- Lentes (CO-393, user story): universo renderiza por lentes registráveis, manifest-driven — endurecido em review: 3 defeitos fatais de alcance corrigidos, boot verificado em navegador (zero erros JS).
- Segurança (CO-388, user story): postura Glasswing — varredura *antes* de superfície pública nova, dentro do CI de scrum existente; o check `security-audit` passou no próprio PR que o introduz.
- Docs (CO-403, task): exemplo CRUD vivo no universo miguel; correções factuais de roadmap no mesmo PR.
- CLI (CO-404, task): `include_str!` do CHANGELOG → cada `release-commit.sh` vira a próxima nota automaticamente.

### Referências

| Item | PR | Spec |
|---|---|---|
| CO-393 lentes | [#196](https://github.com/artelonga/co/pull/196) → `e00c88f` | `work/co/CO-393.md` |
| CO-388 segurança | [#195](https://github.com/artelonga/co/pull/195) | `work/co/CO-388.md` |
| CO-403 docs | [#197](https://github.com/artelonga/co/pull/197) → `d7a682c` | `work/co/CO-403.md` |
| CO-404 co updates | [#198](https://github.com/artelonga/co/pull/198) → `bc093ef` | `work/co/CO-404.md` |
