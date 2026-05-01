//! Parameterized fixture tests for the CO-61 sync protocol v1 reducer.
//!
//! Each fixture in `docs/sync-protocol-v1/fixtures/` specifies:
//!   - `input.base_hlc`, `input.ops_prod`, `input.ops_uat`
//!   - `expected.aplicadas`, `expected.conflitos`, `expected.novas_ops_remotas_ids`,
//!     `expected.blobs_solicitados`
//!
//! The test calls `mesclar(proposta_id, base_hlc, ops_prod, ops_uat)` and
//! compares the result with expected, ignoring generated `Conflito.id` values.

use co::sync::{Alvo, Hlc, Operacao, mesclar};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixture types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FixtureInput {
    proposta_id: Uuid,
    base_hlc: Hlc,
    ops_prod: Vec<Operacao>,
    ops_uat: Vec<Operacao>,
}

#[derive(Deserialize)]
struct FixtureConflitoCmp {
    op_local: Uuid,
    op_remota: Uuid,
    alvo: Alvo,
}

#[derive(Deserialize)]
struct FixtureExpected {
    aplicadas: Vec<Uuid>,
    conflitos: Vec<FixtureConflitoCmp>,
    novas_ops_remotas_ids: Vec<Uuid>,
    blobs_solicitados: Vec<co::sync::Sha256>,
}

#[derive(Deserialize)]
struct Fixture {
    description: String,
    input: FixtureInput,
    expected: FixtureExpected,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture_path(name: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
    PathBuf::from(manifest_dir)
        .parent()
        .expect("core/ must have a parent (workspace root)")
        .join("docs/sync-protocol-v1/fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> Fixture {
    let path = fixture_path(name);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"));
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

fn run_fixture(name: &str) {
    let f = load_fixture(name);
    let relatorio = mesclar(
        f.input.proposta_id,
        &f.input.base_hlc,
        &f.input.ops_prod,
        &f.input.ops_uat,
    );

    // Compare aplicadas as sets (order not guaranteed).
    let got_aplicadas: HashSet<Uuid> = relatorio.aplicadas.iter().copied().collect();
    let exp_aplicadas: HashSet<Uuid> = f.expected.aplicadas.iter().copied().collect();
    assert_eq!(
        got_aplicadas, exp_aplicadas,
        "[{name}] aplicadas mismatch: got {got_aplicadas:?}, expected {exp_aplicadas:?}"
    );

    // Compare novas_ops_remotas as id sets.
    let got_novas: HashSet<Uuid> = relatorio.novas_ops_remotas.iter().map(|op| op.id).collect();
    let exp_novas: HashSet<Uuid> = f.expected.novas_ops_remotas_ids.iter().copied().collect();
    assert_eq!(
        got_novas, exp_novas,
        "[{name}] novas_ops_remotas_ids mismatch"
    );

    // Compare blobs_solicitados as sets.
    let got_blobs: HashSet<&str> = relatorio
        .blobs_solicitados
        .iter()
        .map(|s| s.as_str())
        .collect();
    let exp_blobs: HashSet<&str> = f
        .expected
        .blobs_solicitados
        .iter()
        .map(|s| s.as_str())
        .collect();
    assert_eq!(got_blobs, exp_blobs, "[{name}] blobs_solicitados mismatch");

    // Compare conflitos by (op_local, op_remota, alvo) — ignore generated id.
    assert_eq!(
        relatorio.conflitos.len(),
        f.expected.conflitos.len(),
        "[{name}] conflito count mismatch: got {}, expected {}",
        relatorio.conflitos.len(),
        f.expected.conflitos.len()
    );
    let got_conflicts: HashSet<(Uuid, Uuid, String, String, Option<String>)> = relatorio
        .conflitos
        .iter()
        .map(|c| {
            (
                c.op_local,
                c.op_remota,
                c.alvo.tipo.clone(),
                c.alvo.id.clone(),
                c.alvo.campo.clone(),
            )
        })
        .collect();
    let exp_conflicts: HashSet<(Uuid, Uuid, String, String, Option<String>)> = f
        .expected
        .conflitos
        .iter()
        .map(|c| {
            (
                c.op_local,
                c.op_remota,
                c.alvo.tipo.clone(),
                c.alvo.id.clone(),
                c.alvo.campo.clone(),
            )
        })
        .collect();
    assert_eq!(
        got_conflicts, exp_conflicts,
        "[{name}] conflitos content mismatch"
    );

    // Verify opcoes and sugestao on every returned conflict.
    for c in &relatorio.conflitos {
        assert_eq!(
            c.opcoes,
            ["prod", "uat", "copia", "manual"],
            "[{name}] conflict opcoes wrong"
        );
        assert_eq!(
            c.sugestao, "prod",
            "[{name}] conflict sugestao must be prod"
        );
    }

    // Verify proposta_id is threaded through.
    assert_eq!(
        relatorio.proposta_id, f.input.proposta_id,
        "[{name}] proposta_id not preserved"
    );

    println!("[PASS] {name}: {}", f.description);
}

// ---------------------------------------------------------------------------
// Parameterized tests — one per fixture file
// ---------------------------------------------------------------------------

#[test]
fn fixture_01_clean_add() {
    run_fixture("01-clean-add.json");
}

#[test]
fn fixture_02_prod_advance() {
    run_fixture("02-prod-advance.json");
}

#[test]
fn fixture_03_same_slot_prod_wins() {
    run_fixture("03-same-slot-prod-wins.json");
}

#[test]
fn fixture_04_same_slot_uat_override() {
    run_fixture("04-same-slot-uat-override.json");
}

#[test]
fn fixture_05_copia() {
    run_fixture("05-copia.json");
}

#[test]
fn fixture_06_idempotent_retry() {
    run_fixture("06-idempotent-retry.json");
}

#[test]
fn fixture_07_recursive_resolution() {
    run_fixture("07-recursive-resolution.json");
}

#[test]
fn fixture_08_causality_respected() {
    run_fixture("08-causality-respected.json");
}

#[test]
fn fixture_09_schema_migration() {
    run_fixture("09-schema-migration.json");
}

#[test]
fn fixture_10_reversibility() {
    run_fixture("10-reversibility.json");
}
