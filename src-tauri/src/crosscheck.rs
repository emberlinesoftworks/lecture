// the second opinion: the other model listens again to just the bits around unsure words.
// where both heard the same thing the gray underline goes away, and where they
// heard something different, what the other one heard becomes a one-tap option

use std::path::Path;

use crate::transcriber::{load_audio, Segment, Transcriber, Unsure, WHISPER_SAMPLE_RATE};

// lines close together are listened to as one clip, as long as it stays under this
const LONGEST_CLIP_MS: i64 = 25_000;
// a bit of room on each side so the first and last words aren't cut off
const CLIP_PADDING_MS: i64 = 300;
// a suggestion has to share at least this much with what it replaces. a misheard word sounds
// like the real one ("mcloran" / "Maclaurin"), while "Chain" / "Okay, nope" is someone else talking
const LEAST_ALIKE: f32 = 0.4;

// differences longer than this are a mess, not a misheard word, so they don't become options
const LONGEST_SWAP: usize = 4;

// goes through every line with unsure words, returns new unsure lists by line
pub fn second_opinion(other_model: &Path, wav_path: &Path, lines: &[Segment]) -> Result<Vec<Option<Vec<Unsure>>>, String> {
    let mut results = vec![None; lines.len()];
    let groups = clip_groups(lines);
    if groups.is_empty() {
        return Ok(results);
    }

    let audio = load_audio(wav_path)?;
    let transcriber = Transcriber::load(other_model)?;
    for (first, last) in groups {
        let start_ms = (lines[first].start_ms - CLIP_PADDING_MS).max(0);
        let end_ms = lines[last].end_ms + CLIP_PADDING_MS;
        let to_sample = |ms: i64| ((ms * WHISPER_SAMPLE_RATE as i64 / 1000) as usize).min(audio.len());
        let clip = &audio[to_sample(start_ms)..to_sample(end_ms)];
        if clip.is_empty() {
            continue;
        }

        let heard = transcriber.transcribe_audio(clip, start_ms)?;
        // put the other model's lines together, keeping track of the words it wasn't sure about either
        let mut other = String::new();
        let mut other_unsure = Vec::new();
        for line in &heard {
            if !other.is_empty() {
                other.push(' ');
            }
            let offset = other.chars().count();
            other_unsure.extend(line.unsure.iter().map(|word| (offset + word.start, offset + word.end)));
            other.push_str(&line.text);
        }
        for (index, unsure) in compare(&lines[first..=last], &other, &other_unsure) {
            results[first + index] = Some(unsure);
        }
    }
    Ok(results)
}

// which lines to listen to again, as (first, last) line numbers. a line or two in
// between with nothing unsure comes along so the clip is one stretch of talking
fn clip_groups(lines: &[Segment]) -> Vec<(usize, usize)> {
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.unsure.is_empty() || line.hidden {
            continue;
        }
        if let Some((first, last)) = groups.last_mut() {
            if index <= *last + 2 && line.end_ms - lines[*first].start_ms <= LONGEST_CLIP_MS {
                *last = index;
                continue;
            }
        }
        groups.push((index, index));
    }
    groups
}

// a word in a line: where it sits and what it is without case or punctuation
struct Word {
    line: usize,
    start: usize,
    end: usize,
    bare: String,
}

fn words_of(text: &str, line: usize) -> Vec<Word> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        // the spot leaves out punctuation on the ends, so swapping a word keeps its comma
        let mut start = i;
        let mut stop = end;
        while start < stop && !chars[start].is_alphanumeric() {
            start += 1;
        }
        while stop > start && !chars[stop - 1].is_alphanumeric() {
            stop -= 1;
        }
        let bare: String = chars[start..stop].iter().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect();
        // whisper's notes like [BLANK_AUDIO] or (laughs) aren't words anyone said
        let note = matches!(chars[i], '[' | '(') || matches!(chars[end - 1], ']' | ')');
        if !bare.is_empty() && !note {
            words.push(Word { line, start, end: stop, bare: as_digits(&bare) });
        }
        i = end;
    }
    words
}

