//! ピクセルフォーマット変換モジュール
//!
//! 各種カメラフォーマット (YUYV, NV12, RGB24) を AprilTag 検出に必要な
//! グレースケール (Luma8) 形式に変換する。

use wide::u16x16;

/// Luma8 への変換トレイト
pub trait PixelConverter {
    /// `src` バッファを Luma8 に変換して `dst` に格納する
    ///
    /// * `src`    - 入力ピクセルデータ
    /// * `width`  - 画像幅 [px]
    /// * `height` - 画像高 [px]
    /// * `dst`    - 出力バッファ (width × height バイト)
    fn to_luma8(&self, src: &[u8], width: u32, height: u32, dst: &mut Vec<u8>);
}

/// YUYV フォーマット変換器
///
/// YUYV (YUV 4:2:2 packed) の Y チャネルのみを抽出してグレースケール化する。
/// LLVM の自動ベクトル化に任せることで 、target-cpu=native 時に SIMD 最適化が適用される。
pub struct YuyvConverter;

impl PixelConverter for YuyvConverter {
    fn to_luma8(&self, src: &[u8], width: u32, height: u32, dst: &mut Vec<u8>) {
        let n = (width * height) as usize;
        dst.resize(n, 0);
        yuyv_to_luma8(src, dst);
    }
}

/// YUYV → Luma8 変換
///
/// YUYV バイト列 `[Y0, U0, Y1, V0, ...]` から Y 成分だけ取り出す。
/// チャンク単位ループは LLVM が自動ベクトル化する。
pub fn yuyv_to_luma8(src: &[u8], dst: &mut [u8]) {
    assert_eq!(
        src.len(),
        dst.len() * 2,
        "YUYV src は dst の 2 倍のサイズが必要"
    );
    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
        *d = s[0]; // Y チャネルのみ抽出
    }
}

/// NV12 フォーマット変換器
///
/// NV12 の Y プレーン (先頭 W×H バイト) をそのまま Luma8 として使う。
/// コピー操作のみのためほぼゼロコストで変換できる。
pub struct Nv12Converter;

impl PixelConverter for Nv12Converter {
    fn to_luma8(&self, src: &[u8], width: u32, height: u32, dst: &mut Vec<u8>) {
        let n = (width * height) as usize;
        dst.resize(n, 0);
        dst.copy_from_slice(nv12_luma_plane(src, width, height));
    }
}

/// NV12 の Y プレーンへの参照を返す
///
/// NV12 レイアウト: `[Y plane (W×H bytes)] + [UV plane (W×H/2 bytes)]`
/// 先頭 W×H バイトが直接 Luma8 として利用可能。
pub fn nv12_luma_plane(src: &[u8], width: u32, height: u32) -> &[u8] {
    let n = (width * height) as usize;
    assert!(src.len() >= n, "NV12 src のサイズが不足");
    &src[..n]
}

/// RGB24 フォーマット変換器
///
/// `wide::u16x16` による明示的 SIMD で BT.601 輝度変換を行う。
pub struct RgbConverter;

impl PixelConverter for RgbConverter {
    fn to_luma8(&self, src: &[u8], _width: u32, _height: u32, dst: &mut Vec<u8>) {
        let n = src.len() / 3;
        dst.resize(n, 0);
        rgb_to_luma8_wide(src, dst);
    }
}

