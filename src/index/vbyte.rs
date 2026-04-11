use super::postings::DocId;

/// Encode a u32 as variable-byte (7 bits/byte, MSB=1 = final byte).
#[inline]
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
#[inline]
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

/// Decode a delta + VByte encoded posting list from a byte slice.
///
/// Reads from `data[offset .. offset + byte_len]`.
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

// ---------------------------------------------------------------------------
// Lazy PostingIterator — streaming decode without allocation
// ---------------------------------------------------------------------------

/// Lazy iterator over a delta+VByte encoded posting list.
/// Reads directly from a byte slice (typically mmap'd) without heap allocation.
pub struct PostingIterator<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
    remaining: u32,
    count: u32,
    prev: DocId,
    val: Option<DocId>,
}

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
