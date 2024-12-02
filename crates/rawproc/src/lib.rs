use ndarray::{array, Array1, Array2, Array3, ArrayView1, ArrayView2, Axis, ShapeError};

/// ベイヤーパターン
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    fn ptn(&self) -> Array2<u8> {
        match self {
            BayerPattern::RGGB => array![[1, 2], [2, 4]],
            BayerPattern::BGGR => array![[4, 2], [2, 1]],
            BayerPattern::GBRG => array![[2, 4], [1, 2]],
            BayerPattern::GRBG => array![[2, 1], [4, 2]],
        }
    }

    /// 特定成分のみを取得するマスクを返す
    pub fn mask(&self, ch: ColorChannel) -> ColorMask {
        let v = match ch {
            ColorChannel::R => 1,
            ColorChannel::G => 2,
            ColorChannel::B => 4,
        };
        ColorMask(self.ptn().mapv(|x| x & v == v))
    }
}

/// ベイヤーパターンにある色成分
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorChannel {
    R,
    G,
    B,
}

/// ベイヤーパターンによる特的の色をマスクする
pub struct ColorMask(Array2<bool>);

impl ColorMask {
    /// 対象の色のみのデータを取得する
    pub fn mask<T>(&self, src: &Array2<T>) -> Array2<T>
    where
        T: Clone + Copy + num_traits::identities::Zero,
    {
        let mut dst = Array2::<T>::zeros(src.dim());
        for i in 0..src.shape()[0] {
            for j in 0..src.shape()[1] {
                if self.0[[i % 2, j % 2]] {
                    dst[[i, j]] = src[[i, j]];
                }
            }
        }
        dst
    }

    /// 対象の色を抽出して1次元配列に変換する
    pub fn mask_vec<T>(&self, src: &Array2<T>) -> Array1<T>
    where
        T: Clone + Copy,
    {
        let mut dst = vec![];
        for i in 0..src.shape()[0] {
            for j in 0..src.shape()[1] {
                if self.0[[i % 2, j % 2]] {
                    dst.push(src[[i, j]]);
                }
            }
        }
        Array1::from(dst)
    }
}

/// 画像をndarrayに変換する
pub fn image_to_ndarray(
    img: &image::ImageBuffer<image::Luma<u16>, Vec<u16>>,
) -> Result<Array2<u16>, ShapeError> {
    let view = ArrayView1::from(img.as_raw());
    let x = view.into_shape_with_order((img.height() as usize, img.width() as usize))?;
    Ok(x.into_owned())
}

/// 計算用に画像スタックを保持する構造体
pub struct ImageStack {
    stack: Array3<f64>,
}

impl ImageStack {
    /// 新しい画像スタックを作成する
    pub fn new(img: &ArrayView2<u16>) -> Self {
        let img = img.mapv(|x| x as f64);
        let mut stack = Array3::<f64>::zeros((0, img.shape()[0], img.shape()[1]));
        stack.push(Axis(0), img.view()).unwrap();
        ImageStack { stack }
    }

    /// 画像をスタックに追加する
    pub fn push(&mut self, img: ArrayView2<u16>) {
        let img = img.mapv(|x| x as f64);
        self.stack.push(Axis(0), img.view()).unwrap();
    }

    /// スタックの各画素の平均値を取得する
    pub fn mean(&self) -> Array2<f64> {
        self.stack.mean_axis(Axis(0)).unwrap()
    }

    /// スタックの各画素の標準偏差を取得する
    pub fn std(&self) -> Array2<f64> {
        self.stack.std_axis(Axis(0), 1.0)
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Luma};
    use ndarray::{array, Array3, Axis};

    use crate::{image_to_ndarray, BayerPattern, ColorChannel, ImageStack};

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

        let img = image_to_ndarray(&img).unwrap();
        let mut stack = ImageStack::new(&img.view());
        for _ in 1..64 {
            stack.push(img.view());
        }

        let mean = stack.mean();
        assert_eq!(mean.shape(), [32, 32]);

        let ptns = [
            BayerPattern::RGGB,
            BayerPattern::BGGR,
            BayerPattern::GBRG,
            BayerPattern::GRBG,
        ];

        let colors = [(ColorChannel::R, 256_usize),
            (ColorChannel::G, 512),
            (ColorChannel::B, 256)];

        for ptn in ptns.iter() {
            for (color, count) in colors.iter() {
                let mask = ptn.mask(*color);
                let mean = mask.mask_vec(&mean);
                assert_eq!(mean.shape(), [*count]);
                assert!(mean.mean().unwrap() > 0.0);
                assert!(mean.std(1.0) > 0.0);
            }
        }
    }

    #[test]
    fn test_bayer_mask() {
        let arr: Array3<u16> = array![[[1, 2, 3, 4], [5, 6, 7, 8]], [[3, 4, 5, 6], [7, 8, 9, 0]]];
        let z_sum = arr.sum_axis(Axis(0));

        let ptn = BayerPattern::RGGB;

        let r = ptn.mask(ColorChannel::R).mask_vec(&z_sum);
        assert_eq!(r, array![4, 8]);
        let r = ptn.mask(ColorChannel::G).mask_vec(&z_sum);
        assert_eq!(r, array![6, 10, 12, 16]);
        let r = ptn.mask(ColorChannel::B).mask_vec(&z_sum);
        assert_eq!(r, array![14, 8]);
    }
}
