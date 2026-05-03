//! Sync Protocol — op log + content-addressed blobs + 3-way merge,
//! and CO-151 protobuf SyncDelta/SyncBatch wire format.

/// CO-151: protobuf delta encode/decode + zstd compression.
pub mod delta;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256 as Sha256Hasher};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Hybrid Logical Clock per Kulkarni et al. 2014.
/// Serialized as `{wall_ms}:{counter}:{node_hex32}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hlc {
    pub wall_ms: u64,
    pub counter: u32,
    /// Node identity as u128 (same bits as the node's UUID).
    pub node: u128,
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.wall_ms
            .cmp(&other.wall_ms)
            .then(self.counter.cmp(&other.counter))
            .then(self.node.cmp(&other.node))
    }
}

impl std::fmt::Display for Hlc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{:032x}", self.wall_ms, self.counter, self.node)
    }
}

impl std::str::FromStr for Hlc {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        if parts.len() != 3 {
            return Err(format!(
                "invalid HLC: expected 3 colon-separated parts in {s:?}"
            ));
        }
        let wall_ms = parts[0]
            .parse::<u64>()
            .map_err(|e| format!("invalid wall_ms: {e}"))?;
        let counter = parts[1]
            .parse::<u32>()
            .map_err(|e| format!("invalid counter: {e}"))?;
        let node =
            u128::from_str_radix(parts[2], 16).map_err(|e| format!("invalid node hex: {e}"))?;
        Ok(Hlc {
            wall_ms,
            counter,
            node,
        })
    }
}

impl Serialize for Hlc {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Hlc {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<Hlc>().map_err(serde::de::Error::custom)
    }
}

/// Actor identity: stable node installation + optional authenticated user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ator {
    pub node_id: Uuid,
    pub user_id: Option<Uuid>,
}

/// The addressed entity + optional field for an operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Alvo {
    pub tipo: String,
    pub id: String,
    pub campo: Option<String>,
}

/// Base64-encoded Ed25519 signature — reserved for federation (v1.1).
pub type Ed25519Sig = String;

/// SHA-256 hex digest identifying a content-addressed blob.
pub type Sha256 = String;

/// A single immutable operation in the op log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operacao {
    pub id: Uuid,
    pub hlc: Hlc,
    pub ator: Ator,
    /// Namespaced op type, e.g. "foto.adicionar", "conteudo.atualizar".
    pub tipo: String,
    pub alvo: Alvo,
    pub args: serde_json::Value,
    /// Causal parents — typically 1, multiple for merge ops.
    pub pai: Vec<Uuid>,
    pub assinatura: Option<Ed25519Sig>,
}

/// Peer state summary for divergence detection without full op transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifesto {
    pub node_id: Uuid,
    pub hlc_atual: Hlc,
    pub ops_count: u64,
    #[serde(with = "hex_bytes_32")]
    pub ops_merkle_root: [u8; 32],
    pub blobs_count: u64,
    #[serde(with = "hex_bytes_32")]
    pub blobs_merkle_root: [u8; 32],
    pub schema_version: String,
    pub protocol_version: String,
}

/// Sync proposal — a "pull request" from sender to target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposta {
    pub id: Uuid,
    pub peer_origem: Uuid,
    pub base_hlc: Hlc,
    pub ops: Vec<Operacao>,
    pub blobs_disponiveis: Vec<Sha256>,
    pub criado_em: chrono::DateTime<chrono::Utc>,
}

/// A detected conflict between concurrent ops targeting the same entity/field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflito {
    /// Deterministic ID: SHA-256 of sorted op IDs, first 16 bytes as UUID.
    pub id: Uuid,
    pub op_local: Uuid,
    pub op_remota: Uuid,
    pub alvo: Alvo,
    pub opcoes: Vec<String>,
    pub sugestao: String,
}

