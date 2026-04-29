use super::postings::DocId;

const POSTINGS_SIMPLE: u8 = 0;
const POSTINGS_SKIP: u8 = 1;
const SKIP_MIN_ENTRIES: usize = 256;
const SKIP_BLOCK_SIZE: usize = 128;
const SKIP_META_SIZE: usize = 20; // first_doc:u32 + last_doc:u32 + payload_offset:u32 + entry_count:u16 + next:u8 + loc:u8 + zone:u32

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostingEntry {
    pub doc_id: DocId,
    pub next_mask: u8,
    pub loc_mask: u8,
    pub zone_mask: u32,
}

/// Encode a u32 as variable-byte (7 bits/byte, MSB=1 = final byte).
#[inline(always)]
pub fn encode_u32(value: u32, buf: &mut Vec<u8>) {
    let mut v = value;
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(byte | 0x80); // final byte
            break;
        }
        buf.push(byte); // continuation
    }
}

/// Decode a u32 from variable-byte encoding at the given position.
/// Advances `pos` past the consumed bytes.
///
/// `inline(always)` is critical here: this is the inner loop of every
/// posting-list decode (millions of calls per query) and the compiler
/// can only specialize away the variable byte-count branch when the
/// caller's loop is fully inlined.
#[inline(always)]
pub fn decode_u32(data: &[u8], pos: &mut usize) -> u32 {
    let mut result: u32 = 0;
    let mut shift = 0;
    loop {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 != 0 {
            break;
        }
        shift += 7;
    }
    result
}

/// Encode a sorted slice of DocIds as delta + VByte.
///
/// Format: `[doc_count: vbyte] [delta_0: vbyte] [delta_1: vbyte] ...`
/// where delta_0 = ids[0], delta_i = ids[i] - ids[i-1].
#[allow(dead_code)]
pub fn encode_posting_list(doc_ids: &[DocId]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(doc_ids.len() * 2);
    encode_u32(doc_ids.len() as u32, &mut buf);

    let mut prev: u32 = 0;
    for &id in doc_ids {
        let delta = id - prev;
        encode_u32(delta, &mut buf);
        prev = id;
    }
    buf
}

pub fn encode_posting_entries(entries: &[PostingEntry]) -> Vec<u8> {
    if entries.len() >= SKIP_MIN_ENTRIES {
        return encode_posting_entries_skip(entries);
    }
    encode_posting_entries_simple(entries)
}

fn encode_posting_entries_simple(entries: &[PostingEntry]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(entries.len() * 4 + 1);
    buf.push(POSTINGS_SIMPLE);
    encode_u32(entries.len() as u32, &mut buf);
    encode_entries_payload(entries, &mut buf);
    buf
}

fn encode_posting_entries_skip(entries: &[PostingEntry]) -> Vec<u8> {
    let block_count = entries.len().div_ceil(SKIP_BLOCK_SIZE);
    let mut buf = Vec::with_capacity(entries.len() * 4 + block_count * SKIP_META_SIZE + 16);
    buf.push(POSTINGS_SKIP);
    encode_u32(entries.len() as u32, &mut buf);
    encode_u32(block_count as u32, &mut buf);

    let metadata_start = buf.len();
    buf.resize(metadata_start + block_count * SKIP_META_SIZE, 0);

    let mut payload = Vec::with_capacity(entries.len() * 4);
    for (block_idx, block) in entries.chunks(SKIP_BLOCK_SIZE).enumerate() {
        let payload_offset = payload.len() as u32;
        encode_entries_payload(block, &mut payload);
        let block_next_mask = block.iter().fold(0u8, |acc, entry| acc | entry.next_mask);
        let block_loc_mask = block.iter().fold(0u8, |acc, entry| acc | entry.loc_mask);
        let block_zone_mask = block.iter().fold(0u32, |acc, entry| acc | entry.zone_mask);

        let meta = metadata_start + block_idx * SKIP_META_SIZE;
        buf[meta..meta + 4].copy_from_slice(&block[0].doc_id.to_le_bytes());
        buf[meta + 4..meta + 8].copy_from_slice(&block[block.len() - 1].doc_id.to_le_bytes());
        buf[meta + 8..meta + 12].copy_from_slice(&payload_offset.to_le_bytes());
        buf[meta + 12..meta + 14].copy_from_slice(&(block.len() as u16).to_le_bytes());
        buf[meta + 14] = block_next_mask;
        buf[meta + 15] = block_loc_mask;
        buf[meta + 16..meta + 20].copy_from_slice(&block_zone_mask.to_le_bytes());
    }

    buf.extend_from_slice(&payload);
    buf
}

