//! `serde_json` を使用したシリアライズ/デシリアライズのパフォーマンスベンチマーク。
//!
//! このベンチマークは、`healed-serde` 自体の機能ではなく、
//! 比較対象として一般的なJSONシリアライズの性能を測定するために用意されています。
//! データセットのサイズ（small, medium, large）ごとに、
//! シリアライズ、デシリアライズ、およびその両方の処理時間を計測します。

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde::{Deserialize, Serialize};
use std::hint::black_box;

/// ベンチマーク用のJSONレコード。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonBenchRecord {
    id: u64,
    name: String,
    tags: Vec<String>,
    values: Vec<u64>,
    payload: String,
}

/// ベンチマーク用のJSONデータセット。複数のレコードを含む。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonBenchDataset {
    version: u32,
    records: Vec<JsonBenchRecord>,
}

/// 指定されたレコード数とペイロードサイズでテスト用のデータセットを構築する。
fn build_dataset(record_count: usize, payload_len: usize) -> JsonBenchDataset {
    let mut records = Vec::with_capacity(record_count);
    for i in 0..record_count {
        records.push(JsonBenchRecord {
            id: i as u64,
            name: format!("record-{i}"),
            tags: (0..6).map(|j| format!("tag-{i}-{j}")).collect(),
            values: (0..32).map(|j| (i as u64 + j as u64) * 17).collect(),
            payload: "x".repeat(payload_len),
        });
    }

    JsonBenchDataset {
        version: 1,
        records,
    }
}

/// `serde_json` を使用したシリアライズ/デシリアライズのパフォーマンスを測定するベンチマーク。
///
/// 以下の3つのシナリオを、データサイズを変えながら（small, medium, large）テストする:
/// - `to_vec`: オブジェクトからJSONバイト列へのシリアライズ。
/// - `from_slice`: JSONバイト列からオブジェクトへのデシリアライズ。
/// - `roundtrip`: シリアライズとデシリアライズの両方。
fn benchmark_json_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_serialize");
    let cases = [
        ("small", 32usize, 128usize),
        ("medium", 256usize, 512usize),
        ("large", 1024usize, 2048usize),
    ];

    for (name, record_count, payload_len) in cases {
        let dataset = build_dataset(record_count, payload_len);
        let encoded = serde_json::to_vec(&dataset).expect("failed to pre-serialize benchmark data");

        group.throughput(Throughput::Bytes(encoded.len() as u64));

        group.bench_with_input(BenchmarkId::new("to_vec", name), &dataset, |b, dataset| {
            b.iter(|| serde_json::to_vec(black_box(dataset)).expect("serialize failed"))
        });

        group.bench_with_input(
            BenchmarkId::new("from_slice", name),
            &encoded,
            |b, encoded| {
                b.iter(|| {
                    let decoded: JsonBenchDataset =
                        serde_json::from_slice(black_box(encoded)).expect("deserialize failed");
                    black_box(decoded)
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("roundtrip", name),
            &dataset,
            |b, dataset| {
                b.iter(|| {
                    let encoded = serde_json::to_vec(black_box(dataset)).expect("serialize failed");
                    let decoded: JsonBenchDataset =
                        serde_json::from_slice(black_box(&encoded)).expect("deserialize failed");
                    black_box(decoded)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_json_serialize);
criterion_main!(benches);
