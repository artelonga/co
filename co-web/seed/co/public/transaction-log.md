---
created: 2026-05-15T00:00:00+00:00
modified: 2026-05-15T00:00:00+00:00
order: 22
slug: transaction-log
tags:
- arquitetura
- transacao
- iceberg
- kafka
- flink
- pinot
title: 'Log de transações: eventos, snapshots, lakehouse'
type: page
---

# Log de transações

Modelo: **eventos imutáveis abaixo + snapshots por momento + estado atual no topo**. A camada inferior é a fonte da verdade; tudo acima é derivado e pode ser reconstruído.

```
┌──────────────────────────────────┐
│ entries (current snapshot)       │  ← latest state per (universe, path)
├──────────────────────────────────┤
│ states/<ts>.md (point-in-time)   │  ← universe-wide manifest snapshots
├──────────────────────────────────┤
│ entry_events (append-only log)   │  ← per-write event stream — source of truth
└──────────────────────────────────┘
```

## Camadas hoje

| Camada | Tabela / path | Imutável? | Granularidade |
|---|---|---|---|
| **Estado atual** | `entries` per universo | não — sobrescrita por PUT | última versão |
| **Snapshots** | `states/<ISO-ts>-<nanoid>.md` | sim por convenção | universo inteiro, capturado sob demanda |
| **Log de eventos** | `entry_events` (2.7.25) | **sim** — append-only | cada PUT / DELETE individual |
| Conteúdo dos blobs | `blobs/<body_hash>` | sim — content-addressable BLAKE3 | bytes do corpo |

## Por quê três camadas

1. **Eventos** — o que aconteceu. `(seq, ts_micros, op, path, body_hash, prev_body_hash, body, frontmatter, author, request_id)`. Toda CRUD entra em uma transação SQLite junto com o `UPDATE` do `entries`, então o log nunca diverge do estado.
2. **Snapshots** — captura manual de "como está agora" universe-wide. Útil para diffs entre dois momentos, propostas baseadas em estado, branches. Derivável dos eventos via replay.
3. **Estado** — query-friendly: a linha em `entries` é a versão mais recente. Derivável: `SELECT body FROM entry_events WHERE path=? ORDER BY ts_micros DESC LIMIT 1`.

## Garantias

- **Atomicidade**: `BEGIN; UPDATE entries; INSERT INTO entry_events; COMMIT;` — SQLite WAL torna isso transacionalmente seguro. Ou ambos comitam ou nenhum.
- **Ordering**: `seq INTEGER PRIMARY KEY AUTOINCREMENT` + `ts_micros INTEGER` — `seq` é monotônico dentro do universo; `ts_micros` é monotônico globalmente desde que o relógio do servidor não regrida (Fly + chrony combinam para garantir).
- **Idempotência**: coluna `request_id TEXT UNIQUE`. Retries do cliente carregando o mesmo `request_id` são rejeitados pelo `UNIQUE` constraint — sem dupla escrita.
- **Body addressing**: `body_hash` é BLAKE3 do corpo. Iguais ⇒ mesmo conteúdo. Não dependemos de timestamps para detectar mudanças reais.

## Time travel

- **Por entrada**: `GET /api/v1/universes/{u}/entries/{*path}/history` retorna o stream de eventos para aquele path. *(2.7.25)*
- **Snapshot universe-wide**: `POST /api/v1/universes/{u}/states` captura o estado atual. `GET /states/diff?from=&to=` compara duas capturas.
- **Reverter uma entrada**: cliente lê `/history`, pega o body anterior, PUTa de volta. A versão anterior vira a corrente; a história continua linear.

## Escalabilidade — ceiling atual

| Sistema | Limite | Quando estoura |
|---|---|---|
| SQLite WAL (per-universo) | ~1k–10k inserts/seg sustentados | Universo super-ativo (>100k writes/dia) |
| LiteFS leasing | 1 writer ativo por universo | OK — escolha consciente; outros writers leem replicações |
| Disco Fly volume | 1 GB padrão, 10 GB no quilombo | Mídia, não eventos (eventos: ~1KB cada, comprimem bem) |
| `entry_events` indices | `(path, ts_micros DESC)` é o hot path | Sempre OK; índices são B-tree |

**O modelo atual sustenta por anos a maioria dos universos** que CO hospeda (boards pessoais, comunidades de tamanho médio). Migração para lakehouse só faz sentido quando um único universo passa a gerar volumes que SQLite não acomoda — e mesmo assim, só esse universo migra; os outros continuam.

