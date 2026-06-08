# Federated EDA — Cross-Deployment Event Bus Bridge

CO-384 — Federated event bus bridge for cross-deployment WebSocket pub/sub (CO ↔ Yggdrasil ↔ devices).

## Overview

CO-380 ships an in-process `EdaBus` (tokio broadcast channel). CO-384 adds a **bridge layer** that federates events across deployment boundaries over persistent WebSocket connections.

**Topology: star-with-bridges.**
Each deployment runs its own bus. Bridges connect pairs of deployments. Devices connect to one bus (their primary deployment). No N×N mesh.

```
┌────────────────┐    bridge WS    ┌──────────────────┐
│  CO (Fly, gru) │◄──────────────►│ Yggdrasil (Fly)  │
│  EdaBus        │                 │ EdaBus            │
└────────┬───────┘                 └─────────┬────────┘
         │ /api/v1/events                    │ /api/v1/events
         │ (per-user WS)                     │
   ┌─────┴──────┐                      ┌─────┴──────┐
   │   mobile   │                      │   Godot    │
   │   browser  │                      │   client   │
   └────────────┘                      └────────────┘
```

## Protocol

### Endpoint

```
GET /api/v1/events/bridge?source=<host>&token=<jwt>
Upgrade: websocket
Sec-WebSocket-Protocol: co.eda.bridge.v1
```

### Message format

Two message types exchanged as JSON text frames:

**ReplayRequest** — sent by the connecting side immediately after upgrade:

```json
{
  "bridge_msg_type": "ReplayRequest",
  "last_received_id": "01J0ABCDEFGHIJKLMNOPQRSTU"
}
```

`last_received_id` is the ULID of the last event received from the remote on the previous session. The server replays all events with `id > last_received_id` from `event_log`. Pass `null` to replay all available events (up to 1000).

**Event** — federated event forwarded across the bridge:

```json
{
  "bridge_msg_type": "Event",
  "event": {
    "id": "01J0ABCDEFGHIJKLMNOPQRSTU",
    "event_type": "entry.created",
    "universe_key": "my-universe",
    "user_id": "usr-123",
    "payload": { "path": "notes/foo.md" },
    "visibility": "public",
    "created_at": "2026-06-08T12:00:00Z"
  },
  "origin_deployment": "yggdrasil.artelonga.com.br",
  "signed_by": "yggdrasil.artelonga.com.br",
  "bridge_received_at": "2026-06-08T12:00:00.050Z",
  "hop_count": 0
}
```

### Heartbeat

Server sends a WebSocket `Ping` frame every 30 seconds (configurable via `CO_BRIDGE_HEARTBEAT_S`). Client must respond with `Pong`. Missing pong → connection closed → reconnect with backoff.

### Reconnect (connecting side)

Exponential backoff: 1s → 2s → 4s → 8s → 16s → max 30s. Resets to 1s after a successful connection.

## Privacy rules

Enforced at the bridge layer before forwarding. No exceptions.

| Visibility | Federated? | Reason |
|---|---|---|
| `Public` | ✅ Yes | Safe for all consumers |
| `UniverseMembers` | ✅ Yes | Shared by members |
| `UniverseOwner` | ❌ No | Owner-only; target may not have credentials |
| `UserOnly` | ❌ No | User-scoped; never crosses deployment boundary |
| `System` | ❌ No | Internal telemetry; never forwarded |

## Loop guard

Each event carries a `hop_count`. The bridge increments it before republishing. Events with `hop_count > 3` are silently dropped. This prevents infinite loops in topologies where two deployments both subscribe to each other.

## Trust model

### Inbound (accepting connections)

The server checks `CO_BRIDGE_TRUSTED_SOURCES` before upgrading. Unknown sources receive HTTP 403.

```bash
# Allow Yggdrasil to connect to CO
flyctl secrets set CO_BRIDGE_TRUSTED_SOURCES="yggdrasil.artelonga.com.br" -a co-artelonga

# Multiple sources (comma-separated)
flyctl secrets set CO_BRIDGE_TRUSTED_SOURCES="yggdrasil.artelonga.com.br,quilombo.artelonga.com.br" -a co-artelonga
```

