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

    /// Crate-private: unlike every other constructor here, this accepts raw
    /// `u32` boundaries with no validation against a valid Unicode scalar
    /// range (`char`'s own type already rules out surrogates and values
    /// past `U+10FFFF` for every *other* constructor, which all take `char`
    /// arguments). Kept internal so external callers can only ever build a
    /// `CharSet` through a `char`-validated path -- constructing one over
    /// e.g. the surrogate range, or values beyond `U+10FFFF`, would produce
    /// a `CharSet` no other code in the crate has to account for, since
    /// `char::from_u32` genuinely can't represent it.
    pub(crate) fn from_u32_intervals(mut intervals: Vec<Interval>) -> Self {
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

    pub fn as_singleton(&self) -> Option<char> {
        match *self.intervals.as_slice() {
            [interval] if interval.start == interval.end => char::from_u32(interval.start),
            _ => None,
        }
    }

    /// Compact Graphviz edge label for this set under `alphabet`.
    ///
    /// Prefers `.`, shorthands (`\d`, `\w`, `\s` and their complements), a
    /// singleton character, or a character class — using a negated class when
    /// that spelling is shorter.
    pub fn to_dot_label(&self, alphabet: Alphabet, dot_matches_newline: bool) -> String {
        if self.is_empty() {
            return "∅".to_owned();
        }
        let any = Self::any(alphabet, dot_matches_newline);
        if self == &any {
            return ".".to_owned();
        }
        if self == &Self::ascii_digits() {
            return "\\d".to_owned();
        }
        if self == &Self::ascii_word() {
            return "\\w".to_owned();
        }
        if self == &Self::ascii_space() {
            return "\\s".to_owned();
        }
        let digits_c = Self::ascii_digits().complement(alphabet);
        let word_c = Self::ascii_word().complement(alphabet);
        let space_c = Self::ascii_space().complement(alphabet);
        if self == &digits_c {
            return "\\D".to_owned();
        }
        if self == &word_c {
            return "\\W".to_owned();
        }
        if self == &space_c {
            return "\\S".to_owned();
        }
        if let Some(ch) = self.as_singleton() {
            return format_dot_char(ch);
        }
        let positive = format!("[{}]", format_class_body(self));
        let complement = any.subtract(self);
        if !complement.is_empty() {
            if let Some(ch) = complement.as_singleton() {
                let negated = format!("[^{}]", format_class_char(ch));
                if negated.len() <= positive.len() {
                    return negated;
                }
            }
            let negated = format!("[^{}]", format_class_body(&complement));
            if negated.len() < positive.len() {
                return negated;
            }
        }
        positive
    }
}

