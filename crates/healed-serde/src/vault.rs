use crate::compression::{
    is_compressed_payload, maybe_compress_payload, maybe_decompress_payload, CompressionPolicy,
    COMPRESSED_HEADER_BYTES,
};
use crate::error::Error;
use crate::frame::StorageFrame;
use crate::metadata::MetaDataHeader;
use crate::rs::RsStrategy;
use crate::tmr::{TmrStrategy, TMR_HEADER_GROUP_BYTES};
use crate::DataClass;
use crate::ProtectionLevel;
use bitflip::{BitFlipConfig, BitFlipWriter, BitFlipWriterError};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const FRAME_HEADER_SCAN_BYTES: usize = 32;
const SLOT_SCAN_BYTES: usize = TMR_HEADER_GROUP_BYTES;

/// ストレージバックエンドの振る舞いを定義するトレイト。
///
/// これを実装することで、ファイルシステム以外のバックエンド（KVS、mmapなど）を
/// `ReliableVault` で使用できます。
pub trait StorageBackend {
    /// 指定されたインデックスのスロットからデータをバイト列として読み込みます。
    /// スロットが存在しない場合は `std::io::ErrorKind::NotFound` を含む `Error::Io` を返すべきです。
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error>;

    /// 指定されたインデックスのスロットにデータを書き込みます。
    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error>;

    /// 逐次書き込みでスロットにデータを書き込みます。
    fn write_slot_stream(
        &self,
        index: usize,
        stream_writer: &mut dyn FnMut(&mut dyn Write) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut bytes = Vec::new();
        stream_writer(&mut bytes)?;
        self.write_slot(index, &bytes)
    }

    /// スロットの先頭から指定された長さのデータを読み込みます。
    /// `save` 時のシーケンス番号スキャンを高速化するために使用されます。
    fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error>;

    /// バックエンド（ディレクトリなど）が存在することを保証します。
    fn ensure_backend_exists(&self) -> Result<(), Error>;
}

fn map_bitflip_writer_error(error: BitFlipWriterError) -> Error {
    match error {
        BitFlipWriterError::BitFlip(e) => Error::BitFlip(e),
        BitFlipWriterError::Io(e) => Error::Io(e),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SlotEncoding {
    Tmr,
    Rs,
    Frame(ProtectionLevel),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlotPayload {
    sequence: u64,
    payload: Vec<u8>,
    data_class: DataClass,
    encoding: SlotEncoding,
}

fn decode_slot_payload(bytes: &[u8]) -> Result<SlotPayload, Error> {
    if let Ok(slot) = TmrStrategy::decode_tmr_with_vote(bytes) {
        return Ok(SlotPayload {
            sequence: slot.sequence,
            payload: slot.payload,
            data_class: DataClass::Small,
            encoding: SlotEncoding::Tmr,
        });
    }

    if let Ok(slot) = RsStrategy::decode_record(bytes) {
        return Ok(SlotPayload {
            sequence: slot.sequence,
            payload: slot.payload,
            data_class: DataClass::Large,
            encoding: SlotEncoding::Rs,
        });
    }

    let frame = StorageFrame::recover(bytes)?;
    let level = frame.meta.level;
    let original_len = if is_compressed_payload(&frame.payload) {
        maybe_decompress_payload(&frame.payload)?.len()
    } else {
        frame.payload.len()
    };
    let data_class = DataClass::from_payload_len(original_len);
    Ok(SlotPayload {
        sequence: frame.meta.sequence,
        payload: frame.payload,
        data_class,
        encoding: SlotEncoding::Frame(level),
    })
}

fn encode_slot_payload(slot: &SlotPayload) -> Result<Vec<u8>, Error> {
    match slot.encoding {
        SlotEncoding::Tmr => Ok(TmrStrategy::encode_tmr(slot.sequence, &slot.payload)?),
        SlotEncoding::Rs => Ok(RsStrategy::encode_record(slot.sequence, &slot.payload)?),
        SlotEncoding::Frame(level) => {
            let frame = StorageFrame::new(slot.payload.clone(), slot.sequence, level);
            frame.to_bytes()
        }
    }
}

fn scan_slot_sequence(header_bytes: &[u8]) -> Option<u64> {
    TmrStrategy::peek_sequence(header_bytes)
        .or_else(|| RsStrategy::peek_sequence(header_bytes))
        .or_else(|| scan_frame_sequence(header_bytes))
}

fn scan_frame_sequence(header_bytes: &[u8]) -> Option<u64> {
    if header_bytes.len() < FRAME_HEADER_SCAN_BYTES {
        return None;
    }

    let primary_bytes: [u8; 16] = header_bytes[0..16].try_into().ok()?;
    if let Some(meta) = MetaDataHeader::from_bytes(&primary_bytes).decode() {
        return Some(meta.sequence);
    }

    let secondary_bytes: [u8; 16] = header_bytes[16..32].try_into().ok()?;
    MetaDataHeader::from_bytes(&secondary_bytes)
        .decode()
        .map(|meta| meta.sequence)
}

/// 標準のファイルシステムをバックエンドとして使用する実装。
pub struct FileSystemBackend {
    dir: PathBuf,
    filename_base: String,
    bitflip_config: Option<BitFlipConfig>,
}

impl FileSystemBackend {
    /// 新しい `FileSystemBackend` を作成します。
    pub fn new(dir: impl Into<PathBuf>, filename_base: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            filename_base: filename_base.into(),
            bitflip_config: None,
        }
    }

    /// テスト/検証用途で保存時の故障注入を有効にします。
    pub fn with_bitflip_for_tests(mut self, config: BitFlipConfig) -> Self {
        self.bitflip_config = Some(config);
        self
    }

    fn slot_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{}.{}", self.filename_base, index))
    }

    fn slot_temp_path(&self, index: usize) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.dir
            .join(format!("{}.{}.tmp-{}", self.filename_base, index, stamp))
    }

    fn sync_parent_dir(&self) -> Result<(), Error> {
        let dir = File::open(&self.dir)?;
        dir.sync_all()?;
        Ok(())
    }
}

impl StorageBackend for FileSystemBackend {
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error> {
        fs::read(self.slot_path(index)).map_err(Into::into)
    }

    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error> {
        self.ensure_backend_exists()?;

        let slot_path = self.slot_path(index);
        let temp_path = self.slot_temp_path(index);
        let file = File::create(&temp_path)?;
        let file = match self.bitflip_config {
            Some(config) => {
                let mut writer = BitFlipWriter::new(file, config);
                writer.write_all(data)?;
                writer.finish().map_err(map_bitflip_writer_error)?
            }
            None => {
                let mut file = file;
                file.write_all(data)?;
                file
            }
        };
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }
        if let Err(error) = fs::rename(&temp_path, &slot_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }
        self.sync_parent_dir()?;
        Ok(())
    }

    fn write_slot_stream(
        &self,
        index: usize,
        stream_writer: &mut dyn FnMut(&mut dyn Write) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.ensure_backend_exists()?;

        let slot_path = self.slot_path(index);
        let temp_path = self.slot_temp_path(index);
        let file = File::create(&temp_path)?;
        let file = match self.bitflip_config {
            Some(config) => {
                let mut writer = BitFlipWriter::new(file, config);
                stream_writer(&mut writer)?;
                writer.finish().map_err(map_bitflip_writer_error)?
            }
            None => {
                let mut file = file;
                stream_writer(&mut file)?;
                file
            }
        };

        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }
        if let Err(error) = fs::rename(&temp_path, &slot_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }
        self.sync_parent_dir()?;
        Ok(())
    }

    fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error> {
        let file = File::open(self.slot_path(index))?;
        let mut buf = Vec::with_capacity(len);
        file.take(len as u64).read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn ensure_backend_exists(&self) -> Result<(), Error> {
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

/// メモリ領域のみを使用する `StorageBackend` 実装。
///
/// テスト用途や、プロセス寿命内だけの一時ストレージ用途を想定しています。
#[derive(Debug, Clone)]
pub struct MemoryBackend {
    slots: Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    bitflip_config: Option<BitFlipConfig>,
}

pub type InMemBackend = MemoryBackend;

impl MemoryBackend {
    /// 指定スロット数で新しい `MemoryBackend` を作成します。
    pub fn new(num_slots: usize) -> Self {
        assert!(num_slots > 0, "MemoryBackend requires at least 1 slot");
        Self {
            slots: Arc::new(Mutex::new(vec![None; num_slots])),
            bitflip_config: None,
        }
    }

    /// テスト/検証用途で保存時の故障注入を有効にします。
    pub fn with_bitflip_for_tests(mut self, config: BitFlipConfig) -> Self {
        self.bitflip_config = Some(config);
        self
    }

    fn lock_slots(&self) -> Result<std::sync::MutexGuard<'_, Vec<Option<Vec<u8>>>>, Error> {
        self.slots
            .lock()
            .map_err(|_| std::io::Error::other("MemoryBackend lock poisoned").into())
    }

    fn not_found(index: usize) -> Error {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Memory slot {index} not found"),
        )
        .into()
    }

    fn invalid_index(index: usize) -> Error {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Memory slot index {index} is out of bounds"),
        )
        .into()
    }

    fn slot_ref(slots: &[Option<Vec<u8>>], index: usize) -> Result<&Option<Vec<u8>>, Error> {
        slots.get(index).ok_or_else(|| Self::invalid_index(index))
    }

    fn slot_mut(
        slots: &mut [Option<Vec<u8>>],
        index: usize,
    ) -> Result<&mut Option<Vec<u8>>, Error> {
        slots
            .get_mut(index)
            .ok_or_else(|| Self::invalid_index(index))
    }

    #[cfg(test)]
    fn mutate_slot<F>(&self, index: usize, mutator: F) -> Result<(), Error>
    where
        F: FnOnce(&mut Vec<u8>),
    {
        let mut slots = self.lock_slots()?;
        let slot = Self::slot_mut(&mut slots, index)?;
        let data = slot.as_mut().ok_or_else(|| Self::not_found(index))?;
        mutator(data);
        Ok(())
    }
}

