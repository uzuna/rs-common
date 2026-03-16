//! Benchmarks for healed-serde byte-oriented ECC APIs (`encode`/`decode`).

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use healed_serde::{decode, encode, ProtectionLevel};
use std::hint::black_box;

fn build_payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i as u8).wrapping_mul(31)).wrapping_add(7))
        .collect()
}

fn benchmark_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_decode");

    let levels = [
        ("high", ProtectionLevel::High),
        ("medium", ProtectionLevel::Medium),
        ("low", ProtectionLevel::Low),
    ];

    let sizes = [
        ("4KiB", 4 * 1024usize),
        ("64KiB", 64 * 1024usize),
        ("256KiB", 256 * 1024usize),
    ];

    for (size_name, size) in sizes {
        let payload = build_payload(size);

        for (level_name, level) in levels {
            let case = format!("{level_name}_{size_name}");
            let encoded = encode(&payload, level).expect("failed to pre-encode benchmark payload");

            group.throughput(Throughput::Bytes(payload.len() as u64));

            group.bench_with_input(BenchmarkId::new("encode", &case), &payload, |b, payload| {
                b.iter(|| encode(black_box(payload), level).expect("encode failed"))
            });

            group.bench_with_input(BenchmarkId::new("decode", &case), &encoded, |b, encoded| {
                b.iter(|| decode(black_box(encoded)).expect("decode failed"))
            });

            group.bench_with_input(
                BenchmarkId::new("roundtrip", &case),
                &payload,
                |b, payload| {
                    b.iter(|| {
                        let encoded = encode(black_box(payload), level).expect("encode failed");
                        let decoded = decode(black_box(&encoded)).expect("decode failed");
                        black_box(decoded)
                    })
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, benchmark_encode_decode);
criterion_main!(benches);