/// RGB24 → Luma8 変換 (BT.601)
///
/// Y = (R×77 + G×150 + B×29) >> 8
/// `wide::u16x16` で 16 ピクセルを並列処理し、端数はスカラーフォールバック。
pub fn rgb_to_luma8_wide(src: &[u8], dst: &mut [u8]) {
    const LANE: usize = 16;
    let pixel_count = src.len() / 3;
    assert!(dst.len() >= pixel_count, "dst バッファが不足");

    // 端数以外を SIMD で処理
    let simd_count = pixel_count / LANE * LANE;

    // R, G, B チャネルを別々のバッファに展開してから SIMD 処理
    // wide は格納順 (AoS → SoA) の変換を明示的に行う必要がある
    let mut r_buf = [0u16; LANE];
    let mut g_buf = [0u16; LANE];
    let mut b_buf = [0u16; LANE];

    for chunk_idx in (0..simd_count).step_by(LANE) {
        // 16 ピクセル分を展開
        for i in 0..LANE {
            let base = (chunk_idx + i) * 3;
            r_buf[i] = src[base] as u16;
            g_buf[i] = src[base + 1] as u16;
            b_buf[i] = src[base + 2] as u16;
        }

        let r = u16x16::new(r_buf);
        let g = u16x16::new(g_buf);
        let b = u16x16::new(b_buf);

        // BT.601: Y = (R × 77 + G × 150 + B × 29) >> 8
        // wide::u16x16 の Shr は scalor (u16) のみ対応のため splat ではなく直接 8u16 を使う
        let y: u16x16 = (r * u16x16::splat(77) + g * u16x16::splat(150) + b * u16x16::splat(29))
            >> 8u16;

        let y_arr = y.to_array();
        for i in 0..LANE {
            dst[chunk_idx + i] = y_arr[i].min(255) as u8;
        }
    }

    // 端数スカラー処理
    for i in simd_count..pixel_count {
        let base = i * 3;
        let r = src[base] as u32;
        let g = src[base + 1] as u32;
        let b = src[base + 2] as u32;
        dst[i] = ((r * 77 + g * 150 + b * 29) >> 8).min(255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- 値域確認 ---

    #[test]
    fn test_luma_range() {
        // 全入力範囲 (0–255) で出力が 0–255 に収まることを確認
        let cases: &[(u8, u8, u8)] = &[(0, 0, 0), (255, 255, 255), (255, 0, 0), (0, 255, 0), (0, 0, 255)];
        for &(r, g, b) in cases {
            let src = [r, g, b];
            let mut dst = [0u8; 1];
            rgb_to_luma8_wide(&src, &mut dst);
            // 出力は 0–255 の範囲内 (u8 なので自明だが境界値も確認)
            let _ = dst[0]; // パニックしなければOK
        }
    }

    // --- 正常系 ---

    #[test]
    fn test_yuyv_to_luma8_known() {
        // Y0=100, Y1=200 のピクセルが正しく抽出されることを確認
        let src = [100u8, 50, 200, 120]; // [Y0, U0, Y1, V0]
        let mut dst = [0u8; 2];
        yuyv_to_luma8(&src, &mut dst);
        assert_eq!(dst, [100, 200]);
    }

    #[test]
    fn test_rgb_to_luma8_black() {
        // 黒 (0,0,0) → 0
        let src = [0u8, 0, 0];
        let mut dst = [0u8; 1];
        rgb_to_luma8_wide(&src, &mut dst);
        assert_eq!(dst[0], 0);
    }

    #[test]
    fn test_rgb_to_luma8_white() {
        // 白 (255,255,255) → 255
        let src = [255u8, 255, 255];
        let mut dst = [0u8; 1];
        rgb_to_luma8_wide(&src, &mut dst);
        assert_eq!(dst[0], 255);
    }

    #[test]
    fn test_rgb_to_luma8_known_values() {
        // BT.601 輝度の既知値を検証
        // 純赤 (255,0,0): Y = (255*77) >> 8 = 76
        // 純緑 (0,255,0): Y = (255*150) >> 8 = 149
        // 純青 (0,0,255): Y = (255*29) >> 8 = 28
        let cases: &[(u8, u8, u8, u8)] = &[
            (255, 0, 0, 76),
            (0, 255, 0, 149),
            (0, 0, 255, 28),
        ];
        for &(r, g, b, expected) in cases {
            let src = [r, g, b];
            let mut dst = [0u8; 1];
            rgb_to_luma8_wide(&src, &mut dst);
            assert_eq!(dst[0], expected, "RGB({r},{g},{b}) の輝度が不正");
        }
    }

    #[test]
    fn test_rgb_to_luma8_simd_vs_scalar() {
        // SIMD 処理 (16 ピクセルまとめ) とスカラー処理の結果が一致することを確認
        let pixels: Vec<(u8, u8, u8)> = (0u8..=15).map(|i| (i * 16, i * 8, 255 - i * 16)).collect();
        let src: Vec<u8> = pixels.iter().flat_map(|&(r, g, b)| [r, g, b]).collect();
        let mut simd_dst = vec![0u8; 16];
        rgb_to_luma8_wide(&src, &mut simd_dst);

        // スカラー参照実装
        let scalar_dst: Vec<u8> = pixels.iter().map(|&(r, g, b)| {
            let r = r as u32;
            let g = g as u32;
            let b = b as u32;
            ((r * 77 + g * 150 + b * 29) >> 8).min(255) as u8
        }).collect();

        assert_eq!(simd_dst, scalar_dst, "SIMD とスカラーの結果が異なる");
    }

    #[test]
    fn test_nv12_luma_plane() {
        // NV12 の Y プレーンが先頭 W×H バイトであることを確認
        let width = 4u32;
        let height = 2u32;
        let mut data = vec![0u8; (width * height + width * height / 2) as usize];
        // Y プレーンを識別可能な値で埋める
        for i in 0..(width * height) as usize {
            data[i] = i as u8;
        }
        let luma = nv12_luma_plane(&data, width, height);
        assert_eq!(luma.len(), (width * height) as usize);
        assert_eq!(luma[0], 0);
        assert_eq!(luma[7], 7);
    }

    // --- 異常系 ---

    #[test]
    #[should_panic(expected = "YUYV src は dst の 2 倍のサイズが必要")]
    fn test_yuyv_wrong_size() {
        let src = [0u8; 3]; // 奇数バイト (不正)
        let mut dst = [0u8; 2];
        yuyv_to_luma8(&src, &mut dst);
    }

    #[test]
    #[should_panic(expected = "NV12 src のサイズが不足")]
    fn test_nv12_too_small() {
        // 2×2 の Y プレーンには 4 バイト必要なのに 3 バイトしか渡せない
        let src = [0u8; 3];
        nv12_luma_plane(&src, 2, 2);
    }
}