fn encode_entries_payload(entries: &[PostingEntry], buf: &mut Vec<u8>) {
    let mut prev: u32 = 0;
    for entry in entries {
        let delta = entry.doc_id - prev;
        encode_u32(delta, buf);
        buf.push(entry.next_mask);
        buf.push(entry.loc_mask);
        buf.extend_from_slice(&entry.zone_mask.to_le_bytes());
        prev = entry.doc_id;
    }
}

/// Decode a delta + VByte encoded posting list from a byte slice.
///
/// Reads from `data[offset .. offset + byte_len]`.
#[allow(dead_code)]
pub fn decode_posting_list(data: &[u8], offset: usize, byte_len: usize) -> Vec<DocId> {
    let end = offset + byte_len;
    let mut pos = offset;

    let count = decode_u32(data, &mut pos) as usize;
    let mut result = Vec::with_capacity(count);

    let mut prev: u32 = 0;
    for _ in 0..count {
        if pos >= end {
            break;
        }
        let delta = decode_u32(data, &mut pos);
        prev += delta;
        result.push(prev);
    }
    result
}

pub fn decode_posting_entries(data: &[u8], offset: usize, byte_len: usize) -> Vec<PostingEntry> {
    let end = offset + byte_len;
    let mut pos = offset;
    if pos >= end {
        return Vec::new();
    }

    match data[pos] {
        POSTINGS_SIMPLE => {
            pos += 1;
            decode_posting_entries_payload(data, &mut pos, end)
        }
        POSTINGS_SKIP => decode_posting_entries_skip(data, offset, byte_len),
        _ => decode_posting_entries_legacy(data, offset, byte_len),
    }
}

fn decode_posting_entries_legacy(data: &[u8], offset: usize, byte_len: usize) -> Vec<PostingEntry> {
    let end = offset + byte_len;
    let mut pos = offset;
    decode_posting_entries_payload(data, &mut pos, end)
}

fn decode_posting_entries_payload(data: &[u8], pos: &mut usize, end: usize) -> Vec<PostingEntry> {
    let count = decode_u32(data, pos) as usize;
    let mut result = Vec::with_capacity(count);

    let mut prev: u32 = 0;
    for _ in 0..count {
        if *pos >= end {
            break;
        }
        let delta = decode_u32(data, pos);
        prev += delta;
        if *pos + 2 > end {
            break;
        }
        let next_mask = data[*pos];
        let loc_mask = data[*pos + 1];
        *pos += 2;
        let zone_mask = if *pos + 4 <= end {
            let mask =
                u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            mask
        } else {
            0
        };
        result.push(PostingEntry {
            doc_id: prev,
            next_mask,
            loc_mask,
            zone_mask,
        });
    }
    result
}

fn decode_posting_entries_skip(data: &[u8], offset: usize, byte_len: usize) -> Vec<PostingEntry> {
    let Some(header) = SkipHeader::parse(data, offset, byte_len) else {
        return Vec::new();
    };
    let mut result = Vec::with_capacity(header.doc_count);
    for idx in 0..header.block_count {
        let meta = block_meta_from(data, header, idx);
        let mut pos = header.payload_start + meta.payload_offset as usize;
        let mut block = decode_block_payload(data, &mut pos, header.end, meta.entry_count as usize);
        result.append(&mut block);
    }
    result
}

pub fn posting_entry_count(data: &[u8], offset: usize, byte_len: usize) -> usize {
    let end = offset + byte_len;
    let mut pos = offset;
    if pos >= end {
        return 0;
    }
    match data[pos] {
        POSTINGS_SIMPLE => {
            pos += 1;
            decode_u32(data, &mut pos) as usize
        }
        POSTINGS_SKIP => SkipHeader::parse(data, offset, byte_len)
            .map(|header| header.doc_count)
            .unwrap_or(0),
        _ => decode_u32(data, &mut pos) as usize,
    }
}

pub fn is_skip_encoded(data: &[u8], offset: usize, byte_len: usize) -> bool {
    byte_len > 0 && offset < data.len() && data[offset] == POSTINGS_SKIP
}

