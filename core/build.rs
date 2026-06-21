fn main() {
    prost_build::compile_protos(&["proto/entry.proto"], &["proto/"])
        .expect("Failed to compile proto/entry.proto");
    prost_build::compile_protos(&["proto/co.proto", "proto/sync.proto"], &["proto/"])
        .expect("Failed to compile proto/sync.proto");
    // CO-86 — the rich `.co` content envelope (separate package `co.format.v1`).
    prost_build::compile_protos(&["proto/co_format.proto"], &["proto/"])
        .expect("Failed to compile proto/co_format.proto");
}
