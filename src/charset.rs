use crate::config::Alphabet;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct Interval {
    pub start: u32,
    pub end: u32,
}

impl Interval {
    pub fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }

    pub fn contains(self, value: u32) -> bool {
        (self.start..=self.end).contains(&value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct CharSet {
    intervals: Vec<Interval>,
}

impl CharSet {
    pub fn empty() -> Self {
        Self {
            intervals: Vec::new(),
        }
    }

    pub fn singleton(ch: char) -> Self {
        let value = ch as u32;
        Self {
            intervals: vec![Interval::new(value, value)],
        }
    }

    pub fn from_interval(start: char, end: char) -> Self {
        Self::from_u32_intervals(vec![Interval::new(start as u32, end as u32)])
    }

    pub fn from_u32_intervals(mut intervals: Vec<Interval>) -> Self {
        intervals.sort_by_key(|interval| (interval.start, interval.end));
        let mut merged: Vec<Interval> = Vec::with_capacity(intervals.len());
        for interval in intervals {
            if let Some(last) = merged.last_mut() {
                if interval.start <= last.end.saturating_add(1) {
                    last.end = last.end.max(interval.end);
                    continue;
                }
            }
            merged.push(interval);
        }
        Self { intervals: merged }
    }

    pub fn ascii_digits() -> Self {
        Self::from_interval('0', '9')
    }

    pub fn ascii_word() -> Self {
        Self::from_u32_intervals(vec![
            Interval::new('0' as u32, '9' as u32),
            Interval::new('A' as u32, 'Z' as u32),
            Interval::new('_' as u32, '_' as u32),
            Interval::new('a' as u32, 'z' as u32),
        ])
    }

    pub fn ascii_space() -> Self {
        Self::from_u32_intervals(vec![
            Interval::new('\t' as u32, '\r' as u32),
            Interval::new(' ' as u32, ' ' as u32),
        ])
    }

    pub fn any(alphabet: Alphabet, dot_matches_newline: bool) -> Self {
        let mut result = Self::from_u32_intervals(alphabet.scalar_intervals().to_vec());
        if !dot_matches_newline {
            result = result.subtract(&Self::singleton('\n'));
        }
        result
    }

    pub fn intervals(&self) -> &[Interval] {
        &self.intervals
    }

    pub fn contains(&self, ch: char) -> bool {
        let value = ch as u32;
        let insertion = self
            .intervals
            .partition_point(|interval| interval.start <= value);
        insertion > 0 && self.intervals[insertion - 1].end >= value
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut merged = Vec::with_capacity(self.intervals.len() + other.intervals.len());
        let mut left = 0;
        let mut right = 0;
        while left < self.intervals.len() || right < other.intervals.len() {
            let take_left = right == other.intervals.len()
                || (left < self.intervals.len()
                    && (self.intervals[left].start, self.intervals[left].end)
                        <= (other.intervals[right].start, other.intervals[right].end));
            let interval = if take_left {
                let interval = self.intervals[left];
                left += 1;
                interval
            } else {
                let interval = other.intervals[right];
                right += 1;
                interval
            };
            Self::push_merged(&mut merged, interval);
        }
        Self { intervals: merged }
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let mut result = Vec::new();
        let mut left = 0;
        let mut right = 0;
        while left < self.intervals.len() && right < other.intervals.len() {
            let a = self.intervals[left];
            let b = other.intervals[right];
            let start = a.start.max(b.start);
            let end = a.end.min(b.end);
            if start <= end {
                result.push(Interval::new(start, end));
            }
            if a.end < b.end {
                left += 1;
            } else {
                right += 1;
            }
        }
        Self { intervals: result }
    }

    pub fn subtract(&self, other: &Self) -> Self {
        let mut result = Vec::new();
        for source in &self.intervals {
            let mut cursor = source.start;
            for removed in &other.intervals {
                if removed.end < cursor || removed.start > source.end {
                    continue;
                }
                if removed.start > cursor {
                    result.push(Interval::new(cursor, removed.start - 1));
                }
                cursor = cursor.max(removed.end.saturating_add(1));
                if cursor > source.end {
                    break;
                }
            }
            if cursor <= source.end {
                result.push(Interval::new(cursor, source.end));
            }
        }
        Self { intervals: result }
    }

    pub fn complement(&self, alphabet: Alphabet) -> Self {
        let universe = Self::from_u32_intervals(alphabet.scalar_intervals().to_vec());
        universe.subtract(self)
    }

    pub fn clipped(&self, alphabet: Alphabet) -> Self {
        self.intersect(&Self::from_u32_intervals(
            alphabet.scalar_intervals().to_vec(),
        ))
    }

    fn push_merged(intervals: &mut Vec<Interval>, interval: Interval) {
        if let Some(last) = intervals.last_mut() {
            if interval.start <= last.end.saturating_add(1) {
                last.end = last.end.max(interval.end);
                return;
            }
        }
        intervals.push(interval);
    }

    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_adjacent_and_overlapping_intervals() {
        let set = CharSet::from_u32_intervals(vec![
            Interval::new(10, 12),
            Interval::new(13, 14),
            Interval::new(5, 8),
            Interval::new(7, 11),
        ]);
        assert_eq!(set.intervals(), &[Interval::new(5, 14)]);
    }

    #[test]
    fn union_normalizes_both_inputs() {
        let left = CharSet::from_interval('a', 'c');
        let right = CharSet::from_interval('d', 'f');
        assert_eq!(
            left.union(&right).intervals(),
            &[Interval::new('a' as u32, 'f' as u32)]
        );
    }

    #[test]
    fn intersection_keeps_only_shared_ranges() {
        let left = CharSet::from_u32_intervals(vec![Interval::new(0, 5), Interval::new(10, 20)]);
        let right = CharSet::from_u32_intervals(vec![Interval::new(3, 12), Interval::new(18, 30)]);
        assert_eq!(
            left.intersect(&right).intervals(),
            &[
                Interval::new(3, 5),
                Interval::new(10, 12),
                Interval::new(18, 20),
            ]
        );
    }

    #[test]
    fn subtraction_can_split_an_interval() {
        let source = CharSet::from_u32_intervals(vec![Interval::new(0, 10)]);
        let removed = CharSet::from_u32_intervals(vec![Interval::new(3, 7)]);
        assert_eq!(
            source.subtract(&removed).intervals(),
            &[Interval::new(0, 2), Interval::new(8, 10)]
        );
    }

    #[test]
    fn subtracting_disjoint_set_is_identity() {
        let source = CharSet::from_interval('a', 'z');
        let removed = CharSet::ascii_digits();
        assert_eq!(source.subtract(&removed), source);
    }

    #[test]
    fn complement_respects_ascii_domain() {
        let complement = CharSet::ascii_digits().complement(Alphabet::Ascii);
        assert!(complement.contains('a'));
        assert!(!complement.contains('4'));
        assert!(!complement.contains('é'));
    }

    #[test]
    fn unicode_universe_excludes_surrogates() {
        let universe = CharSet::empty().complement(Alphabet::Unicode);
        assert!(universe.contains('\u{d7ff}'));
        assert!(universe.contains('\u{e000}'));
        assert!(universe.contains('\u{10ffff}'));
        assert_eq!(
            universe.intervals(),
            &[Interval::new(0, 0xd7ff), Interval::new(0xe000, 0x10ffff)]
        );
    }

    #[test]
    fn dot_excludes_only_newline_when_requested() {
        let dot = CharSet::any(Alphabet::Ascii, false);
        assert!(dot.contains('\r'));
        assert!(!dot.contains('\n'));
        assert!(dot.contains('a'));
    }

    #[test]
    fn shorthand_sets_have_documented_ascii_membership() {
        assert!(CharSet::ascii_digits().contains('7'));
        assert!(!CharSet::ascii_digits().contains('x'));
        assert!(CharSet::ascii_word().contains('_'));
        assert!(CharSet::ascii_word().contains('Z'));
        assert!(!CharSet::ascii_word().contains('-'));
        assert!(CharSet::ascii_space().contains('\t'));
        assert!(CharSet::ascii_space().contains(' '));
        assert!(!CharSet::ascii_space().contains('a'));
    }

    #[test]
    fn contains_binary_search_handles_gaps_and_boundaries() {
        let set = CharSet::from_u32_intervals(vec![
            Interval::new('a' as u32, 'c' as u32),
            Interval::new('x' as u32, 'z' as u32),
        ]);
        for member in ['a', 'b', 'c', 'x', 'y', 'z'] {
            assert!(set.contains(member), "{member:?}");
        }
        for nonmember in ['`', 'd', 'w', '{'] {
            assert!(!set.contains(nonmember), "{nonmember:?}");
        }
    }

    #[test]
    fn union_merges_two_normalized_lists_without_resorting() {
        let left = CharSet::from_u32_intervals(vec![Interval::new(1, 2), Interval::new(10, 12)]);
        let right = CharSet::from_u32_intervals(vec![
            Interval::new(3, 4),
            Interval::new(8, 9),
            Interval::new(20, 21),
        ]);
        assert_eq!(
            left.union(&right).intervals(),
            &[
                Interval::new(1, 4),
                Interval::new(8, 12),
                Interval::new(20, 21),
            ]
        );
    }
}
