//! CO-86 — encode/decode throughput and compression-ratio benchmark for the
//! `.co` wire format. Run with `cargo bench -p co --bench co_format_bench`.

use co::co_format::{self, codec, telemetry};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

/// A representative content body resembling real CO markdown (frontmatter +
/// prose with repeated structure that compresses well).
fn sample_markdown(paragraphs: usize) -> String {
    let mut s = String::from(
        "---\ntitle: Documento de Exemplo\ntype: page\ntags:\n- exemplo\n- benchmark\nstatus: done\n---\n\n# Título\n\n",
    );
    for i in 0..paragraphs {
        s.push_str(&format!(
            "## Seção {i}\n\nUm parágrafo com conteúdo representativo, acentuação e \
             pontuação. Repetição estrutural ajuda a medir a razão de compressão. \
             Linha {i} de muitas linhas semelhantes neste documento.\n\n"
        ));
    }
    s
}

fn bench_encode_decode(c: &mut Criterion) {
    let md = sample_markdown(200); // ~ tens of KB
    let bytes_len = md.len() as u64;

    let mut group = c.benchmark_group("co_format");
    group.throughput(Throughput::Bytes(bytes_len));

    group.bench_function("encode_raw", |b| {
        b.iter(|| {
            let co = co_format::from_markdown(black_box(&md));
            black_box(co_format::to_bytes(&co))
        });
    });

    group.bench_function("encode_zstd", |b| {
        b.iter(|| {
            let mut co = co_format::from_markdown(black_box(&md));
            codec::compress_body(&mut co, codec::DEFAULT_LEVEL).unwrap();
            black_box(co_format::to_bytes(&co))
        });
    });

    let encoded = co_format::to_bytes(&co_format::from_markdown(&md));
    group.bench_function("decode_raw", |b| {
        b.iter(|| {
            let co = co_format::from_bytes(black_box(&encoded)).unwrap();
            black_box(co_format::to_markdown(&co).unwrap())
        });
    });

    group.finish();

    // Report the compression ratio once (informational, not a timed benchmark).
    let mut co = co_format::from_markdown(&md);
    codec::compress_body(&mut co, codec::DEFAULT_LEVEL).unwrap();
    telemetry::populate(&mut co, 0);
    if let Some(ratio) = telemetry::compression_ratio(&co) {
        let t = co.telemetry.as_ref().unwrap();
        println!(
            "compression ratio (zstd-{}): {ratio:.3} ({} → {} bytes)",
            codec::DEFAULT_LEVEL,
            t.size_uncompressed,
            t.size_compressed,
        );
    }
}

criterion_group!(benches, bench_encode_decode);
criterion_main!(benches);