#[derive(Clone, Copy)]
struct SkipHeader {
    doc_count: usize,
    block_count: usize,
    metadata_start: usize,
    payload_start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
struct BlockMeta {
    first_doc: DocId,
    last_doc: DocId,
    payload_offset: u32,
    entry_count: u16,
    next_mask: u8,
    loc_mask: u8,
    zone_mask: u32,
}

impl SkipHeader {
    fn parse(data: &[u8], offset: usize, byte_len: usize) -> Option<Self> {
        let end = offset.checked_add(byte_len)?;
        if end > data.len() || offset >= end || data[offset] != POSTINGS_SKIP {
            return None;
        }
        let mut pos = offset + 1;
        let doc_count = decode_u32(data, &mut pos) as usize;
        let block_count = decode_u32(data, &mut pos) as usize;
        let metadata_start = pos;
        let payload_start = metadata_start.checked_add(block_count.checked_mul(SKIP_META_SIZE)?)?;
        if payload_start > end {
            return None;
        }
        Some(Self {
            doc_count,
            block_count,
            metadata_start,
            payload_start,
            end,
        })
    }
}

fn block_meta_from(data: &[u8], header: SkipHeader, idx: usize) -> BlockMeta {
    let base = header.metadata_start + idx * SKIP_META_SIZE;
    let first_doc =
        u32::from_le_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
    let last_doc = u32::from_le_bytes([
        data[base + 4],
        data[base + 5],
        data[base + 6],
        data[base + 7],
    ]);
    let payload_offset = u32::from_le_bytes([
        data[base + 8],
        data[base + 9],
        data[base + 10],
        data[base + 11],
    ]);
    let entry_count = u16::from_le_bytes([data[base + 12], data[base + 13]]);
    let next_mask = data[base + 14];
    let loc_mask = data[base + 15];
    let zone_mask = u32::from_le_bytes([
        data[base + 16],
        data[base + 17],
        data[base + 18],
        data[base + 19],
    ]);
    BlockMeta {
        first_doc,
        last_doc,
        payload_offset,
        entry_count,
        next_mask,
        loc_mask,
        zone_mask,
    }
}

fn decode_block_payload(
    data: &[u8],
    pos: &mut usize,
    end: usize,
    entry_count: usize,
) -> Vec<PostingEntry> {
    let mut result = Vec::with_capacity(entry_count);
    let mut prev = 0;
    for _ in 0..entry_count {
        if *pos >= end {
            break;
        }
        let delta = decode_u32(data, pos);
        prev += delta;
        if *pos + 2 > end {
            break;
        }
        let next_mask = data[*pos];
        let loc_mask = data[*pos + 1];
        *pos += 2;
        let zone_mask = if *pos + 4 <= end {
            let mask =
                u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            mask
        } else {
            0
        };
        result.push(PostingEntry {
            doc_id: prev,
            next_mask,
            loc_mask,
            zone_mask,
        });
    }
    result
}

pub struct PostingEntrySkipper<'a> {
    data: &'a [u8],
    header: Option<SkipHeader>,
    current_block_idx: Option<usize>,
    current_block: Vec<PostingEntry>,
    visited_blocks: usize,
    decoded_entries: usize,
}

impl<'a> PostingEntrySkipper<'a> {
    pub fn new(data: &'a [u8], offset: usize, byte_len: usize) -> Self {
        Self {
            data,
            header: SkipHeader::parse(data, offset, byte_len),
            current_block_idx: None,
            current_block: Vec::new(),
            visited_blocks: 0,
            decoded_entries: 0,
        }
    }