/// Merge report returned after processing a Proposta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatorioMesclagem {
    pub proposta_id: Uuid,
    /// Remote (sender) ops that merged cleanly.
    pub aplicadas: Vec<Uuid>,
    /// Remote ops in conflict with local post-base ops.
    pub conflitos: Vec<Conflito>,
    /// Local (target/prod) ops since base_hlc — returned for sender's display.
    pub novas_ops_remotas: Vec<Operacao>,
    /// Blobs referenced by applied ops that the target needs fetched.
    pub blobs_solicitados: Vec<Sha256>,
}

/// Core sync abstraction for implementors (HTTP server, in-memory store, etc.).
pub trait SyncProtocol {
    fn manifesto(&self) -> Manifesto;
    fn ops_since(&self, since: &Hlc) -> Vec<Operacao>;
    fn aplicar_proposta(&mut self, proposta: Proposta) -> RelatorioMesclagem;
}

/// Compute the 3-way merge of `ops_remota` (sender/UAT) against `ops_local`
/// (target/prod), given the last shared HLC `base_hlc`.
///
/// Calls `mesclar_com_blobs` with an empty local blob store.
pub fn mesclar(
    proposta_id: Uuid,
    base_hlc: &Hlc,
    ops_local: &[Operacao],
    ops_remota: &[Operacao],
) -> RelatorioMesclagem {
    mesclar_com_blobs(proposta_id, base_hlc, ops_local, ops_remota, &[])
}

/// Merge with an explicit set of locally-available blob digests.
/// Blobs already present locally are excluded from `blobs_solicitados`.
pub fn mesclar_com_blobs(
    proposta_id: Uuid,
    base_hlc: &Hlc,
    ops_local: &[Operacao],
    ops_remota: &[Operacao],
    blobs_locais: &[Sha256],
) -> RelatorioMesclagem {
    // Deduplicate remote ops by ID — idempotent re-submission of same proposta.
    let mut seen = HashSet::new();
    let ops_remota: Vec<&Operacao> = ops_remota.iter().filter(|op| seen.insert(op.id)).collect();

    // Local ops that are strictly after the shared base.
    let ops_local_pos_base: Vec<&Operacao> =
        ops_local.iter().filter(|op| &op.hlc > base_hlc).collect();

    // Combined op lookup for causality walks.
    let all_ops: HashMap<Uuid, &Operacao> = ops_local
        .iter()
        .chain(ops_remota.iter().copied())
        .map(|op| (op.id, op))
        .collect();

    let mut aplicadas = Vec::new();
    let mut conflitos = Vec::new();

    for r in &ops_remota {
        let concorrentes: Vec<&&Operacao> = ops_local_pos_base
            .iter()
            .filter(|l| {
                l.alvo == r.alvo
                    && !causal_ancestor(l.id, r, &all_ops)
                    && !causal_ancestor(r.id, l, &all_ops)
            })
            .collect();

        if concorrentes.is_empty() {
            aplicadas.push(r.id);
        } else {
            let first_local = concorrentes[0];
            let conflito_id = conflito_id_de(r.id, concorrentes.iter().map(|&&op| op.id).collect());
            conflitos.push(Conflito {
                id: conflito_id,
                op_local: first_local.id,
                op_remota: r.id,
                alvo: r.alvo.clone(),
                opcoes: vec![
                    "prod".to_string(),
                    "uat".to_string(),
                    "copia".to_string(),
                    "manual".to_string(),
                ],
                sugestao: "prod".to_string(),
            });
        }
    }

    // Request blobs referenced by applied foto.adicionar ops not already local.
    let blobs_solicitados: Vec<Sha256> = aplicadas
        .iter()
        .filter_map(|id| ops_remota.iter().copied().find(|op| op.id == *id))
        .filter(|op| op.tipo == "foto.adicionar")
        .filter_map(|op| {
            op.args
                .get("sha256")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|sha| !blobs_locais.contains(sha))
        .collect();

    RelatorioMesclagem {
        proposta_id,
        aplicadas,
        conflitos,
        novas_ops_remotas: ops_local_pos_base.into_iter().cloned().collect(),
        blobs_solicitados,
    }
}

/// Returns true if `op_a_id` is a transitive causal ancestor of `op_b`.
/// Walks `op_b.pai` recursively through `all_ops`.
fn causal_ancestor(op_a_id: Uuid, op_b: &Operacao, all_ops: &HashMap<Uuid, &Operacao>) -> bool {
    let mut visited: HashSet<Uuid> = HashSet::new();
    let mut queue: Vec<Uuid> = op_b.pai.clone();
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        if id == op_a_id {
            return true;
        }
        if let Some(op) = all_ops.get(&id) {
            queue.extend_from_slice(&op.pai);
        }
    }
    false
}