// how alike two words are, 0 to 1, by how many letters you'd change to get from one to the other
fn alike(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(x != y)).min(row[j] + 1).min(above + 1);
            diagonal = above;
        }
    }
    1.0 - row[b.len()] as f32 / longest as f32
}

// "six" and "6" are the same thing said
fn as_digits(bare: &str) -> String {
    const NUMBERS: [&str; 21] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
        "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty",
    ];
    match NUMBERS.iter().position(|number| *number == bare) {
        Some(value) => value.to_string(),
        None => bare.to_string(),
    }
}

// lines up the two versions word by word (longest common run of words, like a diff)
// and works out the new unsure list for each line
// other_unsure is where the other model wasn't sure either, its guesses there aren't offered
fn compare(lines: &[Segment], other: &str, other_unsure: &[(usize, usize)]) -> Vec<(usize, Vec<Unsure>)> {
    let ours: Vec<Word> = lines.iter().enumerate().flat_map(|(index, line)| words_of(&line.text, index)).collect();
    let theirs = words_of(other, 0);
    let other_chars: Vec<char> = other.chars().collect();

    // table[i][j]: how many words match from ours[i..] and theirs[j..]
    let mut table = vec![vec![0u16; theirs.len() + 1]; ours.len() + 1];
    for i in (0..ours.len()).rev() {
        for j in (0..theirs.len()).rev() {
            table[i][j] = if ours[i].bare == theirs[j].bare {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    // partner[i] is the word in theirs that matched ours[i], if any
    let mut partner = vec![None; ours.len()];
    let (mut i, mut j) = (0, 0);
    while i < ours.len() && j < theirs.len() {
        if ours[i].bare == theirs[j].bare {
            partner[i] = Some(j);
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    let mut results = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        if line.unsure.is_empty() {
            continue;
        }
        let mut updated: Vec<Unsure> = Vec::new();
        for unsure in &line.unsure {
            let covered: Vec<usize> = (0..ours.len())
                .filter(|&k| ours[k].line == line_index && ours[k].start < unsure.end && ours[k].end > unsure.start)
                .collect();
            let (Some(&first), Some(&last)) = (covered.first(), covered.last()) else { continue };

            // both models heard the same thing, so it's probably right
            if covered.iter().all(|&k| partner[k].is_some()) {
                continue;
            }

            // the stretch that differs: from the matched word before to the matched word after
            let before = (0..first).rev().find(|&k| partner[k].is_some());
            let after = (last + 1..ours.len()).find(|&k| partner[k].is_some());
            let our_from = before.map_or(0, |k| k + 1);
            let our_to = after.unwrap_or(ours.len());
            let their_from = before.and_then(|k| partner[k]).map_or(0, |j| j + 1);
            let their_to = after.and_then(|k| partner[k]).unwrap_or(theirs.len());

            // the other model heard nothing there, or wasn't sure itself:
            // no second opinion, so this isn't worth pointing out
            if their_to <= their_from {
                continue;
            }
            let (their_start, their_end) = (theirs[their_from].start, theirs[their_to - 1].end);
            if other_unsure.iter().any(|&(start, end)| start < their_end && end > their_start) {
                continue;
            }

            let same_line = ours[our_from..our_to].iter().all(|word| word.line == line_index);
            let fits = our_to - our_from <= LONGEST_SWAP && their_to - their_from <= LONGEST_SWAP;
            if !same_line || !fits {
                // still unsure, just no good option to offer
                if !updated.iter().any(|done| done.start < unsure.end && done.end > unsure.start) {
                    updated.push(unsure.clone());
                }
                continue;
            }

            let start = ours[our_from].start;
            let end = ours[our_to - 1].end;
            let option: String = other_chars[their_start..their_end].iter().collect();
            let our_bare: String = ours[our_from..our_to].iter().map(|word| word.bare.as_str()).collect();
            let their_bare: String = theirs[their_from..their_to].iter().map(|word| word.bare.as_str()).collect();
            // "Taylor-Polynova" vs "Taylor Polynova" is the same thing written two ways
            if our_bare == their_bare || alike(&our_bare, &their_bare) < LEAST_ALIKE {
                continue;
            }
            if updated.iter().any(|done| done.start < end && done.end > start) {
                continue;
            }
            updated.push(Unsure { start, end, options: vec![option] });
        }
        updated.sort_by_key(|word| word.start);
        results.push((line_index, updated));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, unsure: &[&str]) -> Segment {
        let mut line = Segment { text: text.to_string(), ..Default::default() };
        line.unsure_words = unsure.iter().map(|word| word.to_string()).collect();
        crate::transcriber::place_unsure(&mut line);
        line
    }

    fn options(text: &str, unsure: &[Unsure]) -> Vec<(String, Vec<String>)> {
        let chars: Vec<char> = text.chars().collect();
        unsure.iter().map(|word| (chars[word.start..word.end].iter().collect(), word.options.clone())).collect()
    }

    #[test]
    fn a_different_word_becomes_an_option() {
        let lines = [line("So the mcloran series, and the Taylor-Pollinger.", &["mcloran", "Taylor-Pollinger"])];
        let results = compare(&lines, "So the Maclaurin series and the Taylor polynomial.", &[]);
        assert_eq!(
            options(&lines[0].text, &results[0].1),
            [
                ("mcloran".into(), vec!["Maclaurin".into()]),
                ("Taylor-Pollinger".into(), vec!["Taylor polynomial".into()]),
            ]
        );
    }

    #[test]
    fn when_both_heard_the_same_the_underline_goes() {
        let lines = [line("We use the ratio test here.", &["ratio"])];
        let results = compare(&lines, "we use the ratio test here", &[]);
        assert!(results[0].1.is_empty());
    }

    #[test]
    fn a_mess_stays_unsure_with_nothing_to_offer() {
        let lines = [line("So basketball front again today folks.", &["basketball"])];
        let results = compare(&lines, "Pascal's triangle shows up in the binomial expansion today folks.", &[]);
        assert_eq!(options(&lines[0].text, &results[0].1), [("basketball".into(), Vec::<String>::new())]);
    }

    #[test]
    fn what_went_wrong_on_a_real_class() {
        // numbers written two ways are the same
        let lines = [line("Three times two gives a six and we leave it.", &["six"])];
        assert!(compare(&lines, "Three times two gives a 6 and we leave it.", &[])[0].1.is_empty());

        // [BLANK_AUDIO] isn't a suggestion, and nothing heard means no underline
        let lines = [line("Cosine.", &["Cosine"])];
        assert!(compare(&lines, "[BLANK_AUDIO]", &[])[0].1.is_empty());

        // someone else talking isn't a suggestion
        let lines = [line("Chain?", &["Chain"])];
        assert!(compare(&lines, "Okay, nope.", &[])[0].1.is_empty());

        // the other model wasn't sure about "change" either, so it isn't offered
        let lines = [line("we applied our chain rule.", &["rule"])];
        let other = "we applied our change.";
        let change = other.find("change").unwrap();
        assert!(compare(&lines, other, &[(change, change + 6)])[0].1.is_empty());
    }

    #[test]
    fn nearby_lines_share_a_clip() {
        let mut lines: Vec<Segment> = (0..6)
            .map(|i| Segment { start_ms: i * 4000, end_ms: i * 4000 + 3500, text: "word".into(), ..Default::default() })
            .collect();
        for index in [0, 2, 5] {
            lines[index].unsure.push(Unsure { start: 0, end: 4, options: Vec::new() });
        }
        assert_eq!(clip_groups(&lines), [(0, 2), (5, 5)]);
    }
}
