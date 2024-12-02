use ndarray::{array, Array1, Array2};

pub enum BayerPattern {
    /// RGx
    RGGB,
    // BGx
    BGGR,
    /// GBx
    GBRG,
    /// GRx
    GRBG,
}

impl BayerPattern {
    #[inline]
    fn ptn(&self) -> Array2<u16> {
        match self {
            BayerPattern::RGGB => array![[1, 2], [2, 4]],
            BayerPattern::BGGR => array![[4, 2], [2, 1]],
            BayerPattern::GBRG => array![[2, 4], [1, 2]],
            BayerPattern::GRBG => array![[2, 1], [4, 2]],
        }
    }

    pub fn r(&self) -> ColorMask {
        ColorMask(self.ptn().mapv(|x| if x & 1 == 1 { 1 } else { 0 }))
    }

    pub fn g(&self) -> ColorMask {
        ColorMask(self.ptn().mapv(|x| if x & 2 == 2 { 1 } else { 0 }))
    }

    pub fn b(&self) -> ColorMask {
        ColorMask(self.ptn().mapv(|x| if x & 4 == 4 { 1 } else { 0 }))
    }
}

pub struct ColorMask(Array2<u16>);

impl ColorMask {
    pub fn mask(&self, src: &Array2<u16>) -> Array2<u16> {
        let mut dst = Array2::<u16>::zeros(src.dim());
        for i in 0..src.shape()[0] {
            for j in 0..src.shape()[1] {
                dst[[i, j]] = src[[i, j]] * self.0[[i % 2, j % 2]];
            }
        }
        dst
    }

    pub fn mask_vec(&self, src: &Array2<u16>) -> Array1<u16> {
        let mut dst = vec![];
        for i in 0..src.shape()[0] {
            for j in 0..src.shape()[1] {
                if self.0[[i % 2, j % 2]] > 0 {
                    dst.push(src[[i, j]]);
                }
            }
        }
        Array1::from(dst)
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Luma};
    use ndarray::{array, Array3, Axis};

    use crate::BayerPattern;

    const TESTIMAGE_32X32: &[u8] = include_bytes!("../../../testdata/32x32.png");

    fn test_load_image() -> ImageBuffer<Luma<u16>, Vec<u16>> {
        image::load_from_memory(TESTIMAGE_32X32)
            .unwrap()
            .into_luma16()
    }

    #[test]
    fn test_load_image_32x32() {
        let img = test_load_image();
        assert_eq!(img.width(), 32);
        assert_eq!(img.height(), 32);
    }

    #[test]
    fn test_bayer_mask() {
        let arr: Array3<u16> = array![[[1, 2, 3, 4], [5, 6, 7, 8]], [[3, 4, 5, 6], [7, 8, 9, 0]]];
        let z_sum = arr.sum_axis(Axis(0));

        let r = BayerPattern::RGGB.r().mask_vec(&z_sum);
        assert_eq!(r, array![4, 8]);
        let r = BayerPattern::RGGB.g().mask_vec(&z_sum);
        assert_eq!(r, array![6, 10, 12, 16]);
        let r = BayerPattern::RGGB.b().mask_vec(&z_sum);
        assert_eq!(r, array![14, 8]);
    }
}
