# Homomorphic encryption × functional programming — review for CO

> Written 2026-04-30 to answer: "can CO get a true zero-trust analytics tier via homomorphic encryption, expressed naturally as functional programming?" Honest answer: not in full, not yet — but a narrow slice **is** practical, and the FP framing makes the boundary precise instead of hand-wavy.

---

## 1. The honest framing

Two truths to hold simultaneously:

1. **Fully homomorphic encryption (FHE) is computationally impractical** for CO's analytics workload at v1-v3 scale. State-of-the-art TFHE on `u32` integers is ~10⁴–10⁵× slower than plaintext arithmetic; queries that take milliseconds on plaintext take minutes on FHE. Building CO's hot OLAP path (CO-123, CO-127) on FHE means the analytics surface is unusable.

2. **Partial HE (PHE) is practical and useful for narrow operations** — specifically for additive aggregations (Paillier) over encrypted counters, sums, time-spent. Ciphertext is ~256 bytes; one homomorphic add is ~1 ms. At CO's scale this is fine.

The CO design therefore needs a **two-tier privacy story**, not a one-size answer:

- **Tier A — Privileged compute zone** (Phase 4 / CO-130-132). General-case analytics: Flink decrypts inside an isolated, audited zone, k-anonymous aggregations exit, raw rows never leave. Trust is in the zone's hardening, not in cryptography.
- **Tier B — Partial HE for narrow counters** (Phase 4-5, this doc). Additive aggregations on encrypted values. Server never decrypts; only the user holds the key. True zero-trust for the ops PHE supports.

Tier A covers everything Tier B can't. Tier B is strictly stronger where it applies.

---

## 2. Why the FP framing matters

CO already speaks in `co-core` Rust traits and pure functions. The encryption boundary maps cleanly onto FP machinery — and the type system **enforces the boundary** so we never accidentally try to homomorphically compute something the underlying scheme doesn't support.

### 2.1 The algebraic structure

| FP structure | What it captures | Why it matters for HE |
|--------------|------------------|------------------------|
| **Monoid** | A set with associative `combine` and identity `empty` | If `(M, ⊕, 0)` is a monoid and the encryption preserves `⊕`, we can aggregate encrypted partial results in any order — perfect for distributed map-reduce |
| **Semigroup** | Like monoid but no identity | Same idea, slightly weaker |
| **Functor** | `map: (a → b) → f a → f b` | `enc` as a "functor": `map_enc: (a → b) → enc a → enc b` works **only** for HE-compatible `b ← a` |
| **Applicative** | `ap: f (a → b) → f a → f b` | Lets us compose multi-input HE operations cleanly when the scheme allows |
| **Additive PHE** (Paillier) | `enc(a) ⊞ enc(b) = enc(a + b)` | A monoid morphism from `(ℤₙ, +)` to ciphertext space |
| **Multiplicative PHE** (ElGamal) | `enc(a) ⊠ enc(b) = enc(a·b)` | Another monoid morphism, less commonly useful |
| **Levelled HE** (BGV/BFV/CKKS) | `+` and `×` up to a circuit depth `d` | A bounded-applicative — only finitely many operations before noise dominates |
| **FHE** (TFHE) | All circuits | Full functor, with massive cost |

### 2.2 The type system as the enforcer

In Rust we can express "this metric is computable on encrypted data" as a trait constraint, not a comment. Sketch:

```rust
/// A metric expressible on a single value.
pub trait Metric<T> {
    type Output;
    fn compute(values: &[T]) -> Self::Output;
}

/// A metric that's homomorphically additive — summable under PHE.
pub trait AdditiveHE<T>: Metric<T>
where
    T: Add<Output = T> + Copy,
{
    /// Combine encrypted partial results without decrypting.
    fn combine_enc(a: &Ciphertext, b: &Ciphertext) -> Ciphertext;
}

/// Sum is additive-HE-compatible.
impl<T: Add<Output = T> + Copy> AdditiveHE<T> for SumMetric { ... }

/// Median is NOT — does not implement AdditiveHE. Compiler enforces.
impl<T: Ord + Copy> Metric<T> for MedianMetric { ... }
```

Now the analytics planner takes `dyn AdditiveHE<_>` for the encrypted path and `dyn Metric<_>` for the privileged-zone path. **The trait bound, not a comment, decides which path a metric takes.** No more hand-waving about "this should be safe."

### 2.3 The metric DSL