    #[allow(dead_code)]
    pub fn entry_count(&self) -> usize {
        self.header.map(|header| header.doc_count).unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn advance_to(&mut self, doc_id: DocId) -> Option<PostingEntry> {
        let header = self.header?;
        let block_idx = self.find_block(doc_id)?;
        self.load_block(header, block_idx);
        self.current_block
            .iter()
            .copied()
            .find(|entry| entry.doc_id >= doc_id)
    }

    pub fn advance_to_masked(
        &mut self,
        doc_id: DocId,
        next_mask: u8,
        loc_mask: u8,
        zone_mask: u32,
    ) -> (Option<PostingEntry>, bool) {
        let Some(header) = self.header else {
            return (None, false);
        };
        let Some(block_idx) = self.find_block(doc_id) else {
            return (None, false);
        };
        if !self.block_may_match(block_idx, next_mask, loc_mask, zone_mask) {
            return (None, true);
        }
        self.load_block(header, block_idx);
        (
            self.current_block
                .iter()
                .copied()
                .find(|entry| entry.doc_id >= doc_id),
            false,
        )
    }

    pub fn visited_blocks(&self) -> usize {
        self.visited_blocks
    }

    pub fn decoded_entries(&self) -> usize {
        self.decoded_entries
    }

    pub fn block_may_match(
        &self,
        block_idx: usize,
        next_mask: u8,
        loc_mask: u8,
        zone_mask: u32,
    ) -> bool {
        let Some(header) = self.header else {
            return true;
        };
        if block_idx >= header.block_count {
            return false;
        }
        let meta = block_meta_from(self.data, header, block_idx);
        (next_mask == 0 || meta.next_mask & next_mask != 0)
            && (loc_mask == 0 || meta.loc_mask & loc_mask != 0)
            && (zone_mask == 0 || meta.zone_mask & zone_mask != 0)
    }

    fn find_block(&self, doc_id: DocId) -> Option<usize> {
        let header = self.header?;
        let mut lo = self.current_block_idx.unwrap_or(0);
        let mut hi = header.block_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let meta = block_meta_from(self.data, header, mid);
            if meta.last_doc < doc_id {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo >= header.block_count {
            return None;
        }
        let meta = block_meta_from(self.data, header, lo);
        if doc_id <= meta.last_doc || doc_id < meta.first_doc {
            Some(lo)
        } else {
            None
        }
    }

    fn load_block(&mut self, header: SkipHeader, block_idx: usize) {
        if self.current_block_idx == Some(block_idx) {
            return;
        }
        let meta = block_meta_from(self.data, header, block_idx);
        let mut pos = header.payload_start + meta.payload_offset as usize;
        self.current_block =
            decode_block_payload(self.data, &mut pos, header.end, meta.entry_count as usize);
        self.current_block_idx = Some(block_idx);
        self.visited_blocks += 1;
        self.decoded_entries += self.current_block.len();
    }
}

// ---------------------------------------------------------------------------
// Lazy PostingIterator — streaming decode without allocation
// ---------------------------------------------------------------------------

/// Lazy iterator over a delta+VByte encoded posting list.
/// Reads directly from a byte slice (typically mmap'd) without heap allocation.
#[allow(dead_code)]
pub struct PostingIterator<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
    remaining: u32,
    count: u32,
    prev: DocId,
    val: Option<DocId>,
}

#[allow(dead_code)]
impl<'a> PostingIterator<'a> {
    /// Create a new iterator over a VByte-encoded posting list at `data[offset..offset+byte_len]`.
    pub fn new(data: &'a [u8], offset: usize, byte_len: usize) -> Self {
        let end = offset + byte_len;
        let mut pos = offset;
        let count = if byte_len > 0 && pos < end {
            decode_u32(data, &mut pos)
        } else {
            0
        };
        let mut iter = Self {
            data,
            pos,
            end,
            remaining: count,
            count,
            prev: 0,
            val: None,
        };
        iter.read_next(); // pre-load first value
        iter
    }

    #[inline]
    fn read_next(&mut self) {
        if self.remaining == 0 || self.pos >= self.end {
            self.val = None;
            return;
        }
        self.remaining -= 1;
        let delta = decode_u32(self.data, &mut self.pos);
        self.prev += delta;
        self.val = Some(self.prev);
    }

    /// Peek at the current value without advancing.
    #[inline]
    pub fn current(&self) -> Option<DocId> {
        self.val
    }

    /// Advance to the next value.
    #[inline]
    pub fn advance(&mut self) {
        self.read_next();
    }

    /// Advance until `current() >= target`. Returns the new current value.
    #[inline]
    pub fn advance_to(&mut self, target: DocId) -> Option<DocId> {
        while let Some(v) = self.val {
            if v >= target {
                return Some(v);
            }
            self.read_next();
        }
        None
    }

    /// Total number of entries in this posting list (for sorting by size).
    #[inline]
    pub fn doc_count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_u32() {
        let cases: &[u32] = &[0, 1, 127, 128, 255, 256, 16383, 16384, 2_097_151, u32::MAX];
        for &val in cases {
            let mut buf = Vec::new();
            encode_u32(val, &mut buf);
            let mut pos = 0;
            let decoded = decode_u32(&buf, &mut pos);
            assert_eq!(decoded, val, "failed for {val}");
            assert_eq!(pos, buf.len(), "didn't consume all bytes for {val}");
        }
    }

