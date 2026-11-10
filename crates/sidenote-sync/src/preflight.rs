//! Format guards for pinned Automerge 0.12. Runs inside the resource-limited worker.
//! This parser only bounds framing/inflation. Automerge still verifies checksums and ops.
use anyhow::{Result, ensure};
use std::io::Read;
pub const MAX_DOC: usize = 32 * 1024 * 1024;
pub const MAX_BULK: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 50_000;

pub(crate) struct Reader<'a> {
    pub bytes: &'a [u8],
    pub offset: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(n)
            .ok_or_else(|| anyhow::anyhow!("length overflow"))?;
        ensure!(end <= self.bytes.len(), "truncated input");
        let data = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(data)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn varint(&mut self) -> Result<usize> {
        let mut out = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            ensure!(shift < 63 || b <= 1, "varint overflow");
            out |= u64::from(b & 127) << shift;
            if b & 128 == 0 {
                return usize::try_from(out).map_err(Into::into);
            }
        }
        anyhow::bail!("varint overflow")
    }
    fn skip_signed(&mut self) -> Result<()> {
        for _ in 0..10 {
            if self.byte()? & 128 == 0 {
                return Ok(());
            }
        }
        anyhow::bail!("invalid signed varint")
    }
    pub fn blob(&mut self, cap: usize) -> Result<&'a [u8]> {
        let n = self.varint()?;
        ensure!(n <= cap, "blob cap");
        self.take(n)
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.varint()?;
        ensure!(n <= MAX_ITEMS, "item cap");
        Ok(n)
    }
    fn hashes(&mut self) -> Result<()> {
        let n = self.count()?;
        self.take(
            n.checked_mul(32)
                .ok_or_else(|| anyhow::anyhow!("hash cap"))?,
        )?;
        Ok(())
    }
    fn actors(&mut self) -> Result<()> {
        for _ in 0..self.count()? {
            self.blob(128)?;
        }
        Ok(())
    }
}

pub fn sync(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= MAX_BULK, "sync cap");
    let mut r = Reader::new(bytes);
    ensure!([0x42, 0x43].contains(&r.byte()?), "sync version");
    r.hashes()?;
    r.hashes()?;
    for _ in 0..r.count()? {
        r.hashes()?;
        let bloom = r.blob(MAX_DOC)?;
        if !bloom.is_empty() {
            let mut b = Reader::new(bloom);
            let entries = b.varint()?;
            let bits = b.varint()?;
            let probes = b.varint()?;
            ensure!(probes <= 64, "bloom probes cap");
            let bit_count = entries
                .checked_mul(bits)
                .ok_or_else(|| anyhow::anyhow!("bloom overflow"))?;
            ensure!(entries == 0 || bit_count > 0, "zero-bit bloom with entries");
            let capacity = bit_count
                .checked_add(7)
                .ok_or_else(|| anyhow::anyhow!("bloom overflow"))?
                / 8;
            ensure!(
                capacity <= MAX_DOC && b.rest().len() == capacity,
                "bloom length"
            );
        }
    }
    let mut budget = MAX_BULK;
    for _ in 0..r.count()? {
        chunks(r.blob(MAX_BULK)?, &mut budget)?;
    }
    if !r.rest().is_empty() {
        r.blob(64)?;
    }
    ensure!(r.rest().is_empty(), "trailing sync data");
    Ok(())
}

pub fn document(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= MAX_DOC, "document cap");
    let mut budget = MAX_BULK;
    chunks(bytes, &mut budget)
}

pub fn split_chunks(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    let mut r = Reader::new(bytes);
    let mut chunks = Vec::new();
    while !r.rest().is_empty() {
        ensure!(chunks.len() < MAX_ITEMS, "chunk count cap");
        let start = r.offset;
        ensure!(r.take(4)? == [0x85, 0x6f, 0x4a, 0x83], "chunk magic");
        r.take(4)?;
        r.byte()?;
        let len = r.varint()?;
        r.take(len)?;
        chunks.push(&bytes[start..r.offset]);
    }
    Ok(chunks)
}

