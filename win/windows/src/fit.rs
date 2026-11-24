//! Making the bar fit its maximum width, following the GNOME extension's
//! order: drop speeds first, then metrics from the right, keeping the most
//! important ones (commit, memory, CPU) for last.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Name, value and speed.
    Full,
    /// Name and value.
    Compact,
}

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub full: i32,
    pub compact: i32,
    /// Lower is dropped first.
    pub keep_priority: u8,
}

/// Returns the detail level and which segments to show (by index), so that
/// the total plus `gap` between segments fits in `max_width`. At least one
/// segment always stays.
pub fn fit(segments: &[Segment], gap: i32, max_width: i32) -> (Detail, Vec<usize>) {
    let total = |detail: Detail, shown: &[usize]| -> i32 {
        let widths: i32 = shown
            .iter()
            .map(|&i| match detail {
                Detail::Full => segments[i].full,
                Detail::Compact => segments[i].compact,
            })
            .sum();
        widths + gap * (shown.len() as i32 - 1).max(0)
    };
    let mut shown: Vec<usize> = (0..segments.len()).collect();
    if total(Detail::Full, &shown) <= max_width {
        return (Detail::Full, shown);
    }
    while shown.len() > 1 && total(Detail::Compact, &shown) > max_width {
        // Drop the least important; on a tie, the rightmost.
        let (pos, _) = shown
            .iter()
            .enumerate()
            .min_by(|(pa, &a), (pb, &b)| segments[a].keep_priority.cmp(&segments[b].keep_priority).then(pb.cmp(pa)))
            .expect("non-empty");
        shown.remove(pos);
    }
    (Detail::Compact, shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(full: i32, compact: i32, keep_priority: u8) -> Segment {
        Segment { full, compact, keep_priority }
    }

    #[test]
    fn full_when_room() {
        let s = [seg(50, 40, 9), seg(50, 40, 8)];
        assert_eq!(fit(&s, 10, 200), (Detail::Full, vec![0, 1]));
    }

    #[test]
    fn speeds_go_first_then_low_priority() {
        let s = [seg(80, 40, 9), seg(80, 40, 2), seg(80, 40, 5)];
        assert_eq!(fit(&s, 10, 140), (Detail::Compact, vec![0, 1, 2]));
        assert_eq!(fit(&s, 10, 100), (Detail::Compact, vec![0, 2]));
        assert_eq!(fit(&s, 10, 10), (Detail::Compact, vec![0]));
    }
}
