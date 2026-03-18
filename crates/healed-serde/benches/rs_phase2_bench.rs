//! Phase2向けのRSセグメント化ベンチマーク。
//!
//! `RsStrategy` のエンコード、全体デコード、範囲デコードを測定する。

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use healed_serde::rs::{RsStrategy, RS_DATA_BYTES_PER_SEGMENT};
use std::hint::black_box;

fn build_payload(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| ((index as u8).wrapping_mul(31)).wrapping_add(17))
        .collect()
}

fn benchmark_rs_phase2(c: &mut Criterion) {
    let mut group = c.benchmark_group("rs_phase2");
    let cases = [
        ("64KiB", 64 * 1024usize),
        ("256KiB", 256 * 1024usize),
        ("1MiB", 1024 * 1024usize),
    ];

    for (name, size) in cases {
        let payload = build_payload(size);
        let encoded = RsStrategy::encode_record(11, &payload)
            .expect("failed to pre-encode RS benchmark payload");

        let range_start = size / 3;
        let range_end = (range_start + RS_DATA_BYTES_PER_SEGMENT / 2).min(size);

        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(
            BenchmarkId::new("encode_record", name),
            &payload,
            |b, payload| {
                b.iter(|| {
                    RsStrategy::encode_record(black_box(11), black_box(payload))
                        .expect("encode_record failed")
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("decode_record", name),
            &encoded,
            |b, encoded| {
                b.iter(|| {
                    RsStrategy::decode_record(black_box(encoded)).expect("decode_record failed")
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("decode_payload_range", name),
            &encoded,
            |b, encoded| {
                b.iter(|| {
                    RsStrategy::decode_payload_range(black_box(encoded), range_start..range_end)
                        .expect("decode_payload_range failed")
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_rs_phase2);
criterion_main!(benches);