/// Deterministic conflict ID: SHA-256 of sorted op IDs, first 16 bytes as UUID v5.
fn conflito_id_de(remote_id: Uuid, mut local_ids: Vec<Uuid>) -> Uuid {
    local_ids.push(remote_id);
    local_ids.sort();
    let mut hasher = Sha256Hasher::new();
    for id in &local_ids {
        hasher.update(id.as_bytes());
    }
    let hash = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50; // version 5
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    Uuid::from_bytes(bytes)
}

mod hex_bytes_32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        serializer.serialize_str(&hex)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(deserializer)?;
        if s.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "expected 64 hex chars, got {}",
                s.len()
            )));
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex_str = std::str::from_utf8(chunk).map_err(serde::de::Error::custom)?;
            bytes[i] = u8::from_str_radix(hex_str, 16).map_err(serde::de::Error::custom)?;
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn hlc_display_and_parse_roundtrip() {
        let hlc = Hlc {
            wall_ms: 1_714_500_000_000,
            counter: 42,
            node: 1,
        };
        let s = hlc.to_string();
        assert_eq!(s, "1714500000000:42:00000000000000000000000000000001");
        let parsed: Hlc = s.parse().unwrap();
        assert_eq!(hlc, parsed);
    }

    #[test]
    fn hlc_ordering_by_wall_then_counter_then_node() {
        let a = Hlc {
            wall_ms: 1000,
            counter: 0,
            node: 1,
        };
        let b = Hlc {
            wall_ms: 2000,
            counter: 0,
            node: 1,
        };
        let c = Hlc {
            wall_ms: 1000,
            counter: 1,
            node: 1,
        };
        let d = Hlc {
            wall_ms: 1000,
            counter: 0,
            node: 2,
        };
        assert!(a < b, "higher wall wins");
        assert!(a < c, "higher counter wins at same wall");
        assert!(a < d, "higher node breaks final tie");
    }

    #[test]
    fn hlc_serde_roundtrip() {
        let hlc = Hlc {
            wall_ms: 999,
            counter: 7,
            node: 0xdead,
        };
        let json = serde_json::to_string(&hlc).unwrap();
        let back: Hlc = serde_json::from_str(&json).unwrap();
        assert_eq!(hlc, back);
    }

    #[test]
    fn causal_ancestor_direct() {
        let node = Uuid::nil();
        let ator = Ator {
            node_id: node,
            user_id: None,
        };
        let hlc = Hlc {
            wall_ms: 1,
            counter: 0,
            node: 0,
        };
        let alvo = Alvo {
            tipo: "t".into(),
            id: "1".into(),
            campo: None,
        };

        let op_a = Operacao {
            id: Uuid::from_u128(1),
            hlc: hlc.clone(),
            ator: ator.clone(),
            tipo: "x".into(),
            alvo: alvo.clone(),
            args: serde_json::Value::Null,
            pai: vec![],
            assinatura: None,
        };
        let op_b = Operacao {
            id: Uuid::from_u128(2),
            hlc,
            ator,
            tipo: "x".into(),
            alvo,
            args: serde_json::Value::Null,
            pai: vec![Uuid::from_u128(1)],
            assinatura: None,
        };
        let all_ops: HashMap<Uuid, &Operacao> =
            [(op_a.id, &op_a), (op_b.id, &op_b)].into_iter().collect();
        assert!(causal_ancestor(op_a.id, &op_b, &all_ops));
        assert!(!causal_ancestor(op_b.id, &op_a, &all_ops));
    }

    #[test]
    fn conflito_id_is_deterministic() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(conflito_id_de(a, vec![b]), conflito_id_de(b, vec![a]));
    }
}