The natural shape is a sum type (Rust enum) of metric variants, each tagged with its computability class:

```rust
pub enum MetricExpr {
    // Tier B — additive PHE on encrypted ciphertexts
    Count,
    Sum(FieldRef<Numeric>),
    Mean(FieldRef<Numeric>),         // = Sum / Count, both additive
    HistogramFixed(FieldRef<Numeric>, BinSpec),  // bucket counters, additive

    // Tier A — privileged zone only (require decryption)
    Median(FieldRef<Numeric>),
    Quantile(FieldRef<Numeric>, f64),
    UniqueCount(FieldRef<Any>),       // unless using HLL with HE-friendly merge
    GroupBy(FieldRef<Categorical>, Box<MetricExpr>),

    // Composition
    Compose(Box<MetricExpr>, Box<MetricExpr>, fn(f64, f64) -> f64),
}

impl MetricExpr {
    pub fn is_phe_decidable(&self) -> bool { ... }
    pub fn execution_plan(&self) -> Plan {
        if self.is_phe_decidable() { Plan::PartialHE }
        else { Plan::PrivilegedZone }
    }
}
```

The planner picks **per metric**, not per query. The user's analytics dashboard composes metrics; CO routes each to the right path automatically. The privacy guarantee surfaced to the user reflects the **strongest** path used in any query — if the dashboard uses one `Median`, the whole dashboard runs in the privileged zone.

---

## 3. Concrete Rust tooling

