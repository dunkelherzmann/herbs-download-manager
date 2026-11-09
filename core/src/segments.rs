//! The byte ranges of one download and which connection works on each (SPECIFICATION.md §8).
//!
//! A download starts as one segment over the whole file. A free connection first takes a segment nobody owns;
//! otherwise it takes the second half of the largest remaining range, which the owner of that range then stops
//! short of. No range is split into halves smaller than `min_split`.

/// `[start, end)` of the file; bytes `start..pos` are on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
    /// The connection working on it (never saved).
    #[serde(skip)]
    pub owner: Option<usize>,
}

impl Segment {
    pub fn remaining(&self) -> u64 {
        self.end - self.pos
    }
}

#[derive(Clone, Debug)]
pub struct Segments {
    list: Vec<Segment>,
    min_split: u64,
}

impl Segments {
    pub fn new(size: u64, min_split: u64) -> Self {
        Segments { list: vec![Segment { start: 0, end: size, pos: 0, owner: None }], min_split }
    }

    /// Rebuilds the table from saved segments (all unowned).
    pub fn from_saved(list: Vec<Segment>, min_split: u64) -> Self {
        let list = list.into_iter().map(|s| Segment { owner: None, ..s }).collect();
        Segments { list, min_split }
    }

    /// Whether saved segments cover `0..size` exactly, in order, with each position inside its range.
    pub fn valid_for(list: &[Segment], size: u64) -> bool {
        let mut next = 0;
        for s in list {
            if s.start != next || s.start > s.pos || s.pos > s.end {
                return false;
            }
            next = s.end;
        }
        next == size && !list.is_empty()
    }

    pub fn get(&self, idx: usize) -> Segment {
        self.list[idx]
    }

    pub fn all(&self) -> &[Segment] {
        &self.list
    }

    /// Gives connection `conn` a range to work on and returns its index, or `None` when nothing is left to give.
    pub fn take(&mut self, conn: usize) -> Option<usize> {
        // An unowned segment with bytes left (released by a stalled or failed connection), largest first.
        if let Some((idx, _)) = self
            .list
            .iter()
            .enumerate()
            .filter(|(_, s)| s.owner.is_none() && s.remaining() > 0)
            .max_by_key(|(_, s)| s.remaining())
        {
            self.list[idx].owner = Some(conn);
            return Some(idx);
        }
        // Otherwise the second half of the largest range someone is working on.
        let (idx, seg) = self
            .list
            .iter()
            .enumerate()
            .filter(|(_, s)| s.owner.is_some())
            .max_by_key(|(_, s)| s.remaining())?;
        let half = seg.remaining() / 2;
        if half < self.min_split || seg.remaining() - half < self.min_split {
            return None;
        }
        let mid = seg.pos + seg.remaining() - half;
        let new = Segment { start: mid, end: seg.end, pos: mid, owner: Some(conn) };
        self.list[idx].end = mid;
        self.list.insert(idx + 1, new);
        Some(idx + 1)
    }

    /// Records `n` bytes written at the segment's position.
    pub fn advance(&mut self, idx: usize, n: u64) {
        let s = &mut self.list[idx];
        s.pos = (s.pos + n).min(s.end);
    }

    /// The connection stops working on the segment; its remaining bytes go back to the pool.
    pub fn release(&mut self, idx: usize) {
        self.list[idx].owner = None;
    }

    /// Finds the segment a connection owns (indexes shift when a segment is split).
    pub fn owned_by(&self, conn: usize) -> Option<usize> {
        self.list.iter().position(|s| s.owner == Some(conn) && s.remaining() > 0)
    }

    pub fn downloaded(&self) -> u64 {
        self.list.iter().map(|s| s.pos - s.start).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.list.iter().all(|s| s.pos == s.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const K: u64 = 1024;

    #[test]
    fn splits_the_largest_remaining_range_in_half() {
        let mut s = Segments::new(4096 * K, 256 * K);
        assert_eq!(s.take(0), Some(0));
        s.advance(0, 96 * K);
        let b = s.take(1).unwrap();
        // Remaining 4000 KiB: the new connection starts at the midpoint of what's left.
        assert_eq!(s.get(0).end, 96 * K + 2000 * K);
        assert_eq!(s.get(b).start, 96 * K + 2000 * K);
        assert_eq!(s.get(b).end, 4096 * K);
        let c = s.take(2).unwrap();
        assert_eq!(s.get(c).end - s.get(c).start, 1000 * K);
        assert_eq!(s.all().iter().map(|x| x.end - x.start).sum::<u64>(), 4096 * K);
    }

    #[test]
    fn never_splits_below_the_minimum() {
        let mut s = Segments::new(600 * K, 256 * K);
        s.take(0);
        assert!(s.take(1).is_some()); // 300 + 300
        assert!(s.take(2).is_none()); // halves of 300 KiB would be 150 KiB
        let mut tiny = Segments::new(100 * K, 256 * K);
        tiny.take(0);
        assert!(tiny.take(1).is_none());
    }

    #[test]
    fn released_ranges_are_taken_before_splitting() {
        let mut s = Segments::new(2048 * K, 256 * K);
        s.take(0);
        let b = s.take(1).unwrap();
        s.release(b);
        assert_eq!(s.take(2), Some(b));
        assert_eq!(s.owned_by(2), Some(b));
    }

    #[test]
    fn checks_saved_segments() {
        let a = Segment { start: 0, end: 10, pos: 4, owner: None };
        let b = Segment { start: 10, end: 20, pos: 20, owner: None };
        assert!(Segments::valid_for(&[a, b], 20));
        assert!(!Segments::valid_for(&[a, b], 30));
        assert!(!Segments::valid_for(&[b, a], 20));
        assert!(!Segments::valid_for(&[Segment { pos: 11, ..a }, b], 20));
        assert!(!Segments::valid_for(&[], 0));
    }

    #[test]
    fn completes_when_every_byte_is_in() {
        let mut s = Segments::new(1024 * K, 256 * K);
        s.take(0);
        let b = s.take(1).unwrap();
        s.advance(0, 512 * K);
        assert!(!s.is_complete());
        s.advance(b, 10_000 * K);
        assert!(s.is_complete());
        assert_eq!(s.downloaded(), 1024 * K);
    }
}