    #[test]
    fn test_vbyte_sizes() {
        // 0-127 should encode to 1 byte
        let mut buf = Vec::new();
        encode_u32(0, &mut buf);
        assert_eq!(buf.len(), 1);

        buf.clear();
        encode_u32(127, &mut buf);
        assert_eq!(buf.len(), 1);

        // 128-16383 should encode to 2 bytes
        buf.clear();
        encode_u32(128, &mut buf);
        assert_eq!(buf.len(), 2);

        buf.clear();
        encode_u32(16383, &mut buf);
        assert_eq!(buf.len(), 2);

        // 16384+ should encode to 3+ bytes
        buf.clear();
        encode_u32(16384, &mut buf);
        assert_eq!(buf.len(), 3);

        // u32::MAX should encode to 5 bytes
        buf.clear();
        encode_u32(u32::MAX, &mut buf);
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn test_posting_list_roundtrip() {
        let ids: Vec<DocId> = vec![3, 7, 15, 42, 100, 1000, 50000];
        let encoded = encode_posting_list(&ids);
        let decoded = decode_posting_list(&encoded, 0, encoded.len());
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_posting_list_empty() {
        let ids: Vec<DocId> = vec![];
        let encoded = encode_posting_list(&ids);
        let decoded = decode_posting_list(&encoded, 0, encoded.len());
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_posting_list_single() {
        let ids: Vec<DocId> = vec![42];
        let encoded = encode_posting_list(&ids);
        let decoded = decode_posting_list(&encoded, 0, encoded.len());
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_posting_list_consecutive() {
        let ids: Vec<DocId> = (0..100).collect();
        let encoded = encode_posting_list(&ids);
        let decoded = decode_posting_list(&encoded, 0, encoded.len());
        assert_eq!(decoded, ids);
        // Consecutive IDs should compress very well: 1 byte per delta
        // count (1 byte) + delta_0=0 (1 byte) + 99 deltas of 1 (1 byte each) = ~101 bytes
        // vs raw: 400 bytes
        assert!(encoded.len() < 110, "encoded too large: {}", encoded.len());
    }

    #[test]
    fn test_posting_list_large_gaps() {
        let ids: Vec<DocId> = vec![0, 100_000, 200_000, u32::MAX - 1];
        let encoded = encode_posting_list(&ids);
        let decoded = decode_posting_list(&encoded, 0, encoded.len());
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_posting_list_with_offset() {
        let ids1: Vec<DocId> = vec![1, 2, 3];
        let ids2: Vec<DocId> = vec![10, 20, 30];
        let enc1 = encode_posting_list(&ids1);
        let enc2 = encode_posting_list(&ids2);

        // Concatenate and decode at offset
        let mut combined = enc1.clone();
        let offset2 = combined.len();
        combined.extend_from_slice(&enc2);

        let decoded1 = decode_posting_list(&combined, 0, enc1.len());
        let decoded2 = decode_posting_list(&combined, offset2, enc2.len());
        assert_eq!(decoded1, ids1);
        assert_eq!(decoded2, ids2);
    }

    #[test]
    fn test_posting_entries_roundtrip() {
        let entries = vec![
            PostingEntry {
                doc_id: 3,
                next_mask: 0b0000_0010,
                loc_mask: 0b0000_1000,
                zone_mask: 0b0001,
            },
            PostingEntry {
                doc_id: 7,
                next_mask: 0b0001_0000,
                loc_mask: 0b0100_0000,
                zone_mask: 0b0010,
            },
            PostingEntry {
                doc_id: 1000,
                next_mask: 0,
                loc_mask: 0b0000_0001,
                zone_mask: 0b0100,
            },
        ];
        let encoded = encode_posting_entries(&entries);
        let decoded = decode_posting_entries(&encoded, 0, encoded.len());
        assert_eq!(decoded, entries);
    }

    #[test]
    fn test_posting_entries_simple_threshold_roundtrip() {
        let entries: Vec<PostingEntry> = (0..255)
            .map(|i| PostingEntry {
                doc_id: i * 2,
                next_mask: (i % 8) as u8,
                loc_mask: 1 << (i % 8),
                zone_mask: 1u32 << (i % 31),
            })
            .collect();
        let encoded = encode_posting_entries(&entries);
        assert_eq!(encoded[0], POSTINGS_SIMPLE);
        assert_eq!(
            posting_entry_count(&encoded, 0, encoded.len()),
            entries.len()
        );
        assert!(!is_skip_encoded(&encoded, 0, encoded.len()));
        assert_eq!(decode_posting_entries(&encoded, 0, encoded.len()), entries);
    }

    #[test]
    fn test_posting_entries_skip_threshold_roundtrip() {
        let entries: Vec<PostingEntry> = (0..256)
            .map(|i| PostingEntry {
                doc_id: i * 3,
                next_mask: 1 << (i % 8),
                loc_mask: 1 << ((i + 3) % 8),
                zone_mask: 1u32 << (i % 31),
            })
            .collect();
        let encoded = encode_posting_entries(&entries);
        assert_eq!(encoded[0], POSTINGS_SKIP);
        assert_eq!(
            posting_entry_count(&encoded, 0, encoded.len()),
            entries.len()
        );
        assert!(is_skip_encoded(&encoded, 0, encoded.len()));
        assert_eq!(decode_posting_entries(&encoded, 0, encoded.len()), entries);
    }

    #[test]
    fn test_posting_entry_skipper_advance_to() {
        let entries: Vec<PostingEntry> = (0..300)
            .map(|i| PostingEntry {
                doc_id: i * 5,
                next_mask: 1 << (i % 8),
                loc_mask: 1 << ((i + 1) % 8),
                zone_mask: 1u32 << (i % 31),
            })
            .collect();
        let encoded = encode_posting_entries(&entries);
        let mut skipper = PostingEntrySkipper::new(&encoded, 0, encoded.len());

        assert_eq!(skipper.entry_count(), entries.len());
        assert_eq!(skipper.advance_to(0), Some(entries[0]));
        assert_eq!(skipper.advance_to(7), Some(entries[2]));
        assert_eq!(skipper.visited_blocks(), 1);
        assert!(skipper.decoded_entries() < entries.len());
        assert_eq!(skipper.advance_to(640), Some(entries[128]));
        assert_eq!(skipper.advance_to(1495), Some(entries[299]));
        assert_eq!(skipper.advance_to(1500), None);
        assert_eq!(skipper.visited_blocks(), 3);
    }

    // -- PostingIterator tests --

    #[test]
    fn test_posting_iter_basic() {
        let ids: Vec<DocId> = vec![3, 7, 15, 42, 100];
        let encoded = encode_posting_list(&ids);
        let mut iter = PostingIterator::new(&encoded, 0, encoded.len());
        assert_eq!(iter.doc_count(), 5);
        let mut result = Vec::new();
        while let Some(v) = iter.current() {
            result.push(v);
            iter.advance();
        }
        assert_eq!(result, ids);
    }

    #[test]
    fn test_posting_iter_advance_to() {
        let ids: Vec<DocId> = vec![3, 7, 15, 42, 100, 200, 500];
        let encoded = encode_posting_list(&ids);
        let mut iter = PostingIterator::new(&encoded, 0, encoded.len());

        assert_eq!(iter.advance_to(10), Some(15));
        assert_eq!(iter.current(), Some(15));
        assert_eq!(iter.advance_to(42), Some(42));
        assert_eq!(iter.advance_to(150), Some(200));
        assert_eq!(iter.advance_to(1000), None);
    }

    #[test]
    fn test_posting_iter_advance_to_exact() {
        let ids: Vec<DocId> = vec![5, 10, 15, 20];
        let encoded = encode_posting_list(&ids);
        let mut iter = PostingIterator::new(&encoded, 0, encoded.len());

        assert_eq!(iter.advance_to(5), Some(5)); // exact match at start
        assert_eq!(iter.advance_to(5), Some(5)); // already there
        iter.advance();
        assert_eq!(iter.advance_to(10), Some(10));
    }

    #[test]
    fn test_posting_iter_empty() {
        let ids: Vec<DocId> = vec![];
        let encoded = encode_posting_list(&ids);
        let iter = PostingIterator::new(&encoded, 0, encoded.len());
        assert_eq!(iter.doc_count(), 0);
        assert_eq!(iter.current(), None);
    }

    #[test]
    fn test_posting_iter_single() {
        let ids: Vec<DocId> = vec![42];
        let encoded = encode_posting_list(&ids);
        let mut iter = PostingIterator::new(&encoded, 0, encoded.len());
        assert_eq!(iter.doc_count(), 1);
        assert_eq!(iter.current(), Some(42));
        iter.advance();
        assert_eq!(iter.current(), None);
    }
}