Default: empty string (rejects all inbound connections).

### Outbound (initiating connections)

`CO_BRIDGE_OUTBOUND_TOKENS_JSON` maps destination host → JWT token. CO connects to each destination at startup and reconnects with backoff on drop.

```bash
# CO connects to Yggdrasil
flyctl secrets set CO_BRIDGE_OUTBOUND_TOKENS_JSON='{"yggdrasil.artelonga.com.br":"<jwt>"}' -a co-artelonga
```

Default: empty / not set (no outbound connections initiated).

### Yggdrasil side (matching pair)

```bash
# Allow CO to connect to Yggdrasil
flyctl secrets set YGG_BRIDGE_TRUSTED_SOURCES="co.artelonga.com.br" -a yggdrasil-artelonga
flyctl secrets set YGG_BRIDGE_OUTBOUND_TOKENS_JSON='{"co.artelonga.com.br":"<jwt>"}' -a yggdrasil-artelonga
```

## Configuration reference

| Env var | Default | Description |
|---|---|---|
| `CO_BRIDGE_TRUSTED_SOURCES` | `""` (empty → reject all) | Comma-separated list of trusted inbound source hosts |
| `CO_BRIDGE_OUTBOUND_TOKENS_JSON` | not set (no outbound) | JSON map: `{"<host>": "<jwt>"}` — destinations to connect to at startup |
| `CO_BRIDGE_HEARTBEAT_S` | `30` | Ping interval in seconds |
| `CO_DEPLOYMENT_ID` | `FLY_APP_NAME` or `"co-local"` | Our own deployment identifier (used as `origin_deployment`) |

## Bridge state (database)

Migration v65 adds `bridge_state`:

```sql
CREATE TABLE bridge_state (
    id                      TEXT PRIMARY KEY,        -- '<source>:<target>'
    source_deployment       TEXT NOT NULL,
    target_deployment       TEXT NOT NULL,
    last_delivered_event_id TEXT,                    -- ULID of last ACK'd event
    last_connected_at       TEXT,
    last_disconnected_at    TEXT,
    state                   TEXT NOT NULL
      CHECK (state IN ('connected','disconnected','degraded'))
);
```

`last_delivered_event_id` is updated on disconnect so reconnect replay starts from the right point.

## Telemetry events

All published to the local bus with `Visibility::Public` — visible in `/agora`.

| Event type | When |
|---|---|
| `bridge.connected` | New WS connection established |
| `bridge.disconnected` | WS connection closed |
| `bridge.event_received` | Inbound federated event processed |
| `bridge.event_sent` | Local event forwarded to remote |
| `bridge.replay_completed` | Replay batch finished on reconnect |

Payload examples:

```json
{ "source": "yggdrasil.artelonga.com.br", "target": "co-artelonga" }
{ "source": "yggdrasil.artelonga.com.br", "events_count": 42 }
```

## Backpressure

The local `TokioBroadcastBus` has a 4096-event capacity. Slow bridges drop events (broadcast receiver lag) and trigger replay on reconnect. The `AtividadesPersistor` (CO-380) persists every event to `event_log` so replay is always possible for events within the 30-day retention window.

## Failure modes

| Mode | Behavior |
|---|---|
| Remote deployment down | Bridge marks `disconnected`, reconnects with backoff, replays on next connect |
| Network partition | Same as remote down |
| Slow consumer (bridge) | Bus drops lagged events; replay fills the gap on reconnect |
| Untrusted source | HTTP 403 at handshake; no WS upgrade |
| Loop (A→B→A) | `hop_count > 3` drops event |
| Invalid message format | Warn + skip; connection stays open |

## Not in scope (v3.0)

- **Full local-first offline model** — CO-386 (v3.1)
- **CRUD conflict UI** — CO-385 (v3.1)
- **Multi-hop routing** (A→B→C beyond hop_count=3 limit) — v3.2
- **End-to-end encryption** of bridged events — Wave 6 (CO-145)
- **Federated identity** beyond shared-secret trust — Wave 6+
- **Cross-organization federation** (third-party deployments) — defer