fn inflate(bytes: &[u8], budget: &mut usize) -> Result<Vec<u8>> {
    let cap = (*budget).min(MAX_DOC);
    let mut decoder = flate2::bufread::DeflateDecoder::new(bytes).take(cap as u64 + 1);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    ensure!(out.len() <= cap, "inflated cap");
    *budget -= out.len();
    Ok(out)
}
fn columns(r: &mut Reader<'_>) -> Result<Vec<(bool, usize)>> {
    let n = r.varint()?;
    ensure!(n <= 1024, "column count cap");
    let mut columns = Vec::with_capacity(n);
    for _ in 0..n {
        let spec = r.varint()?;
        let len = r.varint()?;
        ensure!(len <= MAX_DOC, "column cap");
        columns.push((spec & 8 != 0, len));
    }
    Ok(columns)
}
fn column_data(r: &mut Reader<'_>, columns: &[(bool, usize)], budget: &mut usize) -> Result<()> {
    for &(compressed, len) in columns {
        let data = r.take(len)?;
        if compressed {
            inflate(data, budget)?;
        } else {
            ensure!(len <= *budget, "column budget");
            *budget -= len;
        }
    }
    Ok(())
}
fn change(bytes: &[u8], budget: &mut usize) -> Result<()> {
    let mut r = Reader::new(bytes);
    r.hashes()?;
    r.blob(128)?;
    r.varint()?;
    r.varint()?;
    r.skip_signed()?;
    r.blob(MAX_DOC)?;
    r.actors()?;
    let cols = columns(&mut r)?;
    ensure!(
        cols.iter().all(|(compressed, _)| !compressed),
        "compressed change column"
    );
    column_data(&mut r, &cols, budget)
}
fn chunks(bytes: &[u8], budget: &mut usize) -> Result<()> {
    for chunk in split_chunks(bytes)? {
        let mut header = Reader::new(chunk);
        header.take(8)?;
        let kind = header.byte()?;
        let len = header.varint()?;
        let body = header.take(len)?;
        match kind {
            0 => {
                let mut r = Reader::new(body);
                r.actors()?;
                r.hashes()?;
                let changes = columns(&mut r)?;
                let ops = columns(&mut r)?;
                column_data(&mut r, &changes, budget)?;
                column_data(&mut r, &ops, budget)?;
            }
            1 => change(body, budget)?,
            2 => {
                let uncompressed = inflate(body, budget)?;
                change(&uncompressed, budget)?;
            }
            3 => {
                let mut r = Reader::new(body);
                r.hashes()?;
                r.actors()?;
                let changes = columns(&mut r)?;
                column_data(&mut r, &changes, budget)?;
                let ops = columns(&mut r)?;
                column_data(&mut r, &ops, budget)?;
            }
            _ => anyhow::bail!("unknown chunk kind"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn f8_sync_wire_cap_and_trailing_bytes_are_rejected() {
        assert_eq!(
            sync(&vec![0; MAX_BULK + 1])
                .expect_err("sync wire cap")
                .to_string(),
            "sync cap"
        );
        assert_eq!(
            sync(&[0x42, 0, 0, 0, 0, 0, 0])
                .expect_err("trailing sync bytes")
                .to_string(),
            "trailing sync data"
        );
    }
    #[test]
    fn f8_compressed_chunk_is_bounded_through_document() -> Result<()> {
        use std::io::Write;
        let mut compressor =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        let block = [0u8; 64 * 1024];
        for _ in 0..=MAX_DOC / block.len() {
            compressor.write_all(&block)?;
        }
        let body = compressor.finish()?;
        let mut chunk = vec![0x85, 0x6f, 0x4a, 0x83, 0, 0, 0, 0, 2];
        let mut len = body.len();
        loop {
            let mut byte = (len & 127) as u8;
            len >>= 7;
            if len > 0 {
                byte |= 128;
            }
            chunk.push(byte);
            if len == 0 {
                break;
            }
        }
        chunk.extend_from_slice(&body);
        assert!(chunk.len() < MAX_DOC);
        assert_eq!(
            document(&chunk)
                .expect_err("inflated document cap")
                .to_string(),
            "inflated cap"
        );
        Ok(())
    }
    #[test]
    fn rejects_zero_bits_and_excessive_probes_before_automerge() {
        // V1, empty heads/need, one have, empty last_sync, length-prefixed bloom, zero changes.
        assert!(sync(&[0x42, 0, 0, 1, 0, 3, 1, 0, 7, 0]).is_err());
        assert!(sync(&[0x42, 0, 0, 1, 0, 4, 1, 8, 65, 0, 0]).is_err());
    }
    #[test]
    fn bounds_forged_lengths_and_inflated_output() -> Result<()> {
        assert!(sync(&[0x43, 255, 255, 255, 255, 255, 255, 255, 255, 255, 127]).is_err());
        use std::io::Write;
        let mut compressor =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        compressor.write_all(&[0; 4096])?;
        assert!(inflate(&compressor.finish()?, &mut 1024).is_err());
        Ok(())
    }
}
