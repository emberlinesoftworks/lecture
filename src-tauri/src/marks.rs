use serde::{Deserialize, Serialize};

use crate::transcriber::Segment;

// people press mark right after hearing the important part, not before,
// so a mark reaches back to where that thought started:
// a moment before the press, then back to the last pause (up to 20 seconds)
const REACTION_MS: i64 = 2000;
const PAUSE_MS: i64 = 900;
const MAX_LOOKBACK_MS: i64 = 20_000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MarkedRange {
    pub first_line: usize,
    pub last_line: usize,
    pub mark_ms: i64,
}

pub fn marked_ranges(lines: &[Segment], marks: &[u64]) -> Vec<MarkedRange> {
    marks
        .iter()
        .filter_map(|&mark| marked_range(lines, mark as i64))
        .collect()
}

fn marked_range(lines: &[Segment], mark_ms: i64) -> Option<MarkedRange> {
    if lines.is_empty() {
        return None;
    }

    // the line that was being said a moment before the press
    // (pressed in the first couple seconds, just use the first line)
    let last_line = lines
        .iter()
        .rposition(|line| line.start_ms <= mark_ms - REACTION_MS)
        .unwrap_or(0);

    let mut first_line = last_line;
    while first_line > 0 {
        // a pause before this line means the thought started here
        if lines[first_line].pause_before_ms >= PAUSE_MS {
            break;
        }
        if mark_ms - lines[first_line - 1].start_ms > MAX_LOOKBACK_MS {
            break;
        }
        first_line -= 1;
    }

    Some(MarkedRange { first_line, last_line, mark_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start_ms: i64, pause_before_ms: i64) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 3000,
            text: String::new(),
            pause_before_ms,
        }
    }

    #[test]
    fn mark_reaches_back_to_the_last_pause() {
        let lines = [line(0, 0), line(4000, 0), line(8000, 1500), line(12000, 300), line(16000, 300)];
        // pressed just after the line at 16s finished
        let range = marked_range(&lines, 19500).unwrap();
        assert_eq!((range.first_line, range.last_line), (2, 4));
    }

    #[test]
    fn mark_never_reaches_back_more_than_20_seconds() {
        let lines: Vec<Segment> = (0..10).map(|i| line(i * 4000, 0)).collect();
        let range = marked_range(&lines, 38000).unwrap();
        assert!(38000 - lines[range.first_line].start_ms <= MAX_LOOKBACK_MS);
    }

    #[test]
    fn early_mark_uses_the_first_line() {
        let lines = [line(0, 0), line(4000, 0)];
        assert_eq!(marked_range(&lines, 500).unwrap().first_line, 0);
    }
}
