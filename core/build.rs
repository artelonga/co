fn main() {
    prost_build::compile_protos(&["proto/entry.proto"], &["proto/"])
        .expect("Failed to compile proto/entry.proto");
    prost_build::compile_protos(&["proto/co.proto", "proto/sync.proto"], &["proto/"])
        .expect("Failed to compile proto/sync.proto");
}
