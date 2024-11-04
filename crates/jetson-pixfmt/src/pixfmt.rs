//! ピクセルフォーマットの定義

/// CSI入力のピクセルフォーマット
///
/// センサーの動作モードから期待するデータフォーマットを指定する
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsiPixelFormat {
    /// 10bit RAWデータ
    Raw10,
    /// 12bit RAWデータ
    Raw12,
}

impl CsiPixelFormat {
    /// 16bit幅で左詰めされたデータに対するマスク
    #[inline]
    pub const fn lmask_u16(&self) -> u16 {
        match self {
            CsiPixelFormat::Raw10 => 0xffc0,
            CsiPixelFormat::Raw12 => 0xfff0,
        }
    }

    /// 32bit幅のデータ若しくは2つの16ビットデータに対するマスク
    #[inline]
    pub const fn lmask_u32(&self) -> u32 {
        (self.lmask_u16() as u32).overflowing_shl(16).0 + self.lmask_u16() as u32
    }

    /// 64bit幅単位のデータに対するマスク
    #[inline]
    pub const fn lmask_u64(&self) -> u64 {
        (self.lmask_u32() as u64).overflowing_shl(32).0 + self.lmask_u32() as u64
    }

    /// 128bit幅単位のデータに対するマスク
    #[inline]
    pub const fn lmask_u128(&self) -> u128 {
        (self.lmask_u64() as u128).overflowing_shl(64).0 + self.lmask_u64() as u128
    }

    /// 左詰めデータを右詰めにするためのビットシフト数
    #[inline]
    pub const fn bitshift(&self) -> i32 {
        match self {
            CsiPixelFormat::Raw10 => 6,
            CsiPixelFormat::Raw12 => 4,
        }
    }

    /// 16bit幅のデータを左詰めにする
    #[inline]
    pub const fn format_u16(&self, data: u16) -> u16 {
        (data & self.lmask_u16()) >> self.bitshift()
    }

    /// 32bit幅のデータを左詰めにする
    #[inline]
    pub const fn format_u32(&self, data: u32) -> u32 {
        (data & self.lmask_u32()) >> self.bitshift()
    }

    /// 64bit幅のデータを左詰めにする
    #[inline]
    pub const fn format_u64(&self, data: u64) -> u64 {
        (data & self.lmask_u64()) >> self.bitshift()
    }

    /// 128bit幅のデータを左詰めにする
    #[inline]
    pub const fn format_u128(&self, data: u128) -> u128 {
        (data & self.lmask_u128()) >> self.bitshift()
    }

    /// 16bit幅で右詰めされたデータに対するマスク
    #[inline]
    pub const fn rmask_u16(&self) -> u16 {
        match self {
            CsiPixelFormat::Raw10 => 0x03ff,
            CsiPixelFormat::Raw12 => 0x0fff,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rand::Rng;

    impl CsiPixelFormat {
        // Jetson PIXFMTの動作再現
        // LittleEndianの先詰めで、残りのビットは繰り返しで埋める仕様となっている
        // 詳細はNVIDIA Orin Series System-on-Chip DP-10508-002_v1.1 | Page 1925
        #[cfg(test)]
        pub fn test_padding_u16(&self, data: u16) -> u16 {
            match self {
                CsiPixelFormat::Raw10 => {
                    let res = data << self.bitshift();
                    res + ((res & 0xfc00) >> 10)
                }
                CsiPixelFormat::Raw12 => {
                    let res = data << self.bitshift();
                    res + ((res & 0xf000) >> 12)
                }
            }
        }
    }

    /// 16bit幅のデータを複製して32bit幅に拡張する
    ///
    /// 複数ピクセルまとめて処理するテストの動作確認に使う
    pub fn u16_extend_u32(data: u16) -> u32 {
        (data as u32).overflowing_shl(16).0 + data as u32
    }

    /// 16bit幅のデータを複製して64bit幅に拡張する
    pub fn u16_extend_u64(data: u16) -> u64 {
        let data = u16_extend_u32(data);
        (data as u64).overflowing_shl(32).0 + data as u64
    }

    /// 16bit幅のデータを複製して128bit幅に拡張する
    pub fn u16_extend_u128(data: u16) -> u128 {
        let data = u16_extend_u64(data);
        (data as u128).overflowing_shl(64).0 + data as u128
    }

    // PIXFMTの動作再現テスト
    #[test]
    fn test_padding() {
        // CPUのデータはLittleEndianで後ろ詰め
        // 0x09a6 =[00001001,10100110]
        let test_data = 0b0000100110100110;
        // RAW10[01101001,10011010]
        assert_eq!(
            CsiPixelFormat::Raw10.test_padding_u16(test_data),
            0b0110100110011010
        );
        // RAW10[10011010, 01101001]
        assert_eq!(
            CsiPixelFormat::Raw12.test_padding_u16(test_data),
            0b1001101001101001
        );
    }

    #[test]
    fn test_mask() {
        // CPUのデータはLittleEndianで後ろ詰め
        // [01101001,10100110]
        let test_data = 0b01101001_10100110;
        assert_eq!(
            CsiPixelFormat::Raw12.lmask_u16() & test_data,
            0b01101001_10100000
        );

        let test_data: u32 = 0b01101001_10100110_01101001_10100110;
        assert_eq!(
            CsiPixelFormat::Raw12.lmask_u32() & test_data,
            0b01101001_10100000_01101001_10100000
        );

        let test_data: u64 =
            0b01101001_10100110_01101001_10100110_01101001_10100110_01101001_10100110;
        assert_eq!(
            CsiPixelFormat::Raw12.lmask_u64() & test_data,
            0b01101001_10100000_01101001_10100000_01101001_10100000_01101001_10100000
        );
    }

    // PIXFMTの復元テスト
    #[test]
    fn test_csi_pixel_format() {
        // テストデータで動作確認
        let rawdata = 0xc9a6;
        let td = vec![CsiPixelFormat::Raw10, CsiPixelFormat::Raw12];

        for tc in &td {
            let reference_data = tc.rmask_u16() & rawdata;
            let paddata = tc.test_padding_u16(reference_data);
            let decode_data = tc.format_u16(paddata);
            // println!("{:#018b} {:#018b} {:#018b}", reference_data, paddata, decode_data);
            assert_eq!(decode_data, reference_data);
        }

        // ランダムデータテスト
        let mut rng = rand::thread_rng();
        for _ in 0..1000 {
            let rawdata = rng.gen::<u16>();
            for tc in &td {
                let reference_u16 = tc.rmask_u16() & rawdata;
                let paddata_u16 = tc.test_padding_u16(reference_u16);

                let decode_data = tc.format_u16(paddata_u16);
                assert_eq!(decode_data, reference_u16);
                let decode_data = tc.format_u32(u16_extend_u32(paddata_u16));
                assert_eq!(decode_data, u16_extend_u32(reference_u16));
                let decode_data = tc.format_u64(u16_extend_u64(paddata_u16));
                assert_eq!(decode_data, u16_extend_u64(reference_u16));
                let decode_data = tc.format_u128(u16_extend_u128(paddata_u16));
                assert_eq!(decode_data, u16_extend_u128(reference_u16));
            }
        }
    }
}