impl StorageBackend for MemoryBackend {
    fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error> {
        let slots = self.lock_slots()?;
        let slot = Self::slot_ref(&slots, index)?;
        slot.clone().ok_or_else(|| Self::not_found(index))
    }

    fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error> {
        let mut slots = self.lock_slots()?;
        let slot = Self::slot_mut(&mut slots, index)?;

        let stored = match self.bitflip_config {
            Some(config) => {
                let mut writer = BitFlipWriter::new(Vec::<u8>::new(), config);
                writer.write_all(data)?;
                writer.finish().map_err(map_bitflip_writer_error)?
            }
            None => data.to_vec(),
        };

        *slot = Some(stored);
        Ok(())
    }

    fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error> {
        let mut data = self.read_slot(index)?;
        if data.len() > len {
            data.truncate(len);
        }
        Ok(data)
    }

    fn ensure_backend_exists(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// 複数のスロットを用いたローリングアップデートによる永続化ストレージ。
///
/// 書き込み時は最も古いスロットを上書きし、読み込み時は破損していない最新のスロットを選択します。
/// ストレージバックエンドは `StorageBackend` トレイトを介して抽象化されています。
///
/// # Examples
///
/// ```no_run
/// use healed_serde::vault::ReliableVault;
/// use healed_serde::ProtectionLevel;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize, PartialEq, Debug)]
/// struct Config {
///     version: u32,
///     name: String,
/// }
///
/// // ファイルシステムをバックエンドとして使用する例
/// let vault = ReliableVault::<_>::new_with_fs("./data", "config");
///
/// let config = Config { version: 1, name: "device-001".to_string() };
/// vault.save(&config, ProtectionLevel::Medium).unwrap();
///
/// let loaded: Config = vault.load().unwrap();
/// ```
pub struct ReliableVault<T, B: StorageBackend = FileSystemBackend> {
    backend: B,
    num_slots: usize,
    compression_policy: CompressionPolicy,
    _phantom: std::marker::PhantomData<T>,
}

/// スクラブ実行結果の集計情報。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrubReport {
    pub scanned_slots: usize,
    pub healthy_slots: usize,
    pub repaired_slots: usize,
    pub recoverable_error_slots: usize,
    pub budget_skipped_slots: usize,
}

impl<T> ReliableVault<T, FileSystemBackend>
where
    T: Serialize + DeserializeOwned,
{
    /// ファイルシステムをバックエンドとして使用する新しいVaultを3スロットで作成します。
    ///
    /// # Arguments
    /// * `dir`: データファイルを保存するディレクトリ。
    /// * `filename_base`: ファイル名のプレフィックス（例: "data" -> "data.0", "data.1", "data.2"）。
    pub fn new_with_fs(dir: impl Into<PathBuf>, filename_base: impl Into<String>) -> Self {
        let backend = FileSystemBackend::new(dir, filename_base);
        Self::new(backend, 3)
    }
}

impl<T, B: StorageBackend> ReliableVault<T, B>
where
    T: Serialize + DeserializeOwned,
{
    /// 指定されたバックエンドとスロット数で新しいVaultを作成します。
    pub fn new(backend: B, num_slots: usize) -> Self {
        assert!(num_slots >= 3, "ReliableVault requires at least 3 slots");
        Self {
            backend,
            num_slots,
            compression_policy: CompressionPolicy::default(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// データ分類ごとの圧縮ポリシーを設定します。
    pub fn with_compression_policy(mut self, compression_policy: CompressionPolicy) -> Self {
        self.compression_policy = compression_policy;
        self
    }

    /// 現在の圧縮ポリシーを返します。
    pub fn compression_policy(&self) -> CompressionPolicy {
        self.compression_policy
    }

    fn next_generation_target(&self) -> Result<(u64, usize), Error> {
        let mut max_sequence = 0;
        let mut found_any = false;
        let mut sequenced_slots = Vec::with_capacity(self.num_slots);
        let mut empty_slots = Vec::new();

        for index in 0..self.num_slots {
            let header = match self.backend.read_header(index, SLOT_SCAN_BYTES) {
                Ok(header) => header,
                Err(error) if error.is_recoverable() => {
                    empty_slots.push(index);
                    continue;
                }
                Err(error) => return Err(error),
            };

            if let Some(sequence) = scan_slot_sequence(&header) {
                max_sequence = max_sequence.max(sequence);
                found_any = true;
                sequenced_slots.push((sequence, index));
            } else {
                empty_slots.push(index);
            }
        }

        let next_sequence = if found_any {
            max_sequence.saturating_add(1)
        } else {
            1
        };

        let target_slot = if !empty_slots.is_empty() {
            let preferred_slot = (next_sequence as usize) % self.num_slots;
            if empty_slots.contains(&preferred_slot) {
                preferred_slot
            } else {
                empty_slots[0]
            }
        } else {
            sequenced_slots.sort_by_key(|(sequence, index)| (*sequence, *index));
            sequenced_slots
                .first()
                .map(|(_, index)| *index)
                .unwrap_or(0)
        };

        Ok((next_sequence, target_slot))
    }

    fn latest_slot(&self, expected_class: Option<DataClass>) -> Result<SlotPayload, Error> {
        let mut candidates = Vec::new();

        for index in 0..self.num_slots {
            let bytes = match self.backend.read_slot(index) {
                Ok(bytes) => bytes,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            let slot = match decode_slot_payload(&bytes) {
                Ok(slot) => slot,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            if expected_class.is_none_or(|class| slot.data_class == class) {
                candidates.push(slot);
            }
        }

        if candidates.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No valid data slots found",
            )
            .into());
        }

        candidates.sort_by_key(|slot| std::cmp::Reverse(slot.sequence));
        Ok(candidates.remove(0))
    }

    fn large_slot_indexes_desc(&self) -> Result<Vec<usize>, Error> {
        let mut candidates = Vec::new();

        for index in 0..self.num_slots {
            let header = match self.backend.read_header(index, SLOT_SCAN_BYTES) {
                Ok(header) => header,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            let sequence =
                RsStrategy::peek_sequence(&header).or_else(|| scan_frame_sequence(&header));
            if let Some(sequence) = sequence {
                candidates.push((sequence, index));
            }
        }

        candidates.sort_by_key(|(sequence, _)| std::cmp::Reverse(*sequence));
        Ok(candidates.into_iter().map(|(_, index)| index).collect())
    }

    /// 最新の有効なデータを読み込みます。
    ///
    /// 全てのスロットを確認し、破損していないデータの中で最も新しいシーケンス番号を持つものを返します。
    pub fn load(&self) -> Result<T, Error> {
        let latest = self.latest_slot(None)?;
        let payload = maybe_decompress_payload(&latest.payload)?;
        Ok(bincode::deserialize(&payload)?)
    }

    /// 小サイズデータをTMRで保存します。
    pub fn save_small(&self, data: &T) -> Result<(), Error> {
        self.backend.ensure_backend_exists()?;

        let serialized_payload = bincode::serialize(data)?;
        if DataClass::from_payload_len(serialized_payload.len()) != DataClass::Small {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Serialized payload size {} exceeds small-data threshold {}",
                    serialized_payload.len(),
                    crate::SMALL_DATA_THRESHOLD_BYTES
                ),
            )
            .into());
        }

        let payload = maybe_compress_payload(
            &serialized_payload,
            DataClass::Small,
            self.compression_policy,
        )?;

        let (sequence, target_slot) = self.next_generation_target()?;
        let bytes = TmrStrategy::encode_tmr(sequence, &payload)?;
        self.backend.write_slot(target_slot, &bytes)
    }

    /// TMRで保存された最新の小サイズデータを読み込みます。
    pub fn load_small(&self) -> Result<T, Error> {
        let latest = self.latest_slot(Some(DataClass::Small))?;
        let payload = maybe_decompress_payload(&latest.payload)?;
        Ok(bincode::deserialize(&payload)?)
    }

    /// 大サイズデータをRSセグメント化で保存します。
    pub fn save_large(&self, data: &T) -> Result<(), Error> {
        self.backend.ensure_backend_exists()?;

        let serialized_payload = bincode::serialize(data)?;
        if DataClass::from_payload_len(serialized_payload.len()) != DataClass::Large {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Serialized payload size {} is below large-data threshold {}",
                    serialized_payload.len(),
                    crate::SMALL_DATA_THRESHOLD_BYTES
                ),
            )
            .into());
        }

        let payload = maybe_compress_payload(
            &serialized_payload,
            DataClass::Large,
            self.compression_policy,
        )?;

        let (sequence, target_slot) = self.next_generation_target()?;
        let mut writer = |sink: &mut dyn Write| -> Result<(), Error> {
            RsStrategy::encode_record_to_writer(sequence, &payload, sink)?;
            Ok(())
        };
        self.backend.write_slot_stream(target_slot, &mut writer)
    }

    /// RSまたは従来フレームで保存された最新の大サイズデータを読み込みます。
    pub fn load_large(&self) -> Result<T, Error> {
        let latest = self.latest_slot(Some(DataClass::Large))?;
        let payload = maybe_decompress_payload(&latest.payload)?;
        Ok(bincode::deserialize(&payload)?)
    }

    /// 最新の大サイズスロットから、シリアライズ済みペイロードの指定バイト範囲を読み込みます。
    pub fn load_large_range(&self, range: Range<usize>) -> Result<Vec<u8>, Error> {
        if range.start > range.end {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Invalid large range: start {} is greater than end {}",
                    range.start, range.end
                ),
            )
            .into());
        }

        let candidates = self.large_slot_indexes_desc()?;
        if candidates.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No valid large-data slots found",
            )
            .into());
        }

        for index in candidates {
            let bytes = match self.backend.read_slot(index) {
                Ok(bytes) => bytes,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            if let Ok(prefix) = RsStrategy::decode_payload_range(&bytes, 0..COMPRESSED_HEADER_BYTES)
            {
                if is_compressed_payload(&prefix) {
                    let record = match RsStrategy::decode_record(&bytes) {
                        Ok(record) => record,
                        Err(error) => {
                            let mapped: Error = error.into();
                            if mapped.is_recoverable() {
                                continue;
                            }
                            return Err(mapped);
                        }
                    };

                    let payload = maybe_decompress_payload(&record.payload)?;
                    let start = range.start.min(payload.len());
                    let end = range.end.min(payload.len());
                    return Ok(payload[start..end].to_vec());
                }

                if let Ok(partial) = RsStrategy::decode_payload_range(&bytes, range.clone()) {
                    return Ok(partial);
                }
            }

            let frame = match StorageFrame::recover(&bytes) {
                Ok(frame) => frame,
                Err(error) if error.is_recoverable() => continue,
                Err(error) => return Err(error),
            };

            let payload = maybe_decompress_payload(&frame.payload)?;

            let start = range.start.min(payload.len());
            let end = range.end.min(payload.len());
            return Ok(payload[start..end].to_vec());
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No valid large-data slots found",
        )
        .into())
    }

    /// 全スロットを1巡し、復元可能な破損を再書き戻しで修復します。
    pub fn scrub(&self) -> Result<ScrubReport, Error> {
        self.scrub_with_budget(usize::MAX)
    }

    /// 全スロットを1巡し、復元可能な破損を修復します。
    ///
    /// `max_repair_writes` で1回のスクラブで許容する修復書き込み回数を制限できます。
    pub fn scrub_with_budget(&self, max_repair_writes: usize) -> Result<ScrubReport, Error> {
        self.backend.ensure_backend_exists()?;
        let mut report = ScrubReport::default();

        for index in 0..self.num_slots {
            report.scanned_slots += 1;

            let bytes = match self.backend.read_slot(index) {
                Ok(bytes) => bytes,
                Err(error) if error.is_recoverable() => {
                    report.recoverable_error_slots += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };

            let decoded = match decode_slot_payload(&bytes) {
                Ok(decoded) => decoded,
                Err(error) if error.is_recoverable() => {
                    report.recoverable_error_slots += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };

            let repaired_bytes = encode_slot_payload(&decoded)?;
            if repaired_bytes == bytes {
                report.healthy_slots += 1;
                continue;
            }

            if report.repaired_slots >= max_repair_writes {
                report.budget_skipped_slots += 1;
                continue;
            }

            self.backend.write_slot(index, &repaired_bytes)?;
            report.repaired_slots += 1;
        }

        Ok(report)
    }

    /// データを保存します。
    ///
    /// 現在の最大シーケンス番号を確認し、最も古い世代を保持するスロットを上書きします。
    /// 大サイズデータはRSセグメント化を使用し、小サイズは従来フレーム形式を維持します。
    pub fn save(&self, data: &T, level: ProtectionLevel) -> Result<(), Error> {
        self.backend.ensure_backend_exists()?;
        let (new_sequence, target_slot) = self.next_generation_target()?;

        // ペイロードのシリアライズ
        let serialized_payload = bincode::serialize(data)?;
        let data_class = DataClass::from_payload_len(serialized_payload.len());
        let payload =
            maybe_compress_payload(&serialized_payload, data_class, self.compression_policy)?;

        if data_class == DataClass::Large {
            let mut writer = |sink: &mut dyn Write| -> Result<(), Error> {
                RsStrategy::encode_record_to_writer(new_sequence, &payload, sink)?;
                Ok(())
            };
            self.backend.write_slot_stream(target_slot, &mut writer)
        } else {
            let frame = StorageFrame::new(payload, new_sequence, level);
            let bytes = frame.to_bytes()?;
            self.backend.write_slot(target_slot, &bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rs::{
        RS_DATA_BYTES_PER_SEGMENT, RS_RECORD_HEADER_BYTES, RS_SHARD_BYTES, RS_TOTAL_SHARDS,
    };
    use bitflip::{BitFlipConfig, BitFlipTarget, TemporaryReadErrorReader, TreConfig};
    use serde::Deserialize;
    use std::path::Path;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
    struct TestData {
        id: u32,
        message: String,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct JsonCaseData {
        id: u64,
        name: String,
        tags: Vec<String>,
        values: Vec<u64>,
        payload: String,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
    struct LargeCaseData {
        id: u64,
        payload: Vec<u8>,
    }

    /// ストレステストで注入する故障シナリオ。
    #[derive(Debug, Clone, Copy)]
    enum StressScenario {
        LatestFooterCrcCorrupt,
        LatestTruncated,
        LatestHeaderOnly,
        TwoLatestFooterCrcCorrupt,
        LatestHeaderOnlyAndSecondCorrupt,
        AllFooterCrcCorrupt,
        AllTruncated,
        AllHeaderOnly,
    }

    /// シナリオ実行後に期待される読み出し結果。
    #[derive(Debug, Clone, Copy)]
    enum ExpectedOutcome {
        LoadedId(u32),
        NotFound,
    }

    fn corrupt_slot_footer_crc(path: &Path) {
        let mut bytes = fs::read(path).unwrap();
        let crc_offset = bytes.len() - 8;
        bytes[crc_offset] ^= 0xFF;
        fs::write(path, bytes).unwrap();
    }

    fn truncate_slot(path: &Path, size: usize) {
        let mut bytes = fs::read(path).unwrap();
        bytes.truncate(size);
        fs::write(path, bytes).unwrap();
    }

    fn count_diff_bits(lhs: &[u8], rhs: &[u8]) -> usize {
        assert_eq!(lhs.len(), rhs.len(), "length mismatch");
        lhs.iter()
            .zip(rhs.iter())
            .map(|(l, r)| (l ^ r).count_ones() as usize)
            .sum()
    }

    fn assert_not_found(result: Result<TestData, Error>) {
        assert!(matches!(
            result,
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
    }

    fn build_json_case_data(id: u64, values_len: usize, payload_len: usize) -> JsonCaseData {
        JsonCaseData {
            id,
            name: format!("case-{id}"),
            tags: (0..8).map(|i| format!("tag-{id}-{i}")).collect(),
            values: (0..values_len).map(|i| (id + i as u64) * 3).collect(),
            payload: "x".repeat(payload_len),
        }
    }

    fn build_large_case_data(id: u64, payload_len: usize) -> LargeCaseData {
        LargeCaseData {
            id,
            payload: (0..payload_len)
                .map(|index| ((id as usize + index) % 251) as u8)
                .collect(),
        }
    }

    fn corrupt_rs_segment_shards(bytes: &mut [u8], shard_indices: &[usize]) {
        const RS_SEGMENT_HEADER_BYTES_FOR_TEST: usize = 24;
        let shard_crc_table_bytes = RS_TOTAL_SHARDS * 4;
        let shard_data_start =
            RS_RECORD_HEADER_BYTES + RS_SEGMENT_HEADER_BYTES_FOR_TEST + shard_crc_table_bytes;

        for (offset, shard_index) in shard_indices.iter().enumerate() {
            let target = shard_data_start + (*shard_index * RS_SHARD_BYTES) + offset;
            bytes[target] ^= 1u8 << (offset % 8);
        }
    }

    fn first_present_slot(backend: &MemoryBackend, num_slots: usize) -> (usize, Vec<u8>) {
        for index in 0..num_slots {
            if let Ok(bytes) = backend.read_slot(index) {
                return (index, bytes);
            }
        }
        panic!("no present slot found");
    }

    fn collect_slot_sequences(backend: &MemoryBackend, num_slots: usize) -> Vec<(usize, u64)> {
        let mut entries = Vec::new();
        for index in 0..num_slots {
            let Ok(header) = backend.read_header(index, SLOT_SCAN_BYTES) else {
                continue;
            };
            if let Some(sequence) = scan_slot_sequence(&header) {
                entries.push((index, sequence));
            }
        }
        entries
    }

    #[derive(Clone)]
    struct WriteFailBackend {
        inner: MemoryBackend,
        fail_index: usize,
    }

    impl WriteFailBackend {
        fn new(inner: MemoryBackend, fail_index: usize) -> Self {
            Self { inner, fail_index }
        }
    }

    impl StorageBackend for WriteFailBackend {
        fn read_slot(&self, index: usize) -> Result<Vec<u8>, Error> {
            self.inner.read_slot(index)
        }

        fn write_slot(&self, index: usize, data: &[u8]) -> Result<(), Error> {
            if index == self.fail_index {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected write failure",
                )
                .into());
            }
            self.inner.write_slot(index, data)
        }

        fn read_header(&self, index: usize, len: usize) -> Result<Vec<u8>, Error> {
            self.inner.read_header(index, len)
        }

        fn ensure_backend_exists(&self) -> Result<(), Error> {
            self.inner.ensure_backend_exists()
        }
    }

    /// MemoryBackendの基本I/O (read/write/read_header) と NotFound を検証。
    #[test]
    fn test_memory_backend_read_write_and_header() {
        let backend = MemoryBackend::new(3);
        backend.ensure_backend_exists().unwrap();

        let payload = vec![1u8, 2, 3, 4, 5];
        backend.write_slot(1, &payload).unwrap();

        let full = backend.read_slot(1).unwrap();
        assert_eq!(full, payload);

        let header = backend.read_header(1, 3).unwrap();
        assert_eq!(header, vec![1u8, 2, 3]);

        assert!(matches!(
            backend.read_slot(0),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ));
    }

    /// BitFlip未設定時は保存データがそのまま保持されることを検証。
    #[test]
    fn test_backend_bitflip_disabled_by_default() {
        let backend = MemoryBackend::new(3);
        let payload = vec![0x5Au8; 64];

        backend.write_slot(0, &payload).unwrap();
        let stored = backend.read_slot(0).unwrap();

        assert_eq!(stored, payload);
    }

    /// Memory/FileSystem の両バックエンドでBitFlip注入が機能することを検証。
    #[test]
    fn test_backend_bitflip_injection_enabled_cases() {
        let payload = vec![0xA3u8; 64];
        let cases = [
            (
                "memory_backend",
                BitFlipConfig::new(5, Some(12345), BitFlipTarget::FullFrame),
            ),
            (
                "filesystem_backend",
                BitFlipConfig::new(5, Some(12345), BitFlipTarget::FullFrame),
            ),
        ];

        for (name, config) in cases {
            if name == "memory_backend" {
                let backend = MemoryBackend::new(3).with_bitflip_for_tests(config);
                backend
                    .write_slot(1, &payload)
                    .unwrap_or_else(|e| panic!("{name}: write_slot failed: {e}"));
                let stored = backend
                    .read_slot(1)
                    .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));

                assert_eq!(
                    count_diff_bits(&payload, &stored),
                    config.flip_bits,
                    "{name}: unexpected flipped bit count"
                );
                continue;
            }

            let dir = tempfile::tempdir().unwrap();
            let backend =
                FileSystemBackend::new(dir.path(), "bitflip").with_bitflip_for_tests(config);
            backend
                .write_slot(2, &payload)
                .unwrap_or_else(|e| panic!("{name}: write_slot failed: {e}"));
            let stored = backend
                .read_slot(2)
                .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));

            assert_eq!(
                count_diff_bits(&payload, &stored),
                config.flip_bits,
                "{name}: unexpected flipped bit count"
            );
        }
    }

    /// MemoryBackendの境界外インデックスが InvalidInput として扱われることを検証。
    #[test]
    fn test_memory_backend_boundary_invalid_index() {
        let backend = MemoryBackend::new(3);

        assert!(matches!(
            backend.read_slot(3),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            backend.read_header(99, 8),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            backend.write_slot(10, &[1, 2, 3]),
            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
    }

    /// read_header の長さ0指定で空データが返ることを検証。
    #[test]
    fn test_memory_backend_boundary_zero_header_len() {
        let backend = MemoryBackend::new(3);
        backend.write_slot(0, &[1, 2, 3, 4]).unwrap();

        let header = backend.read_header(0, 0).unwrap();
        assert!(header.is_empty());
    }

    /// スロット数0のMemoryBackend生成は不正なためpanicすることを検証。
    #[test]
    #[should_panic(expected = "MemoryBackend requires at least 1 slot")]
    fn test_memory_backend_new_panics_with_zero_slots() {
        let _ = MemoryBackend::new(0);
    }

    /// ReliableVaultは3スロット以上前提のため、2スロット指定でpanicすることを検証。
    #[test]
    #[should_panic(expected = "ReliableVault requires at least 3 slots")]
    fn test_vault_new_panics_when_slots_less_than_three() {
        let backend = MemoryBackend::new(2);
        let _ = ReliableVault::<TestData, _>::new(backend, 2);
    }

    /// MemoryBackend上でsave/loadの往復が成立することを検証。
    #[test]
    fn test_vault_with_memory_backend_roundtrip() {
        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<TestData, _>::new(backend, 3);

        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 4);
    }

    /// MemoryBackend上で最新スロット破損時に次点スロットへフォールバックできることを検証。
    #[test]
    fn test_vault_with_memory_backend_fallback_after_corruption() {
        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3);

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::High).unwrap();
        }

        backend
            .mutate_slot(1, |bytes| {
                let crc_pos = bytes.len() - 8;
                bytes[crc_pos] ^= 0xFF;
            })
            .unwrap();

        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 5スロット運用時のローテーション境界と複数スロット破損後のフォールバックを検証。
    #[test]
    fn test_vault_with_memory_backend_boundary_rotation_five_slots() {
        let backend = MemoryBackend::new(5);
        let vault = ReliableVault::<TestData, _>::new(backend.clone(), 5);

        // seq1..7 を保存して、5スロット循環の境界を跨ぐ
        for i in 1..=7 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新は seq7
        let loaded_latest = vault.load().unwrap();
        assert_eq!(loaded_latest.id, 7);

        // seq7(slot2) と seq6(slot1) を壊すと次点は seq5(slot0)
        backend
            .mutate_slot(2, |bytes| {
                let crc_pos = bytes.len() - 8;
                bytes[crc_pos] ^= 0xFF;
            })
            .unwrap();
        backend
            .mutate_slot(1, |bytes| {
                let crc_pos = bytes.len() - 8;
                bytes[crc_pos] ^= 0xFF;
            })
            .unwrap();

        let loaded_fallback = vault.load().unwrap();
        assert_eq!(loaded_fallback.id, 5);
    }

    /// 最新スロット破損時に1世代前へフォールバックできることを検証。
    #[test]
    fn test_vault_rotation_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // 1. 正常なローテーション (4回保存 -> seq 1, 2, 3, 4)
        // slot mapping: 1->1, 2->2, 3->0, 4->1(overwrite)
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 4, "最新のデータ(id=4)が読み込まれるべき");

        // 2. 最新データの破損 (slot 1, seq 4 を破壊)
        let slot1 = dir.path().join("test.1");
        let mut bytes = fs::read(&slot1).unwrap();
        let crc_pos = bytes.len() - 8;
        bytes[crc_pos] ^= 0xFF; // CRCを破壊
        fs::write(&slot1, bytes).unwrap();

        // 3. 自動フォールバック (次に新しい seq 3 が読み込まれるべき)
        let loaded_fallback = vault.load().unwrap();
        assert_eq!(
            loaded_fallback.id, 3,
            "破損した最新データの代わりに一つ前のデータが読み込まれるべき"
        );
    }

    /// 最新2スロット破損時にさらに古い健全スロットへフォールバックできることを検証。
    #[test]
    fn test_vault_recovery_with_two_corrupted_slots() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1, 5->slot2
        // 最終状態: slot0=seq3, slot1=seq4, slot2=seq5
        for i in 1..=5 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新2スロットを破損させる
        corrupt_slot_footer_crc(&dir.path().join("test.2")); // seq5
        corrupt_slot_footer_crc(&dir.path().join("test.1")); // seq4

        // 残存する有効なseq3へフォールバックできること
        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 全スロット破損時にNotFound相当エラーとなることを検証。
    #[test]
    fn test_vault_load_fails_when_all_slots_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        for i in 1..=3 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Low).unwrap();
        }

        for slot in 0..3 {
            corrupt_slot_footer_crc(&dir.path().join(format!("test.{}", slot)));
        }

        assert_not_found(vault.load());
    }

    /// 最新スロットが途中書き込み（truncate）でも次点へフォールバックできることを検証。
    #[test]
    fn test_vault_recovery_with_truncated_latest_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新slot(seq4)を途中書き込み想定で短くする
        truncate_slot(&dir.path().join("test.1"), 12);

        // 有効な次点seq3にフォールバックできること
        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 全スロットが途中書き込み状態（truncate）の場合にNotFoundとなることを検証。
    #[test]
    fn test_vault_load_fails_when_all_slots_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        for i in 1..=3 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::High).unwrap();
        }

        // 3スロットすべて途中書き込み相当の短い不完全ファイルにする
        for slot in 0..3 {
            truncate_slot(&dir.path().join(format!("test.{}", slot)), 8);
        }

        assert_not_found(vault.load());
    }

    /// 最新スロットがヘッダーのみ（32B）でも次点へフォールバックできることを検証。
    #[test]
    fn test_vault_recovery_with_header_only_latest_slot() {
        let dir = tempfile::tempdir().unwrap();
        let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "test");

        // seq: 1->slot1, 2->slot2, 3->slot0, 4->slot1
        for i in 1..=4 {
            let data = TestData {
                id: i,
                message: format!("msg {}", i),
            };
            vault.save(&data, ProtectionLevel::Medium).unwrap();
        }

        // 最新slot(seq4)をヘッダーのみ(Primary+Secondary=32B)に切り詰める
        truncate_slot(&dir.path().join("test.1"), 32);

        // 不完全ファイルを無視して、有効な次点seq3へフォールバックできること
        let loaded = vault.load().unwrap();
        assert_eq!(loaded.id, 3);
    }

    /// 代表的な破損シナリオを列挙し、期待結果（復旧 or NotFound）を網羅的に検証。
    #[test]
    fn test_vault_stress_cases_enumerated() {
        let cases = [
            (
                "latest_footer_crc_corrupt",
                StressScenario::LatestFooterCrcCorrupt,
                ExpectedOutcome::LoadedId(4),
            ),
            (
                "latest_truncated",
                StressScenario::LatestTruncated,
                ExpectedOutcome::LoadedId(4),
            ),
            (
                "latest_header_only",
                StressScenario::LatestHeaderOnly,
                ExpectedOutcome::LoadedId(4),
            ),
            (
                "two_latest_footer_crc_corrupt",
                StressScenario::TwoLatestFooterCrcCorrupt,
                ExpectedOutcome::LoadedId(3),
            ),
            (
                "latest_header_only_and_second_corrupt",
                StressScenario::LatestHeaderOnlyAndSecondCorrupt,
                ExpectedOutcome::LoadedId(3),
            ),
            (
                "all_footer_crc_corrupt",
                StressScenario::AllFooterCrcCorrupt,
                ExpectedOutcome::NotFound,
            ),
            (
                "all_truncated",
                StressScenario::AllTruncated,
                ExpectedOutcome::NotFound,
            ),
            (
                "all_header_only",
                StressScenario::AllHeaderOnly,
                ExpectedOutcome::NotFound,
            ),
        ];

        for (name, scenario, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "stress");

            for id in 1..=5 {
                let data = TestData {
                    id,
                    message: format!("msg {}", id),
                };
                vault.save(&data, ProtectionLevel::Medium).unwrap();
            }

            let slot0 = dir.path().join("stress.0"); // seq3
            let slot1 = dir.path().join("stress.1"); // seq4
            let slot2 = dir.path().join("stress.2"); // seq5

            match scenario {
                StressScenario::LatestFooterCrcCorrupt => corrupt_slot_footer_crc(&slot2),
                StressScenario::LatestTruncated => truncate_slot(&slot2, 12),
                StressScenario::LatestHeaderOnly => truncate_slot(&slot2, 32),
                StressScenario::TwoLatestFooterCrcCorrupt => {
                    corrupt_slot_footer_crc(&slot2);
                    corrupt_slot_footer_crc(&slot1);
                }
                StressScenario::LatestHeaderOnlyAndSecondCorrupt => {
                    truncate_slot(&slot2, 32);
                    corrupt_slot_footer_crc(&slot1);
                }
                StressScenario::AllFooterCrcCorrupt => {
                    corrupt_slot_footer_crc(&slot0);
                    corrupt_slot_footer_crc(&slot1);
                    corrupt_slot_footer_crc(&slot2);
                }
                StressScenario::AllTruncated => {
                    truncate_slot(&slot0, 8);
                    truncate_slot(&slot1, 8);
                    truncate_slot(&slot2, 8);
                }
                StressScenario::AllHeaderOnly => {
                    truncate_slot(&slot0, 32);
                    truncate_slot(&slot1, 32);
                    truncate_slot(&slot2, 32);
                }
            }

            match expected {
                ExpectedOutcome::LoadedId(expected_id) => {
                    let loaded = vault
                        .load()
                        .unwrap_or_else(|e| panic!("{name}: load failed unexpectedly: {e}"));
                    assert_eq!(loaded.id, expected_id, "{name}");
                }
                ExpectedOutcome::NotFound => {
                    let result = vault.load();
                    assert!(
                        matches!(
                            result,
                            Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
                        ),
                        "{name}: expected NotFound, got {result:?}"
                    );
                }
            }
        }
    }

    /// JSONシリアライズのサイズ・復元整合性を small/medium/large ケースで検証。
    #[test]
    fn test_json_serialize_cases() {
        let cases = [
            ("small", build_json_case_data(1, 32, 256)),
            ("medium", build_json_case_data(2, 512, 4 * 1024)),
            ("large", build_json_case_data(3, 4096, 32 * 1024)),
        ];

        let mut prev_size = 0usize;
        for (name, case) in cases {
            let encoded = serde_json::to_vec(&case)
                .unwrap_or_else(|e| panic!("{name}: failed to serialize JSON: {e}"));
            assert!(
                !encoded.is_empty(),
                "{name}: serialized JSON should not be empty"
            );
            assert!(
                encoded.len() > prev_size,
                "{name}: serialized JSON size should grow with input size"
            );

            let decoded: JsonCaseData = serde_json::from_slice(&encoded)
                .unwrap_or_else(|e| panic!("{name}: failed to deserialize JSON: {e}"));
            assert_eq!(decoded, case, "{name}: JSON roundtrip mismatch");

            prev_size = encoded.len();
        }
    }

    /// Phase1の値域確認として、DataClass境界とエラー分類を検証。
    #[test]
    fn test_phase1_value_range() {
        let data_class_cases = [
            ("empty_payload", 0usize, crate::DataClass::Small),
            (
                "max_small_payload",
                crate::SMALL_DATA_THRESHOLD_BYTES - 1,
                crate::DataClass::Small,
            ),
            (
                "large_payload_boundary",
                crate::SMALL_DATA_THRESHOLD_BYTES,
                crate::DataClass::Large,
            ),
        ];

        for (name, payload_len, expected) in data_class_cases {
            let actual = crate::DataClass::from_payload_len(payload_len);
            assert_eq!(actual, expected, "{name}: DataClass mismatch");
        }

        let error_class_cases = [
            (
                "recoverable_crc",
                Error::CrcMismatch,
                crate::error::ErrorClass::Recoverable,
            ),
            (
                "fatal_invalid_protection_level",
                Error::InvalidProtectionLevel,
                crate::error::ErrorClass::Fatal,
            ),
            (
                "recoverable_not_found",
                Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
                crate::error::ErrorClass::Recoverable,
            ),
        ];

        for (name, error, expected) in error_class_cases {
            assert_eq!(error.class(), expected, "{name}: error class mismatch");
        }
    }

    /// Phase1の正常系として、TMR小サイズ保存/復元と原子的書き込みを検証。
    #[test]
    fn test_phase1_ok_cases() {
        let small_data = TestData {
            id: 101,
            message: "small tmr payload".to_string(),
        };

        let memory_cases = [(
            "memory_save_small_roundtrip",
            MemoryBackend::new(3),
            small_data.clone(),
        )];

        for (name, backend, data) in memory_cases {
            let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3);
            vault
                .save_small(&data)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

            let loaded = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(loaded, data, "{name}: small roundtrip mismatch");

            backend
                .mutate_slot(1, |bytes| {
                    let payload_offset = crate::tmr::TMR_HEADER_GROUP_BYTES + 5;
                    bytes[payload_offset] ^= 0b0000_0010;
                })
                .unwrap_or_else(|e| panic!("{name}: mutate_slot failed: {e}"));

            let recovered = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small after corruption failed: {e}"));
            assert_eq!(recovered, data, "{name}: TMR recovery mismatch");
        }

        let file_cases = [("filesystem_save_small_atomic", small_data.clone())];

        for (name, data) in file_cases {
            let dir = tempfile::tempdir().unwrap();
            let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "phase1");

            vault
                .save_small(&data)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

            let loaded = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(loaded, data, "{name}: filesystem roundtrip mismatch");

            let mut entries = fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            entries.sort();
            assert!(
                entries.iter().all(|entry| !entry.contains(".tmp-")),
                "{name}: temporary file should not remain: {entries:?}"
            );
        }
    }

    /// Phase1の異常系として、small APIのサイズ超過とsmall未存在時エラーを検証。
    #[test]
    fn test_phase1_error_cases() {
        #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
        struct LargeCaseData {
            payload: Vec<u8>,
        }

        let too_large = LargeCaseData {
            payload: vec![0xEF; crate::SMALL_DATA_THRESHOLD_BYTES + 512],
        };
        let large_backend = MemoryBackend::new(3);
        let large_vault = ReliableVault::<LargeCaseData, _>::new(large_backend, 3);

        let save_small_error = match large_vault.save_small(&too_large) {
            Ok(_) => panic!("save_small should fail for large payload"),
            Err(error) => error,
        };
        assert!(
            matches!(
                save_small_error,
                Error::Io(ref error) if error.kind() == std::io::ErrorKind::InvalidInput
            ),
            "save_small large payload error mismatch: {save_small_error:?}"
        );

        let dir = tempfile::tempdir().unwrap();
        let large_only_vault = ReliableVault::<JsonCaseData>::new_with_fs(dir.path(), "large-only");
        let large_case = build_json_case_data(90, 2048, 16 * 1024);
        large_only_vault
            .save(&large_case, ProtectionLevel::Medium)
            .unwrap();

        let load_small_error = match large_only_vault.load_small() {
            Ok(_) => panic!("load_small should fail when only large slots exist"),
            Err(error) => error,
        };
        assert!(
            matches!(
                load_small_error,
                Error::Io(ref error) if error.kind() == std::io::ErrorKind::NotFound
            ),
            "load_small missing small slot error mismatch: {load_small_error:?}"
        );
    }

    /// Phase2の正常系として、RS保存/復元・範囲読込・2シャード欠損復元を検証。
    #[test]
    fn test_phase2_ok_cases() {
        let data = build_large_case_data(501, RS_DATA_BYTES_PER_SEGMENT + 2048);
        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<LargeCaseData, _>::new(backend.clone(), 3);

        vault
            .save_large(&data)
            .unwrap_or_else(|e| panic!("save_large failed: {e}"));

        let loaded = vault
            .load_large()
            .unwrap_or_else(|e| panic!("load_large failed: {e}"));
        assert_eq!(loaded, data, "phase2 roundtrip mismatch");

        let serialized = bincode::serialize(&data).unwrap();
        let range_start = RS_DATA_BYTES_PER_SEGMENT.saturating_sub(64);
        let range_end = (RS_DATA_BYTES_PER_SEGMENT + 64).min(serialized.len());
        let partial = vault
            .load_large_range(range_start..range_end)
            .unwrap_or_else(|e| panic!("load_large_range failed: {e}"));
        assert_eq!(
            partial,
            serialized[range_start..range_end].to_vec(),
            "phase2 range read mismatch"
        );

        backend
            .mutate_slot(1, |bytes| {
                corrupt_rs_segment_shards(bytes, &[0, 1]);
            })
            .unwrap();

        let recovered = vault
            .load_large()
            .unwrap_or_else(|e| panic!("load_large after 2-shard corruption failed: {e}"));
        assert_eq!(recovered, data, "phase2 2-shard recovery mismatch");

        let auto_data = build_large_case_data(502, RS_DATA_BYTES_PER_SEGMENT / 3);
        vault
            .save(&auto_data, ProtectionLevel::Medium)
            .unwrap_or_else(|e| panic!("save(auto large) failed: {e}"));
        let auto_loaded = vault
            .load()
            .unwrap_or_else(|e| panic!("load(auto large) failed: {e}"));
        assert_eq!(auto_loaded, auto_data, "phase2 auto route mismatch");
    }

    /// Phase2の異常系として、save_large境界・不正範囲・復元不能欠損を検証。
    #[test]
    fn test_phase2_error_cases() {
        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<TestData, _>::new(backend, 3);
        let small = TestData {
            id: 1,
            message: "small".to_string(),
        };

        let save_large_error = match vault.save_large(&small) {
            Ok(_) => panic!("save_large should fail for small payload"),
            Err(error) => error,
        };
        assert!(
            matches!(
                save_large_error,
                Error::Io(ref error) if error.kind() == std::io::ErrorKind::InvalidInput
            ),
            "save_large small payload error mismatch: {save_large_error:?}"
        );

        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<LargeCaseData, _>::new(backend.clone(), 3);
        let data = build_large_case_data(503, RS_DATA_BYTES_PER_SEGMENT + 256);
        vault.save_large(&data).unwrap();

        let invalid_range_error = match vault.load_large_range(128..64) {
            Ok(_) => panic!("load_large_range should fail for invalid range"),
            Err(error) => error,
        };
        assert!(
            matches!(
                invalid_range_error,
                Error::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidInput
            ),
            "load_large_range invalid range mismatch: {invalid_range_error:?}"
        );

        backend
            .mutate_slot(1, |bytes| {
                corrupt_rs_segment_shards(bytes, &[0, 1, 2]);
            })
            .unwrap();

        let load_error = match vault.load_large() {
            Ok(_) => panic!("load_large should fail after > parity shard erasures"),
            Err(error) => error,
        };
        assert!(
            matches!(load_error, Error::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
            "load_large too-many-erasures mismatch: {load_error:?}"
        );

        let range_error = match vault.load_large_range(0..256) {
            Ok(_) => panic!("load_large_range should fail after > parity shard erasures"),
            Err(error) => error,
        };
        assert!(
            matches!(range_error, Error::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
            "load_large_range too-many-erasures mismatch: {range_error:?}"
        );
    }

    /// Phase3の値域確認として、スクラブ修復予算の境界動作を検証。
    #[test]
    fn test_phase3_value_range() {
        let cases = [
            ("budget_zero", 0usize, 0usize, 1usize),
            ("budget_one", 1usize, 1usize, 0usize),
        ];

        for (name, budget, expected_repaired, expected_budget_skipped) in cases {
            let backend = MemoryBackend::new(3);
            let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3);
            let data = TestData {
                id: 300,
                message: format!("phase3-{name}"),
            };
            vault
                .save_small(&data)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

            let (slot_index, original_bytes) = first_present_slot(&backend, 3);
            backend
                .mutate_slot(slot_index, |bytes| {
                    bytes[0] ^= 0b0000_0001;
                })
                .unwrap_or_else(|e| panic!("{name}: mutate_slot failed: {e}"));
            let corrupted_bytes = backend
                .read_slot(slot_index)
                .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));

            let report = vault
                .scrub_with_budget(budget)
                .unwrap_or_else(|e| panic!("{name}: scrub_with_budget failed: {e}"));
            assert_eq!(report.scanned_slots, 3, "{name}: scanned slots mismatch");
            assert_eq!(
                report.repaired_slots, expected_repaired,
                "{name}: repaired slots mismatch"
            );
            assert_eq!(
                report.budget_skipped_slots, expected_budget_skipped,
                "{name}: budget skipped slots mismatch"
            );

            let loaded = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(loaded, data, "{name}: load_small mismatch after scrub");

            let current_bytes = backend
                .read_slot(slot_index)
                .unwrap_or_else(|e| panic!("{name}: read_slot after scrub failed: {e}"));
            if expected_repaired == 0 {
                assert_eq!(
                    current_bytes, corrupted_bytes,
                    "{name}: bytes should remain corrupted when budget is zero"
                );
            } else {
                assert_eq!(
                    current_bytes, original_bytes,
                    "{name}: scrub should restore canonical bytes"
                );
            }
        }
    }

    /// Phase3の正常系として、世代ローテーションとスクラブ修復を検証。
    #[test]
    fn test_phase3_ok_cases() {
        let cases = [("rotation_and_scrub", 5usize, 7u64, 1usize)];

        for (name, num_slots, save_count, budget) in cases {
            let backend = MemoryBackend::new(num_slots);
            let vault = ReliableVault::<TestData, _>::new(backend.clone(), num_slots);

            for id in 1..=save_count {
                let data = TestData {
                    id: id as u32,
                    message: format!("{name}-{id}"),
                };
                vault
                    .save_small(&data)
                    .unwrap_or_else(|e| panic!("{name}: save_small({id}) failed: {e}"));
            }

            let latest = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(latest.id as u64, save_count, "{name}: latest id mismatch");

            let mut sequence_entries = collect_slot_sequences(&backend, num_slots);
            sequence_entries.sort_by_key(|(_, sequence)| *sequence);
            let expected_sequences =
                ((save_count - num_slots as u64 + 1)..=save_count).collect::<Vec<_>>();
            let actual_sequences = sequence_entries
                .iter()
                .map(|(_, sequence)| *sequence)
                .collect::<Vec<_>>();
            assert_eq!(
                actual_sequences, expected_sequences,
                "{name}: slot generation range mismatch"
            );

            let (latest_slot_index, _) = sequence_entries
                .last()
                .copied()
                .unwrap_or_else(|| panic!("{name}: latest slot not found"));
            let original_latest_bytes = backend
                .read_slot(latest_slot_index)
                .unwrap_or_else(|e| panic!("{name}: read latest slot failed: {e}"));

            backend
                .mutate_slot(latest_slot_index, |bytes| {
                    bytes[0] ^= 0b0000_0010;
                })
                .unwrap_or_else(|e| panic!("{name}: mutate latest slot failed: {e}"));

            let report = vault
                .scrub_with_budget(budget)
                .unwrap_or_else(|e| panic!("{name}: scrub_with_budget failed: {e}"));
            assert_eq!(report.repaired_slots, 1, "{name}: repaired slots mismatch");

            let repaired_latest_bytes = backend
                .read_slot(latest_slot_index)
                .unwrap_or_else(|e| panic!("{name}: read latest slot after scrub failed: {e}"));
            assert_eq!(
                repaired_latest_bytes, original_latest_bytes,
                "{name}: scrub should restore latest slot bytes"
            );
        }
    }

    /// Phase3の異常系として、スクラブ再書き込み失敗時のエラー伝播を検証。
    #[test]
    fn test_phase3_error_cases() {
        let cases = [("permission_denied", std::io::ErrorKind::PermissionDenied)];

        for (name, expected_kind) in cases {
            let backend = MemoryBackend::new(3);
            let warmup_vault = ReliableVault::<TestData, _>::new(backend.clone(), 3);
            let data = TestData {
                id: 401,
                message: format!("{name}-data"),
            };
            warmup_vault
                .save_small(&data)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

            let (slot_index, _) = first_present_slot(&backend, 3);
            backend
                .mutate_slot(slot_index, |bytes| {
                    bytes[0] ^= 0b0000_0001;
                })
                .unwrap_or_else(|e| panic!("{name}: mutate_slot failed: {e}"));

            let vault = ReliableVault::<TestData, _>::new(
                WriteFailBackend::new(backend.clone(), slot_index),
                3,
            );
            let scrub_error = match vault.scrub_with_budget(1) {
                Ok(_) => panic!("{name}: scrub_with_budget should fail"),
                Err(error) => error,
            };

            assert!(
                matches!(scrub_error, Error::Io(ref e) if e.kind() == expected_kind),
                "{name}: scrub error mismatch: {scrub_error:?}"
            );
        }
    }

    /// Phase4の値域確認として、データ種別ごとの圧縮有効/無効を検証。
    #[test]
    fn test_phase4_value_range() {
        #[derive(Debug, Clone, Copy)]
        enum SaveRoute {
            SaveSmall,
            SaveLarge,
        }

        let compression_available = cfg!(feature = "zstd-compression");
        let cases = [
            (
                "small_compression_disabled",
                CompressionPolicy::disabled(),
                SaveRoute::SaveSmall,
                false,
            ),
            (
                "small_compression_enabled",
                CompressionPolicy::enabled().with_large_enabled(false),
                SaveRoute::SaveSmall,
                compression_available,
            ),
            (
                "large_compression_enabled",
                CompressionPolicy::enabled().with_small_enabled(false),
                SaveRoute::SaveLarge,
                compression_available,
            ),
        ];

        for (name, policy, route, expected_compressed) in cases {
            let backend = MemoryBackend::new(3);

            match route {
                SaveRoute::SaveSmall => {
                    let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3)
                        .with_compression_policy(policy);
                    let data = TestData {
                        id: 501,
                        message: "a".repeat(2048),
                    };

                    vault
                        .save_small(&data)
                        .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));

                    let loaded = vault
                        .load_small()
                        .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
                    assert_eq!(loaded, data, "{name}: load_small mismatch");

                    let (slot_index, _) = first_present_slot(&backend, 3);
                    let slot_bytes = backend
                        .read_slot(slot_index)
                        .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));
                    let slot = decode_slot_payload(&slot_bytes)
                        .unwrap_or_else(|e| panic!("{name}: decode_slot_payload failed: {e}"));
                    assert_eq!(
                        is_compressed_payload(&slot.payload),
                        expected_compressed,
                        "{name}: compressed marker mismatch"
                    );
                }
                SaveRoute::SaveLarge => {
                    let vault = ReliableVault::<LargeCaseData, _>::new(backend.clone(), 3)
                        .with_compression_policy(policy);
                    let data = LargeCaseData {
                        id: 502,
                        payload: vec![0u8; RS_DATA_BYTES_PER_SEGMENT + 4096],
                    };

                    vault
                        .save_large(&data)
                        .unwrap_or_else(|e| panic!("{name}: save_large failed: {e}"));

                    let loaded = vault
                        .load_large()
                        .unwrap_or_else(|e| panic!("{name}: load_large failed: {e}"));
                    assert_eq!(loaded, data, "{name}: load_large mismatch");

                    let (slot_index, _) = first_present_slot(&backend, 3);
                    let slot_bytes = backend
                        .read_slot(slot_index)
                        .unwrap_or_else(|e| panic!("{name}: read_slot failed: {e}"));
                    let slot = decode_slot_payload(&slot_bytes)
                        .unwrap_or_else(|e| panic!("{name}: decode_slot_payload failed: {e}"));
                    assert_eq!(
                        is_compressed_payload(&slot.payload),
                        expected_compressed,
                        "{name}: compressed marker mismatch"
                    );
                }
            }
        }
    }

    /// Phase4の正常系として、圧縮維持とフォールト注入回復率を検証。
    #[test]
    fn test_phase4_ok_cases() {
        let data = LargeCaseData {
            id: 601,
            payload: vec![7u8; RS_DATA_BYTES_PER_SEGMENT + 8192],
        };

        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<LargeCaseData, _>::new(backend.clone(), 3)
            .with_compression_policy(CompressionPolicy::enabled());
        vault
            .save_large(&data)
            .unwrap_or_else(|e| panic!("phase4_large_compress: save_large failed: {e}"));

        let serialized = bincode::serialize(&data).unwrap();
        let range_start = RS_DATA_BYTES_PER_SEGMENT.saturating_sub(128);
        let range_end = (range_start + 512).min(serialized.len());
        let partial = vault
            .load_large_range(range_start..range_end)
            .unwrap_or_else(|e| panic!("phase4_large_compress: load_large_range failed: {e}"));
        assert_eq!(
            partial,
            serialized[range_start..range_end].to_vec(),
            "phase4_large_compress: load_large_range mismatch"
        );

        let small = TestData {
            id: 602,
            message: "x".repeat(1024),
        };
        let bitflip_cases = [
            (
                "independent",
                BitFlipConfig::new(1, Some(7001), BitFlipTarget::FullFrame),
            ),
            (
                "mbu",
                BitFlipConfig::mbu(2, Some(7002), BitFlipTarget::FullFrame, 2),
            ),
        ];

        for (name, config) in bitflip_cases {
            let backend = MemoryBackend::new(3).with_bitflip_for_tests(config);
            let vault = ReliableVault::<TestData, _>::new(backend, 3)
                .with_compression_policy(CompressionPolicy::enabled());
            vault
                .save_small(&small)
                .unwrap_or_else(|e| panic!("{name}: save_small failed: {e}"));
            let loaded = vault
                .load_small()
                .unwrap_or_else(|e| panic!("{name}: load_small failed: {e}"));
            assert_eq!(loaded, small, "{name}: bitflip recovery mismatch");
        }

        let backend = MemoryBackend::new(3);
        let vault = ReliableVault::<TestData, _>::new(backend.clone(), 3)
            .with_compression_policy(CompressionPolicy::enabled());
        let tre_data = TestData {
            id: 603,
            message: "tre-source".repeat(200),
        };
        vault
            .save_small(&tre_data)
            .unwrap_or_else(|e| panic!("tre: save_small failed: {e}"));

        let (slot_index, source_bytes) = first_present_slot(&backend, 3);
        let source_snapshot = source_bytes.clone();
        let tre_config = TreConfig::mbu(48, Some(9001), BitFlipTarget::HeaderOnly, 8, 3);
        let mut reader = TemporaryReadErrorReader::from_bytes_idle(source_bytes, tre_config)
            .unwrap_or_else(|e| panic!("tre: reader init failed: {e}"));

        let mut saw_injected = false;
        let mut recovered = false;
        for attempt in 0..=tre_config.settle_reads {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut reader, &mut bytes)
                .unwrap_or_else(|e| panic!("tre: read_to_end failed: {e}"));

            if attempt == 0 && bytes != source_snapshot {
                saw_injected = true;
            }

            if decode_slot_payload(&bytes).is_ok() {
                recovered = true;
                break;
            }
            reader.restart_read();
        }
        assert!(saw_injected, "tre: should inject transient bit flips");
        assert!(recovered, "tre: slot should recover after retries");

        backend
            .mutate_slot(slot_index, |bytes| {
                bytes[0..32].fill(0);
            })
            .unwrap();

        vault
            .save_small(&TestData {
                id: 604,
                message: "recovered".to_string(),
            })
            .unwrap();
    }

    /// Phase4の異常系として、破壊パターン別の失敗モードを検証。
    #[test]
    fn test_phase4_error_cases() {
        let cases = [
            ("all_slots_zeroed", 0u8),
            ("all_slots_header_corrupt", 1u8),
            ("all_slots_truncated", 2u8),
        ];

        for (name, mode) in cases {
            let dir = tempfile::tempdir().unwrap();
            let vault = ReliableVault::<TestData>::new_with_fs(dir.path(), "phase4-error")
                .with_compression_policy(CompressionPolicy::enabled());

            for id in 1..=3 {
                vault
                    .save_small(&TestData {
                        id,
                        message: format!("{name}-{id}"),
                    })
                    .unwrap_or_else(|e| panic!("{name}: save_small({id}) failed: {e}"));
            }

            for slot in 0..3 {
                let path = dir.path().join(format!("phase4-error.{slot}"));
                match mode {
                    0 => {
                        let mut bytes = fs::read(&path).unwrap();
                        bytes.fill(0);
                        fs::write(&path, bytes).unwrap();
                    }
                    1 => {
                        let mut bytes = fs::read(&path).unwrap();
                        let header_len = 32.min(bytes.len());
                        bytes[0..header_len].fill(0);
                        fs::write(&path, bytes).unwrap();
                    }
                    _ => {
                        truncate_slot(&path, 8);
                    }
                }
            }

            let load_error = match vault.load_small() {
                Ok(_) => panic!("{name}: load_small should fail"),
                Err(error) => error,
            };
            assert!(
                matches!(load_error, Error::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound),
                "{name}: expected NotFound, got {load_error:?}"
            );
        }

        let invalid_tre = TreConfig::new(8, Some(1), BitFlipTarget::FullFrame, 0);
        let tre_error = match TemporaryReadErrorReader::from_bytes(vec![0x11; 128], invalid_tre) {
            Ok(_) => panic!("tre_invalid_settle: reader init should fail"),
            Err(error) => error,
        };
        assert!(
            matches!(tre_error, bitflip::TreError::InvalidSettleReads),
            "tre_invalid_settle: unexpected error: {tre_error}"
        );
    }
}
