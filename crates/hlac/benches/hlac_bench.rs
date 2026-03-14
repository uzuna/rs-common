use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use hlac::HlacExtractor;

fn make_binary_image(width: usize, height: usize) -> Vec<u8> {
    let mut image = vec![0_u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            image[idx] = if ((x * 131 + y * 73 + x * y) % 11) < 5 {
                1
            } else {
                0
            };
        }
    }
    image
}

fn bench_binary_scalar_vs_simd(c: &mut Criterion) {
    let extractor = HlacExtractor::new_binary_25();
    let sizes = [(128_usize, 128_usize), (512_usize, 512_usize)];

    let mut group = c.benchmark_group("binary_scalar_vs_simd");

    for (width, height) in sizes {
        let image = make_binary_image(width, height);
        group.throughput(Throughput::Elements((width * height) as u64));

        group.bench_with_input(
            BenchmarkId::new("scalar", format!("{}x{}", width, height)),
            &(width, height),
            |b, &(w, h)| {
                b.iter(|| {
                    let feature = extractor
                        .extract_binary_u8(black_box(&image), black_box(w), black_box(h))
                        .unwrap();
                    black_box(feature)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("simd_wide", format!("{}x{}", width, height)),
            &(width, height),
            |b, &(w, h)| {
                b.iter(|| {
                    let feature = extractor
                        .extract_binary_u8_simd(black_box(&image), black_box(w), black_box(h))
                        .unwrap();
                    black_box(feature)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_binary_scalar_vs_simd);
criterion_main!(benches);