/// Boundary points (sorted, deduplicated) between every interval across
/// `sets`, each paired with a trailing sentinel one past its end
/// (`interval.end + 1`, up to `0x110000` -- one past the last valid
/// Unicode scalar value, `0x10FFFF`).
///
/// Shared by every "partition the alphabet into maximal same-membership
/// ranges" computation in the crate: [`representative_chars`] and
/// [`alphabet_partition`] here, and previously duplicated (independently,
/// by design -- see the history below) as `representative_chars` in
/// `analysis.rs`/`derivative.rs`/`antimirov.rs` and `representative_symbols`
/// in `minimize.rs`.
///
/// Generic over `T: Borrow<CharSet>` so callers that already hold
/// `&[CharSet]` (owned sets, as `derivative.rs`/`antimirov.rs`'s
/// `first_sets` produce) and callers that hold `&[&CharSet]` (references
/// into someone else's longer-lived data, as `minimize.rs`'s DFA
/// transitions are) can both call this directly, without either needing an
/// extra allocation or clone just to match the other's shape.
///
/// The four now-unified copies used to each maintain this exact loop
/// independently -- deliberately, per a comment on the oldest of them,
/// "so a bug in one copy doesn't automatically show up in the other." In
/// practice it went the other way: `alphabet_partition`'s copy omitted the
/// trailing sentinel for an interval already ending at `0x10FFFF` (since,
/// unlike the other three, it recovers each range by pairing *adjacent*
/// boundaries and so needs one even there), silently dropping the entire
/// top range of the Unicode alphabet from any DFA transition set that
/// reached it -- and the sibling copies' tests, being sibling copies, had
/// no way to catch a bug specific to this one.
fn alphabet_boundaries<T: std::borrow::Borrow<CharSet>>(sets: &[T]) -> Vec<u32> {
    let mut boundaries: Vec<u32> = Vec::new();
    for set in sets {
        for interval in set.borrow().intervals() {
            boundaries.push(interval.start);
            boundaries.push(interval.end + 1);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

/// One representative character per maximal alphabet range that behaves
/// identically (as far as membership in any of `sets` goes) across its
/// whole span. See [`alphabet_boundaries`].
pub(crate) fn representative_chars<T: std::borrow::Borrow<CharSet>>(sets: &[T]) -> Vec<char> {
    alphabet_boundaries(sets)
        .into_iter()
        .filter_map(char::from_u32)
        .filter(|ch| sets.iter().any(|set| set.borrow().contains(*ch)))
        .collect()
}

/// Like [`representative_chars`], but keeps the *whole* inclusive range
/// `(start, end, representative)` for each partition class instead of just
/// the representative -- needed when materializing full DFA transitions
/// rather than just picking one character to test per class. See
/// [`alphabet_boundaries`] for why the trailing sentinel matters here in
/// particular: this is the one consumer that pairs up *adjacent* boundaries
/// via `windows(2)` to recover each range, so a boundary after the very
/// last interval is required, not just one at its start.
pub(crate) fn alphabet_partition<T: std::borrow::Borrow<CharSet>>(
    sets: &[T],
) -> Vec<(u32, u32, char)> {
    let boundaries = alphabet_boundaries(sets);
    let mut out = Vec::new();
    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1] - 1;
        if let Some(rep) = char::from_u32(start) {
            if sets.iter().any(|set| set.borrow().contains(rep)) {
                out.push((start, end, rep));
            }
        }
    }
    out
}

fn format_dot_char(ch: char) -> String {
    match ch {
        '\n' => "\\n".to_owned(),
        '\r' => "\\r".to_owned(),
        '\t' => "\\t".to_owned(),
        '\0' => "\\0".to_owned(),
        ' ' => "SP".to_owned(),
        c if c.is_control() => format!("U+{:04X}", c as u32),
        c => c.to_string(),
    }
}

fn format_class_char(ch: char) -> String {
    match ch {
        '\n' => "\\n".to_owned(),
        '\r' => "\\r".to_owned(),
        '\t' => "\\t".to_owned(),
        '\0' => "\\0".to_owned(),
        '\\' => "\\\\".to_owned(),
        ']' => "\\]".to_owned(),
        '^' => "\\^".to_owned(),
        '-' => "\\-".to_owned(),
        ' ' => "SP".to_owned(),
        c if c.is_control() => format!("U+{:04X}", c as u32),
        c => c.to_string(),
    }
}

fn format_class_body(set: &CharSet) -> String {
    let mut out = String::new();
    for interval in set.intervals() {
        let start = match char::from_u32(interval.start) {
            Some(ch) => ch,
            None => continue,
        };
        let end = match char::from_u32(interval.end) {
            Some(ch) => ch,
            None => continue,
        };
        if interval.start == interval.end {
            out.push_str(&format_class_char(start));
        } else if interval.end == interval.start + 1 {
            out.push_str(&format_class_char(start));
            out.push_str(&format_class_char(end));
        } else {
            out.push_str(&format_class_char(start));
            out.push('-');
            out.push_str(&format_class_char(end));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_partition_covers_a_range_ending_at_the_last_scalar_value() {
        // Regression guard: this used to add a boundary *after* an
        // interval's end only when `end < 0x10ffff`, so an interval ending
        // exactly at the top of the Unicode range got no trailing boundary.
        // Since `alphabet_partition` recovers ranges by pairing *adjacent*
        // boundaries with `windows(2)`, that left the interval's own
        // boundary unpaired -- silently dropping the whole top range from
        // the returned partition (and so from the DFA transitions built
        // from it), even though the interval itself was well-formed.
        let set = CharSet::from_u32_intervals(vec![Interval::new(0x10fffe, 0x10ffff)]);
        let partition = alphabet_partition(&[&set]);
        assert_eq!(
            partition,
            vec![(0x10fffe, 0x10ffff, char::from_u32(0x10fffe).unwrap())],
            "range ending at U+10FFFF must survive the partition, not be silently dropped"
        );
    }

    /// `representative_chars` and `alphabet_partition` both accept either
    /// `&[CharSet]` (owned sets) or `&[&CharSet]` (references into someone
    /// else's data) via `Borrow<CharSet>`, so every existing call site could
    /// switch to the shared versions without an extra allocation or clone
    /// just to match a different shape. Exercise both shapes directly so a
    /// future signature change that breaks one of them fails here, not only
    /// via a knock-on compile error at some call site.
    #[test]
    fn accepts_both_owned_and_referenced_charset_slices() {
        let owned: Vec<CharSet> = vec![CharSet::from_interval('a', 'c')];
        let borrowed: Vec<&CharSet> = owned.iter().collect();
        assert_eq!(
            representative_chars(&owned),
            representative_chars(&borrowed)
        );
        assert_eq!(alphabet_partition(&owned), alphabet_partition(&borrowed));
    }

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
