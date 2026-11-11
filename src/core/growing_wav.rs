//! Bounded, shared reads of a WAV whose writer has not finalized its header.
use super::meeting_watch::{WavHeader, shared_read, wav_header};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

pub const RATE: u64 = 16_000;
pub struct GrowingWav {
    file: File,
    header: WavHeader,
}
impl GrowingWav {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = shared_read(path)?;
        let length = file.metadata()?.len();
        let header = wav_header(&mut file, length)?;
        if !matches!(
            (header.encoding_tag, header.bits_per_sample),
            (1, 16) | (3, 32)
        ) || header.channels > 32
            || !(8_000..=192_000).contains(&header.sample_rate)
            || header.block_align != header.channels * (header.bits_per_sample / 8)
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(Self { file, header })
    }
    /// Current complete frames on the 16 kHz clock, never the declared data size.
    pub fn len(&self) -> io::Result<u64> {
        Ok(self.frames()? * RATE / u64::from(self.header.sample_rate))
    }
    fn frames(&self) -> io::Result<u64> {
        let bytes = self
            .file
            .metadata()?
            .len()
            .checked_sub(self.header.data_offset)
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        Ok(bytes / u64::from(self.header.block_align))
    }
    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }
    /// Absolute output-clock offsets make adjacent reads independent of polling cadence.
    /// A window is bounded to 310 seconds. A short last frame remains for the next read.
    pub fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<f32>> {
        let frames = self.frames()?;
        if end < start
            || end - start > 310 * RATE
            || end > frames * RATE / u64::from(self.header.sample_rate)
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if end == start {
            return Ok(Vec::new());
        }
        let rate = u64::from(self.header.sample_rate);
        let first = start * rate / RATE;
        let last = ((end * rate).div_ceil(RATE) + 1).min(frames);
        let align = usize::from(self.header.block_align);
        let byte_count = (last - first)
            .checked_mul(align as u64)
            .filter(|n| *n <= 128 * 1024 * 1024)
            .ok_or(io::ErrorKind::InvalidData)?;
        let mut bytes = vec![0; byte_count as usize];
        self.file.seek(SeekFrom::Start(
            self.header.data_offset + first * align as u64,
        ))?;
        self.file.read_exact(&mut bytes)?;
        let mono: Vec<f32> = bytes
            .chunks_exact(align)
            .map(|frame| {
                frame
                    .chunks_exact(usize::from(self.header.bits_per_sample / 8))
                    .map(|s| {
                        let value = if self.header.encoding_tag == 3 {
                            f32::from_le_bytes(s.try_into().unwrap())
                        } else {
                            f32::from(i16::from_le_bytes(s.try_into().unwrap())) / 32768.0
                        };
                        if value.is_finite() {
                            value.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        }
                    })
                    .sum::<f32>()
                    / f32::from(self.header.channels)
            })
            .collect();
        // Average each output interval when downsampling to suppress high-frequency aliasing.
        Ok((start..end)
            .map(|i| {
                let a = i * rate / RATE - first;
                let b = ((i + 1) * rate / RATE - first)
                    .max(a + 1)
                    .min(mono.len() as u64);
                mono[a as usize..b as usize].iter().sum::<f32>() / (b - a) as f32
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn synthetic_writer_appends_with_zero_and_stale_sizes_and_partial_frames() {
        for float in [false, true] {
            for declared in [0, 4] {
                let temp = tempfile::tempdir().unwrap();
                let path = temp.path().join("growing.wav");
                let spec = hound::WavSpec {
                    channels: 2,
                    sample_rate: 48000,
                    bits_per_sample: if float { 32 } else { 16 },
                    sample_format: if float {
                        hound::SampleFormat::Float
                    } else {
                        hound::SampleFormat::Int
                    },
                };
                let mut cursor = std::io::Cursor::new(Vec::new());
                {
                    let writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
                    writer.finalize().unwrap();
                }
                let mut header = cursor.into_inner();
                let data = header.windows(4).position(|w| w == b"data").unwrap();
                header[4..8].copy_from_slice(&0u32.to_le_bytes());
                header[data + 4..data + 8].copy_from_slice(&(declared as u32).to_le_bytes());
                let mut writer = std::fs::File::create(&path).unwrap();
                writer.write_all(&header).unwrap();
                writer.flush().unwrap();
                let mut reader = GrowingWav::open(&path).unwrap();
                assert_eq!(reader.len().unwrap(), 0);
                let sample = if float {
                    0.5f32.to_le_bytes().to_vec()
                } else {
                    16384i16.to_le_bytes().to_vec()
                };
                let frame = sample.repeat(2);
                writer.write_all(&frame.repeat(480)).unwrap();
                writer.write_all(&frame[..1]).unwrap();
                writer.flush().unwrap();
                assert_eq!(reader.len().unwrap(), 160);
                assert_eq!(reader.read(0, 160).unwrap(), vec![0.5; 160]);
                writer.write_all(&frame[1..]).unwrap();
                writer.write_all(&frame.repeat(479)).unwrap();
                writer.flush().unwrap();
                assert_eq!(reader.len().unwrap(), 320);
                assert_eq!(reader.read(160, 320).unwrap(), vec![0.5; 160]);
                assert!(reader.read(0, 321).is_err());
            }
        }
    }
    #[test]
    fn writer_thread_and_reader_recover_each_appended_frame_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("threaded.wav");
        let writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        writer.finalize().unwrap();
        let (ready, samples) = std::sync::mpsc::channel();
        let (ack, next) = std::sync::mpsc::channel();
        let source = path.clone();
        let writer = std::thread::spawn(move || {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(source)
                .unwrap();
            for i in 1..=20i16 {
                for _ in 0..160 {
                    file.write_all(&(i * 100).to_le_bytes()).unwrap();
                }
                file.flush().unwrap();
                ready.send(i).unwrap();
                next.recv().unwrap();
            }
        });
        let mut reader = GrowingWav::open(&path).unwrap();
        let mut cursor = 0;
        for i in samples {
            let end = reader.len().unwrap();
            let frames = reader.read(cursor, end).unwrap();
            assert_eq!(frames, vec![f32::from(i * 100) / 32768.0; 160]);
            cursor = end;
            ack.send(()).unwrap();
        }
        writer.join().unwrap();
        assert_eq!(cursor, 3200);
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(0)
            .unwrap();
        assert!(reader.read(0, 1).is_err());
    }
    #[test]
    fn upsampling_and_nonfinite_float_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.wav");
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for sample in [f32::NAN, 2.0, -2.0, 0.25] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let mut reader = GrowingWav::open(&path).unwrap();
        assert_eq!(
            reader.read(0, 8).unwrap(),
            vec![0.0, 0.0, 1.0, 1.0, -1.0, -1.0, 0.25, 0.25]
        );
    }
}
