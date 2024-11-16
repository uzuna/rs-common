use yuvutils_rs::{YuvPackedImage, YuvRange, YuvStandardMatrix};

pub fn yuyv422_to_rgb(buf: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    let src = YuvPackedImage {
        width,
        height,
        yuy: buf,
        yuy_stride: width * 2,
    };
    let rgb_stride = width * 3;
    let mut rgb = vec![0; (rgb_stride * height) as usize];
    let range = YuvRange::Limited;
    let matrix = YuvStandardMatrix::Bt601;
    yuvutils_rs::yuyv422_to_rgb(&src, &mut rgb, rgb_stride, range, matrix)?;
    Ok(rgb)
}
