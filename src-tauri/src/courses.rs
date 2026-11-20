// a folder can be a course: a subject and a list of words whisper tends to get wrong.
// kept in a hidden .course.json inside the folder, so renaming the folder keeps it

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::transcriber::{Segment, Unsure, WordFix};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Course {
    // math, physics, biology... picks which built in words it gets (see dictionary.rs)
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub words: Vec<CourseWord>,
    // the exams and their dates, so things said about "the exam" land under the right one
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exams: Vec<Exam>,
    // where you put exam notes yourself, so a wrong guess stays fixed
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exam_choices: Vec<ExamChoice>,
}

// one exam note you sorted by hand. it's found again by the lecture's file and the time it was said,
// not the line number, so it survives transcribing the lecture again
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExamChoice {
    pub file: String,
    pub at_ms: i64,
    // which exam it's about. none means it isn't about an exam at all
    #[serde(default)]
    pub exam: Option<String>,
    // a line you added yourself, that Lecture didn't find on its own
    #[serde(default)]
    pub added: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Exam {
    // like "Test 4" or "Final exam"
    pub name: String,
    // YYYY-MM-DD
    pub date: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CourseWord {
    // how it should be written, like "Maclaurin"
    pub word: String,
    // what whisper writes instead, like "McLaurin" or "Gordon"
    #[serde(default)]
    pub heard_as: Vec<String>,
}

fn course_path(folder: &Path) -> std::path::PathBuf {
    folder.join(".course.json")
}

pub fn load_course(folder: &Path) -> Course {
    fs::read_to_string(course_path(folder))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn save_course(folder: &Path, course: &Course) -> Result<(), String> {
    // tidy it up before saving: no blanks, no repeats
    let mut cleaned = Course {
        subject: course.subject.clone().filter(|s| !s.trim().is_empty()),
        words: Vec::new(),
        exams: Vec::new(),
        exam_choices: course.exam_choices.clone(),
    };
    for exam in &course.exams {
        let name = exam.name.trim();
        let date = exam.date.trim();
        let real_date = date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-';
        if !name.is_empty() && real_date {
            cleaned.exams.push(Exam { name: name.to_string(), date: date.to_string() });
        }
    }
    cleaned.exams.sort_by(|a, b| a.date.cmp(&b.date));
    for entry in &course.words {
        let word = entry.word.trim();
        if word.is_empty() || cleaned.words.iter().any(|w| w.word.eq_ignore_ascii_case(word)) {
            continue;
        }
        let mut heard_as: Vec<String> = Vec::new();
        for heard in &entry.heard_as {
            let heard = heard.trim();
            if !heard.is_empty() && !heard.eq_ignore_ascii_case(word) && !heard_as.iter().any(|h| h.eq_ignore_ascii_case(heard)) {
                heard_as.push(heard.to_string());
            }
        }
        cleaned.words.push(CourseWord { word: word.to_string(), heard_as });
    }

    let json = serde_json::to_string_pretty(&cleaned).map_err(|e| e.to_string())?;
    fs::write(course_path(folder), json).map_err(|e| e.to_string())
}

// adds one fix from the transcript: what it heard and what it should say.
// if the word's already in the course it just gets another spelling, and if that
// spelling was going to some other word before, the newest one wins
pub fn add_word(course: &mut Course, word: &str, heard: &str) -> Result<(), String> {
    let (word, heard) = (word.trim(), heard.trim());
    if word.is_empty() || heard.is_empty() {
        return Err("Type what it should say".into());
    }
    if word == heard {
        return Err("That's already what it says".into());
    }
    for entry in course.words.iter_mut() {
        entry.heard_as.retain(|old| !old.eq_ignore_ascii_case(heard));
    }
    match course.words.iter_mut().find(|entry| entry.word.eq_ignore_ascii_case(word)) {
        Some(entry) => entry.heard_as.push(heard.to_string()),
        None => course.words.push(CourseWord { word: word.to_string(), heard_as: vec![heard.to_string()] }),
    }
    Ok(())
}

// puts the course's words right in every line: what it was heard as becomes the word,
// and a word with capitals in it ("Maclaurin", "Maple") gets its capitals back.
// only whole words are changed, so "Gordon" doesn't touch "Gordons". every real change is
// noted on the line (where it is, what was heard) so the window can underline it
pub fn fix_words(course: &Course, lines: &mut [Segment]) {
    // (wrong, right), longest wrong first so "Gordon series" is tried before "Gordon"
    let mut swaps: Vec<(Vec<char>, &str)> = Vec::new();
    for entry in &course.words {
        for heard in &entry.heard_as {
            swaps.push((heard.chars().collect(), &entry.word));
        }
        if entry.word.chars().any(|c| c.is_uppercase()) {
            swaps.push((entry.word.chars().collect(), &entry.word));
        }
    }
    swaps.sort_by_key(|(wrong, _)| std::cmp::Reverse(wrong.len()));

    if swaps.is_empty() {
        return;
    }
    for line in lines.iter_mut() {
        fix_line(line, &swaps);
    }
}

fn same_letters(a: &[char], b: &[char]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_lowercase().eq(y.to_lowercase()))
}

// goes through the line once, left to right, swapping wrong words as it finds them
fn fix_line(line: &mut Segment, swaps: &[(Vec<char>, &str)]) {
    let chars: Vec<char> = line.text.chars().collect();
    let kept: Vec<Vec<char>> = line.kept.iter().map(|word| word.chars().collect()).collect();
    let mut fixed: Vec<char> = Vec::with_capacity(chars.len());
    // where each character of the old text ended up, to move the fixes it already had
    let mut moved_to = vec![0; chars.len() + 1];
    let mut new_fixes = Vec::new();

    let mut i = 0;
    while i < chars.len() {
        moved_to[i] = fixed.len();
        let starts_word = i == 0 || !chars[i - 1].is_alphanumeric();
        let swap = swaps.iter().find(|(wrong, _)| {
            let end = i + wrong.len();
            starts_word
                && end <= chars.len()
                && same_letters(&chars[i..end], wrong)
                && (end == chars.len() || !chars[end].is_alphanumeric())
        });
        // a word you changed back in this line stays the way it was heard
        let swap = swap.filter(|(wrong, _)| !kept.iter().any(|word| same_letters(word, wrong)));

        let Some((wrong, right)) = swap else {
            fixed.push(chars[i]);
            i += 1;
            continue;
        };

        let heard: Vec<char> = chars[i..i + wrong.len()].to_vec();
        let mut replacement: Vec<char> = right.chars().collect();
        // a word starting the line keeps a capital first letter
        if i == 0 && heard[0].is_uppercase() {
            replacement = capitalize_first(right).chars().collect();
        }

        let start = fixed.len();
        fixed.extend(&replacement);
        for spot in moved_to.iter_mut().take(i + wrong.len()).skip(i + 1) {
            *spot = start;
        }
        // only fixing the capitals can't change what it means, so no underline for that
        if !same_letters(&heard, &replacement) {
            new_fixes.push(WordFix { start, end: fixed.len(), heard: heard.iter().collect() });
        }
        i += wrong.len();
    }
    moved_to[chars.len()] = fixed.len();

    let old_length = chars.len();
    let mut fixes: Vec<WordFix> = line
        .fixes
        .iter()
        .map(|fix| WordFix {
            start: moved_to[fix.start.min(old_length)],
            end: moved_to[fix.end.min(old_length)],
            heard: fix.heard.clone(),
        })
        .collect();
    // unsure words move with the text, and one a fix just changed is settled
    let unsure: Vec<Unsure> = line
        .unsure
        .iter()
        .map(|word| Unsure {
            start: moved_to[word.start.min(old_length)],
            end: moved_to[word.end.min(old_length)],
            options: word.options.clone(),
        })
        .filter(|word| word.end > word.start && !new_fixes.iter().any(|fix| fix.start < word.end && fix.end > word.start))
        .collect();

    fixes.extend(new_fixes);
    fixes.sort_by_key(|fix| fix.start);

    line.text = fixed.into_iter().collect();
    line.fixes = fixes;
    line.unsure = unsure;
}

// after start..end of a line was swapped for something new_length long:
// unsure words after it move over, one inside it is gone (you just decided what it says)
fn move_unsure(line: &mut Segment, start: usize, end: usize, new_length: usize) {
    line.unsure.retain(|word| !(word.start < end && word.end > start));
    for word in line.unsure.iter_mut().filter(|word| word.start >= end) {
        word.start = word.start + new_length - (end - start);
        word.end = word.end + new_length - (end - start);
    }
}

// puts back what was heard for one fixed word, and remembers it so it isn't fixed again
pub fn change_back(line: &mut Segment, fix_index: usize) -> Result<(), String> {
    if fix_index >= line.fixes.len() {
        return Err("That word isn't fixed anymore".into());
    }
    let fix = line.fixes.remove(fix_index);
    let chars: Vec<char> = line.text.chars().collect();
    let (start, end) = (fix.start.min(chars.len()), fix.end.min(chars.len()));
    let heard_length = fix.heard.chars().count();

    let mut text: String = chars[..start].iter().collect();
    text.push_str(&fix.heard);
    text.extend(&chars[end..]);
    line.text = text;
    move_unsure(line, start, end, heard_length);

    // the fixes after it move over by however much longer or shorter it got
    for other in line.fixes.iter_mut().filter(|other| other.start >= end) {
        other.start = other.start + heard_length - (end - start);
        other.end = other.end + heard_length - (end - start);
    }
    let heard = fix.heard.to_lowercase();
    if !line.kept.contains(&heard) {
        line.kept.push(heard);
    }
    Ok(())
}

// every fix that turned into this word was right (like all five Maclaurins in a stretch)
pub fn keep_all(lines: &mut [Segment], word: &str) {
    let word: Vec<char> = word.chars().collect();
    for line in lines.iter_mut() {
        let chars: Vec<char> = line.text.chars().collect();
        line.fixes.retain(|fix| {
            let fixed = &chars[fix.start.min(chars.len())..fix.end.min(chars.len())];
            !same_letters(fixed, &word)
        });
    }
}

// changes one spot in a line by hand, the one nearest where you double clicked.
// it's underlined like a course fix so it can be changed back, and the course won't fix it again
pub fn change_one(line: &mut Segment, near: usize, heard: &str, word: &str) -> Result<(), String> {
    let (heard, word) = (heard.trim(), word.trim());
    if word.is_empty() || heard.is_empty() {
        return Err("Type what it should say".into());
    }
    let chars: Vec<char> = line.text.chars().collect();
    let heard_chars: Vec<char> = heard.chars().collect();
    let found = (0..chars.len().saturating_sub(heard_chars.len() - 1))
        .filter(|&i| {
            let end = i + heard_chars.len();
            same_letters(&chars[i..end], &heard_chars)
                && (i == 0 || !chars[i - 1].is_alphanumeric())
                && (end == chars.len() || !chars[end].is_alphanumeric())
        })
        .min_by_key(|&i| i.abs_diff(near))
        .ok_or_else(|| format!("\"{heard}\" isn't in this line"))?;
    let end = found + heard_chars.len();
    let word_length = word.chars().count();

    // if it was already fixed, changing it back should still give what whisper wrote
    let mut what_was_heard: String = chars[found..end].iter().collect();
    line.fixes.retain(|fix| {
        let overlaps = fix.start < end && fix.end > found;
        if overlaps && fix.start == found && fix.end == end {
            what_was_heard = fix.heard.clone();
        }
        !overlaps
    });
    for fix in line.fixes.iter_mut().filter(|fix| fix.start >= end) {
        fix.start = fix.start + word_length - (end - found);
        fix.end = fix.end + word_length - (end - found);
    }

    let mut text: String = chars[..found].iter().collect();
    text.push_str(word);
    text.extend(&chars[end..]);
    line.text = text;
    move_unsure(line, found, end, word_length);
    if !same_letters(&what_was_heard.chars().collect::<Vec<_>>(), &word.chars().collect::<Vec<_>>()) {
        line.fixes.push(WordFix { start: found, end: found + word_length, heard: what_was_heard });
        line.fixes.sort_by_key(|fix| fix.start);
    }
    let word = word.to_lowercase();
    if !line.kept.contains(&word) {
        line.kept.push(word);
    }
    Ok(())
}

// the fix was right, so it doesn't need an underline anymore
pub fn keep_fix(line: &mut Segment, fix_index: usize) -> Result<(), String> {
    if fix_index >= line.fixes.len() {
        return Err("That word isn't fixed anymore".into());
    }
    line.fixes.remove(fix_index);
    Ok(())
}

fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Segment {
        Segment { start_ms: 0, end_ms: 1000, text: text.to_string(), ..Default::default() }
    }

    fn math() -> Course {
        Course {
            subject: Some("math".into()),
            words: vec![
                CourseWord { word: "Maclaurin".into(), heard_as: vec!["McLaurin".into(), "Gordon".into(), "McClellan".into()] },
                CourseWord { word: "denominator".into(), heard_as: vec!["terminator".into()] },
                CourseWord { word: "Maple".into(), heard_as: vec![] },
            ],
            exams: Vec::new(),
            exam_choices: Vec::new(),
        }
    }

    fn fixed(text: &str) -> String {
        let mut lines = vec![line(text)];
        fix_words(&math(), &mut lines);
        lines.remove(0).text
    }

    #[test]
    fn misheard_words_are_put_right() {
        assert_eq!(fixed("So the Gordon series are fast."), "So the Maclaurin series are fast.");
        assert_eq!(fixed("we talk more about the mcclellan series"), "we talk more about the Maclaurin series");
        assert_eq!(fixed("push that terminator over to the side"), "push that denominator over to the side");
    }

    #[test]
    fn capitals_come_back_for_names() {
        assert_eq!(fixed("we'll do this in maple tomorrow."), "we'll do this in Maple tomorrow.");
    }

    #[test]
    fn a_fixed_word_starting_a_line_stays_capitalized() {
        assert_eq!(fixed("Terminator goes on the bottom."), "Denominator goes on the bottom.");
    }

    #[test]
    fn only_whole_words_change() {
        assert_eq!(fixed("the Gordons were there"), "the Gordons were there");
        assert_eq!(fixed("terminators"), "terminators");
        // a possessive still counts as the word
        assert_eq!(fixed("Gordon's theorem"), "Maclaurin's theorem");
    }

    fn fixed_line(text: &str) -> Segment {
        let mut lines = vec![line(text)];
        fix_words(&math(), &mut lines);
        lines.remove(0)
    }

    fn underlined(line: &Segment) -> Vec<(String, String)> {
        let chars: Vec<char> = line.text.chars().collect();
        line.fixes.iter().map(|fix| (chars[fix.start..fix.end].iter().collect(), fix.heard.clone())).collect()
    }

    #[test]
    fn fixes_remember_where_they_are_and_what_was_heard() {
        let fixed = fixed_line("Push that terminator over, the Gordon series uses maple.");
        assert_eq!(fixed.text, "Push that denominator over, the Maclaurin series uses Maple.");
        // maple to Maple is only capitals, so it isn't underlined
        assert_eq!(
            underlined(&fixed),
            [("denominator".into(), "terminator".into()), ("Maclaurin".into(), "Gordon".into())]
        );
    }

    #[test]
    fn earlier_fixes_move_when_a_new_word_is_added() {
        let mut lines = vec![line("The Gordon series has a terminator.")];
        let only_maclaurin = Course {
            subject: None,
            words: vec![CourseWord { word: "Maclaurin".into(), heard_as: vec!["Gordon".into()] }],
            exams: Vec::new(),
            exam_choices: Vec::new(),
        };
        fix_words(&only_maclaurin, &mut lines);
        // then the course gets another word, which lands before nothing but changes the length after
        let mut with_more = only_maclaurin.clone();
        with_more.words.insert(0, CourseWord { word: "denominator".into(), heard_as: vec!["terminator".into()] });
        let mut lines2 = lines.clone();
        fix_words(&with_more, &mut lines2);
        assert_eq!(lines2[0].text, "The Maclaurin series has a denominator.");
        assert_eq!(underlined(&lines2[0]), [("Maclaurin".into(), "Gordon".into()), ("denominator".into(), "terminator".into())]);
    }

    #[test]
    fn changing_back_puts_the_heard_word_back_and_it_stays() {
        let mut fixed = fixed_line("We'll use the Gordon series and the terminator.");
        change_back(&mut fixed, 0).unwrap();
        assert_eq!(fixed.text, "We'll use the Gordon series and the denominator.");
        // the other fix still points at the right word after the text got shorter
        assert_eq!(underlined(&fixed), [("denominator".into(), "terminator".into())]);

        // fixing again (like after adding another word) leaves Gordon alone in this line
        let mut lines = vec![fixed];
        fix_words(&math(), &mut lines);
        assert_eq!(lines[0].text, "We'll use the Gordon series and the denominator.");
    }

    #[test]
    fn keeping_a_fix_just_drops_the_underline() {
        let mut fixed = fixed_line("Push that terminator over.");
        keep_fix(&mut fixed, 0).unwrap();
        assert_eq!(fixed.text, "Push that denominator over.");
        assert!(fixed.fixes.is_empty());
    }

    #[test]
    fn adding_from_the_transcript_joins_the_word_already_there() {
        let mut course = math();
        add_word(&mut course, "Maclaurin", "mcloran").unwrap();
        let maclaurin = course.words.iter().find(|entry| entry.word == "Maclaurin").unwrap();
        assert!(maclaurin.heard_as.contains(&"mcloran".to_string()));
        assert_eq!(course.words.iter().filter(|entry| entry.word == "Maclaurin").count(), 1);

        // the same mishearing pointed somewhere new moves over
        add_word(&mut course, "determinant", "terminator").unwrap();
        assert!(course.words.iter().all(|entry| entry.word == "determinant" || !entry.heard_as.contains(&"terminator".to_string())));
        assert!(add_word(&mut course, "same", "same").is_err());
    }

    #[test]
    fn unsure_words_move_with_the_text() {
        let mut lines = vec![line("The Gordon series is a sum, use a nomial.")];
        // "nomial" at 33..39
        lines[0].unsure.push(Unsure { start: 33, end: 39, options: vec!["binomial".into()] });
        // and whisper wasn't sure about Gordon either
        lines[0].unsure.push(Unsure { start: 4, end: 10, options: Vec::new() });
        fix_words(&math(), &mut lines);
        // Gordon became Maclaurin, so that one's settled, and nomial moved over by 3
        assert_eq!(lines[0].text, "The Maclaurin series is a sum, use a nomial.");
        assert_eq!(lines[0].unsure, [Unsure { start: 36, end: 42, options: vec!["binomial".into()] }]);

        change_one(&mut lines[0], 36, "nomial", "binomial").unwrap();
        assert!(lines[0].unsure.is_empty());
    }

    #[test]
    fn keep_all_drops_every_underline_for_that_word() {
        let mut lines = vec![line("The Gordon series, the McLaurin series and a terminator.")];
        fix_words(&math(), &mut lines);
        keep_all(&mut lines, "maclaurin");
        assert_eq!(underlined(&lines[0]), [("denominator".into(), "terminator".into())]);
    }

    #[test]
    fn changing_one_spot_only_changes_that_spot() {
        let mut lines = vec![line("Gordon and Gordon again, then a terminator.")];
        fix_words(&math(), &mut lines);
        assert_eq!(lines[0].text, "Maclaurin and Maclaurin again, then a denominator.");
        // the professor really meant Gordon the second time
        change_one(&mut lines[0], 15, "Maclaurin", "Gordon").unwrap();
        assert_eq!(lines[0].text, "Maclaurin and Gordon again, then a denominator.");
        // the underline says what whisper first wrote, and the later fix moved over
        assert_eq!(
            underlined(&lines[0]),
            [("Maclaurin".into(), "Gordon".into()), ("denominator".into(), "terminator".into())]
        );
        // fixing the course again leaves it alone
        fix_words(&math(), &mut lines);
        assert_eq!(lines[0].text, "Maclaurin and Gordon again, then a denominator.");

        // two words whisper split up
        let mut split = line("the Mac Lauren series");
        change_one(&mut split, 5, "Mac Lauren", "Maclaurin").unwrap();
        assert_eq!(split.text, "the Maclaurin series");
        assert_eq!(underlined(&split), [("Maclaurin".into(), "Mac Lauren".into())]);
        assert!(change_one(&mut split, 0, "nowhere", "x").is_err());
    }

    #[test]
    fn saving_cleans_up_blanks_and_repeats() {
        let folder = std::env::temp_dir().join(format!("lecture-course-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();

        let messy = Course {
            subject: Some("  ".into()),
            words: vec![
                CourseWord { word: " Coulomb ".into(), heard_as: vec!["Bloom".into(), "".into(), "bloom".into(), "coulomb".into()] },
                CourseWord { word: "coulomb".into(), heard_as: vec![] },
                CourseWord { word: "".into(), heard_as: vec!["x".into()] },
            ],
            exams: Vec::new(),
            exam_choices: Vec::new(),
        };
        save_course(&folder, &messy).unwrap();
        let loaded = load_course(&folder);
        assert_eq!(loaded.subject, None);
        assert_eq!(loaded.words, vec![CourseWord { word: "Coulomb".into(), heard_as: vec!["Bloom".into()] }]);
    }
}
