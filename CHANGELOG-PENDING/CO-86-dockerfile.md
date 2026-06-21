## CO-86 — Dockerfile: install libprotobuf-dev for well-known protos

Deploy hotfix. `proto/co_format.proto` (CO-86, the `.co` envelope) imports
`google/protobuf/struct.proto`. The builder image installed `protobuf-compiler`
but not `libprotobuf-dev`, which ships the well-known `.proto` files — so the
Docker build failed with `google/protobuf/struct.proto: File not found` even
though CI (host has the protos) was green. Added `libprotobuf-dev` to the
co-web builder stage. (Dockerfile-not-exercised-in-CI drift.)