| Library | Scheme | Status | Use for |
|---------|--------|--------|---------|
| **[`tfhe-rs`](https://github.com/zama-ai/tfhe-rs)** (Zama) | TFHE (FHE on small ints + bool) | Mature, actively developed | Bounded FHE experiments — `u8`/`u16`/`u32` arithmetic, comparisons, MUX. Slow but usable for narrow workloads |
| **[`concrete`](https://github.com/zama-ai/concrete)** (Zama) | TFHE compiler from MLIR/Python | Mature, Python-facing | Prototyping; if a metric works here, port to `tfhe-rs` |
| **[`paillier-rs`](https://crates.io/crates/paillier)** family | Paillier (additive PHE) | Several crates exist; vet quality | **CO's main PHE choice for Tier B counters** |
| **[`elgamal_ristretto`](https://crates.io/crates/elgamal_ristretto)** etc. | ElGamal (multiplicative PHE) | Niche | Rare; mostly for set-membership protocols |
| **Microsoft SEAL** (via `sealcrypto-rs` FFI) | BGV / BFV / CKKS (levelled HE) | C++ canonical impl | Levelled HE prototypes; bounded-depth circuits |
| **[`tindercrypt`](https://crates.io/crates/tindercrypt)** | AEAD wrapper | — | Not HE; for the at-rest layer (CO-86) |

For CO Phase 4-5: start with **Paillier for counters** + **TFHE-rs for narrow experiments**. Skip Microsoft SEAL until a real BFV/CKKS workload presents itself.

---

## 4. Where partial HE actually pays off in CO

Concrete metrics where Paillier-based PHE gives real zero-trust value at acceptable cost:

| Metric | Decomposition | PHE feasibility | Realistic latency |
|--------|---------------|-----------------|--------------------|
| Per-universe view counter | encrypted increment per view; server homomorphic-adds; user decrypts total | ✅ Direct fit | <10 ms per view |
| Sum of file sizes | same shape | ✅ | <10 ms per upload |
| Total time-spent | session duration → encrypted sum | ✅ | <10 ms per session end |
| Count of N events matching predicate | predicate evaluated client-side; encrypted 1/0; homomorphic sum | ✅ if predicate is client-side | <50 ms per N events |
| Mean of values | ratio of two encrypted sums | ✅ (decrypt both client-side, divide) | <20 ms |
| Bucketed histogram | encrypted increment per bucket; aggregate per bucket | ✅ | <10 ms per event |
| Median / quantiles | requires sort | ❌ — falls to privileged zone | n/a |
| Unique count | requires set ops | ❌ generally — HLL with HE-friendly merge is research-grade | n/a |
| Top-N | requires sort + select | ❌ — privileged zone | n/a |

Roughly: **anything decomposable as "stream of encrypted increments + final user-side decrypt"** is a fit. Anything requiring comparison, sort, or unbounded set operations falls to Tier A.

---

## 5. Why FHE in the hot path is the wrong move (for now)

For CO's user-facing analytics dashboard, FHE on `tfhe-rs` would mean:

- A simple dashboard query touching 100k rows takes minutes-to-hours
- Ciphertext expansion (typically 1000-10000×) blows up R2 storage and Redpanda throughput
- Operational complexity (key management, parameter selection, noise budget) dwarfs the rest of the stack

What changes the calculus would be:
- **Hardware acceleration** for FHE (Intel's HE-Toolkit, Zama's Concrete-CPU) closing the gap to ~100× slowdown — under active development
- **Trusted Execution Environments** (Nitro Enclaves, Intel TDX, AMD SEV) — practical *today*, no cryptographic slowdown, but rests on hardware vendor trust (a different threat model)
- **MPC** (multi-party computation, e.g., MP-SPDZ) — splits trust across operators; works for narrow protocols (private set intersection, secure aggregation), not for general SQL

CO's pragmatic answer for v1-v3 is **the privileged compute zone (Tier A) + Paillier-based partial HE (Tier B)**. Re-evaluate FHE/TEE/MPC every 12 months; the field is moving.

---

## 6. Recommended phasing for CO

| Phase | What ships | Why this phase |
|-------|------------|----------------|
| **Phase 4** (CO-115) | Privileged zone (Tier A): isolation, audit, k-anonymity, allow-list, time-bounded keys | Necessary for "operator cannot read" to be defensible at all |
| **Phase 4 (parallel)** | Tier B narrow PHE: define `MetricExpr` + `AdditiveHE` trait, implement Paillier-based pipeline for **per-universe view counter** as the first proof | Low-risk, high-trust counter; no FHE complexity |
| **Phase 5** | Tier B expanded: sums, means, bucketed histograms; client-side key handling unified with the per-universe `K_u` from CO-86 | Build out as use cases emerge; don't pre-build |
| **Phase 5+ (research track)** | TFHE-rs spike: pick **one** narrow metric currently in Tier A, attempt it in TFHE, measure the cost. Decide whether to invest further | Provides a yearly recalibration on whether full-HE has become viable |
| **Phase 6+** | Re-evaluate TEE (Nitro Enclaves on Fargate), MPC, latest FHE benchmarks | Yearly review tied to the 6-month platform re-evaluation |

---

## 7. The new tickets implied (file later, not now)

When Phase 4 is closer, file as follow-ups to CO-115:

- **CO-FUTURE — `MetricExpr` DSL + `AdditiveHE` trait in `co-core`.** Pure, no integration; sets the type-system fence.
- **CO-FUTURE — Paillier-backed view counter, end-to-end.** Client encrypts increment, server homomorphic-adds, client decrypts total. First real Tier B metric.
- **CO-FUTURE — Metric planner: route `MetricExpr` to PHE path or privileged zone.** Wires the boundary.
- **CO-FUTURE — TFHE-rs research spike (yearly).** One metric, measure, document.

These are *not* filed now because the prerequisite (CO-86 envelope, CO-130 zone) doesn't exist yet. Filing them prematurely creates noise.

---

## 8. Bottom line

- "Achieve homomorphic encryption with functional programming" is the right framing, but the realistic answer is **partial HE, not full**, and the FP discipline is **using the type system to fence the boundary** — not to hide the fact that the boundary exists.
- The `MetricExpr` DSL + `AdditiveHE` trait pattern is the load-bearing engineering. It makes the privacy class of every metric visible at compile time. No metric quietly falls into the privileged zone by accident.
- For CO at Phase 4-5, **Paillier for narrow counters** is the practical PHE win. The privileged compute zone (CO-130) covers everything else honestly.
- FHE/TEE/MPC are research tracks. Yearly re-evaluation, not v1 commitments.

---

## 9. Citations

- TFHE-rs: https://github.com/zama-ai/tfhe-rs
- Concrete (Zama): https://github.com/zama-ai/concrete
- Paillier cryptosystem: https://en.wikipedia.org/wiki/Paillier_cryptosystem
- Microsoft SEAL: https://github.com/microsoft/SEAL
- BGV/BFV/CKKS overview: https://www.microsoft.com/en-us/research/project/microsoft-seal/
- AWS Nitro Enclaves: https://aws.amazon.com/ec2/nitro/nitro-enclaves/
- Apache Iceberg encryption (Phase 4 lake-side): https://iceberg.apache.org/spec/#encryption
- Platform evaluation Part III §22 (privacy guarantees as testable assertions): `docs/platform-evaluation.md`
