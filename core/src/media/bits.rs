//! Reading the bit fields of H.264 and H.265 parameter sets, and writing MP4 boxes.

/// Reads bits, most significant first, and Exp-Golomb numbers.
pub struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0 }
    }

    pub fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let b = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(b as u32)
    }

    pub fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    pub fn skip(&mut self, n: usize) -> Option<()> {
        self.pos += n;
        (self.pos <= self.data.len() * 8).then_some(())
    }

    /// ue(v).
    pub fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some(((1u64 << zeros) - 1 + self.bits(zeros)? as u64) as u32)
    }

    /// se(v).
    pub fn se(&mut self) -> Option<i32> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 { ((k + 1) / 2) as i32 } else { -(k / 2) as i32 })
    }
}

/// A NAL unit's payload without its emulation prevention bytes (`00 00 03` → `00 00`).
pub fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// Annex B (start codes) → the NAL units.
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (n, &s) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map(|&next| next - 3).unwrap_or(data.len());
        // A four-byte start code's leading zero, and trailing zeros, belong to no unit.
        while end > s && data[end - 1] == 0 {
            end -= 1;
        }
        if end > s {
            out.push(&data[s..end]);
        }
    }
    out
}

/// Builds MP4 boxes: `begin` writes a size placeholder that `end` fills in.
#[derive(Default)]
pub struct Boxes {
    pub out: Vec<u8>,
    open: Vec<usize>,
}

impl Boxes {
    pub fn begin(&mut self, kind: &[u8; 4]) -> &mut Self {
        self.open.push(self.out.len());
        self.out.extend([0, 0, 0, 0]);
        self.out.extend(kind);
        self
    }

    /// A full box: version and flags after the type.
    pub fn begin_full(&mut self, kind: &[u8; 4], version: u8, flags: u32) -> &mut Self {
        self.begin(kind);
        self.u8(version);
        self.out.extend(&flags.to_be_bytes()[1..]);
        self
    }

    pub fn end(&mut self) -> &mut Self {
        let start = self.open.pop().expect("a box was begun");
        let size = (self.out.len() - start) as u32;
        self.out[start..start + 4].copy_from_slice(&size.to_be_bytes());
        self
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.out.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.out.extend(v.to_be_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.out.extend(v.to_be_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.out.extend(v.to_be_bytes());
        self
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.out.extend(v);
        self
    }
    pub fn zeros(&mut self, n: usize) -> &mut Self {
        self.out.extend(std::iter::repeat_n(0, n));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb_and_escapes() {
        // 1 → 0, 010 → 1, 011 → 2, 00100 → 3; se: 010 → +1, 011 → -1.
        let mut b = Bits::new(&[0b1010_0110, 0b0100_0000]);
        assert_eq!((b.ue(), b.ue(), b.ue(), b.ue()), (Some(0), Some(1), Some(2), Some(3)));
        let mut s = Bits::new(&[0b0100_1100]);
        assert_eq!((s.se(), s.se()), (Some(1), Some(-1)));
        assert_eq!(unescape(&[1, 0, 0, 3, 1, 0, 0, 3]), [1, 0, 0, 1, 0, 0]);
        let units = nal_units(&[0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3]);
        assert_eq!(units, [&[9, 0xf0][..], &[0x67, 1, 2], &[0x68, 3]]);
        let mut boxes = Boxes::default();
        boxes.begin(b"free").u32(7).end();
        assert_eq!(boxes.out, [0, 0, 0, 12, b'f', b'r', b'e', b'e', 0, 0, 0, 7]);
    }
}
