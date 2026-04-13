fn main() {
    prost_build::compile_protos(&["proto/entry.proto"], &["proto/"])
        .expect("Failed to compile proto/entry.proto");
}
