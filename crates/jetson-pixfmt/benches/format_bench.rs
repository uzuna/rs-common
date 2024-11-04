use criterion::{criterion_group, criterion_main, Criterion};
use jetson_pixfmt::{pixfmt::*, t16::*};

fn criterion_benchmark(c: &mut Criterion) {
    // 720pの画像データ
    let mut buf = vec![0x01_u8; 1280 * 720 * 2];

    cfg_if::cfg_if!(
        if #[cfg(feature = "as-short")] {
            c.bench_function("format use u16", |b| {
                b.iter(|| format_as_u16(&mut buf, CsiPixelFormat::Raw12))
            });
            c.bench_function("format use u64", |b| {
                b.iter(|| format_as_u64(&mut buf, CsiPixelFormat::Raw12))
            });

        }
    );

    c.bench_function("format use u128", |b| {
        b.iter(|| format_as_u128(&mut buf, CsiPixelFormat::Raw12))
    });
    c.bench_function("mask use u128", |b| {
        b.iter(|| mask_as_u128(&mut buf, CsiPixelFormat::Raw12))
    });

    {
        c.bench_function("format use 128 SSE2 or Neon", |b| {
            b.iter(|| unsafe { format_as_u128_simd(&mut buf, CsiPixelFormat::Raw12) })
        });
        c.bench_function("mask use 128 SSE2 or Neon", |b| {
            b.iter(|| unsafe { mask_as_u128_simd(&mut buf, CsiPixelFormat::Raw12) })
        });
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        c.bench_function("format use 256 AVX2", |b| {
            b.iter(|| unsafe { format_as_u256_simd(&mut buf, CsiPixelFormat::Raw12) })
        });

        c.bench_function("mask use 256 AVX2", |b| {
            b.iter(|| unsafe { mask_as_u256_simd(&mut buf, CsiPixelFormat::Raw12) })
        });
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