## Trajeto para Apache Iceberg / Pinot / Flink / Kafka

O log de eventos foi desenhado **para emitir** — protobuf como contrato, particionamento por universo+dia, ts_micros estritamente crescente, idempotência via request_id. Cada sistema downstream consome a mesma forma:

### Kafka — broker de eventos

Producer Rust (`rdkafka`) lê de `entry_events.seq > last_offset`, publica em tópico `co.entry-events.<universo>` (ou `co.entry-events` partitioned by universo). Payload: protobuf `co.v1.EntryEvent`.

```
co.entry-events
├── partition: hash(universe_key) mod N
├── key: universe_key
├── value: EntryEvent protobuf
└── headers: { op, author_id, request_id }
```

Aplicação não bloqueia em Kafka — worker assíncrono (mesmo padrão de `embedding_worker.rs`) processa o backlog e marca `entry_events.exported_at`.

### Flink — stream processor

Job Flink subscribe `co.entry-events`, deriva:
- **Materialized views**: contadores por universo, atividade por autor, top paths editados
- **Joins temporais**: links entre eventos do mesmo universo dentro de janelas
- **Output**: Iceberg (eventos brutos), Kafka tópicos derivados (views), PostgreSQL (dashboards)

### Apache Iceberg — table format na object storage

Stream → Parquet → Iceberg. Cada batch de N eventos vira um arquivo Parquet; catalog (Polaris / Nessie / Glue) registra. Iceberg dá:
- **Time travel SQL**: `SELECT … FROM co.entry_events FOR TIMESTAMP AS OF '2026-05-01'`
- **Schema evolution**: adicionar coluna nova ao proto não quebra leitores antigos
- **Hidden partitioning**: queries filtram por `universe_key` + `day(ts)` sem o usuário pensar em partições

Layout sugerido:
```
s3://co-events/
├── universe_key=co/
│   ├── day=2026-05-15/
│   │   ├── 00001-abc.parquet
│   │   └── 00002-def.parquet
│   └── day=2026-05-16/
└── universe_key=artelonga/
    └── day=2026-05-15/
```

### Apache Pinot — OLAP em real-time

Pinot consome:
- **Kafka direto**: ingestão real-time, query latency < 1s
- **Iceberg via connector**: histórico profundo, time travel queries

Use cases:
- Quantas edições por hora por universo?
- Quem editou `sobre.md` em janeiro?
- Distribuição de tamanho de body (P50, P95, P99) por universo

## Compatibilidade — checklist

| Propriedade | Estado | Necessário para |
|---|---|---|
| Schema formal (protobuf) | Em desenvolvimento (CO-150/151) | Iceberg + Flink + Pinot |
| Monotonic ordering (`seq`, `ts_micros`) | ✓ shipping em 2.7.25 | Kafka topic ordering, Iceberg snapshots |
| Idempotência (`request_id UNIQUE`) | ✓ shipping em 2.7.25 | Kafka retries sem duplicação |
| Particionamento explícito (universe + day) | Modelagem só | Iceberg partition spec |
| Content-addressable body (`body_hash`) | ✓ existente (BLAKE3) | Deduplicação no lakehouse |
| Trait de sink plugável | Próximo passo (depois do log local funcionar) | Trocar SQLite → Kafka sem mudar handlers |
| CDC export worker | Próximo passo | Aplicação não bloqueia em Kafka |

## Fases concretas

| Fase | Entrega | Status |
|---|---|---|
| 1 | `entry_events` table per universo + log de toda PUT/DELETE | shipping em 2.7.25 |
| 2 | Endpoint `/entries/{*path}/history` (time travel por entrada) | shipping em 2.7.25 |
| 3 | Endpoint `/entries/{*path}/undo` (revert ao body anterior) | follow-up |
| 4 | Schema protobuf `co.v1.EntryEvent` + trait `EventSink` | follow-up |
| 5 | KafkaSink (rdkafka), worker assíncrono | follow-up — quando Kafka for instalado |
| 6 | IcebergSink (Parquet via arrow-rs, catalog escolhido) | follow-up |
| 7 | Pinot conector + Flink jobs | depois do 5+6 |

Cada fase é shippable sozinha; as anteriores não bloqueiam as posteriores e vice-versa. O log de eventos local (fase 1) **já é** o contrato — Kafka/Iceberg leem do mesmo formato lógico.
