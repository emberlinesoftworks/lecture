// everything the professor said about the exam: when it is, review days, sample questions,
// what's on it and what isn't. worked out from the transcript each time a lecture opens,
// so it works on old lectures too.
// (tried first: tagging "be careful" and "don't forget" too. a math professor says those
// every few minutes, so it ended up highlighting ordinary work. only the exam talk was useful)

use serde::Serialize;

use crate::courses::{Course, Exam, ExamChoice};
use crate::transcriber::Segment;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ExamNote {
    pub first_line: usize,
    pub last_line: usize,
    // said it won't be on the exam, the most useful thing to know when studying
    pub not_on_exam: bool,
    // said what will be on it ("the exam questions are these Maclaurin problems")
    pub on_exam: bool,
    // named a day ("the Tuesday following the break")
    pub date: bool,
    // sample questions, review days, solutions, study guides
    pub material: bool,
    // just the part about the exam, not the whole run-on paragraph around it
    pub said: String,
    // the exams it names, like "Test 4" or "Final exam" (empty when it just says "the exam")
    pub exams: Vec<String>,
}

// words that mean the exam itself ("the final" on its own is left out, it's usually "the final answer")
const EXAM_WORDS: [&str; 12] = [
    "exam",
    "midterm",
    "quiz",
    "next test",
    "last test",
    "the test",
    "your test",
    "this test",
    "our test",
    "test four",
    "test three",
    "on the test",
];

// "test" in math, not an exam
const MATH_TESTS: [&str; 12] = [
    "ratio test",
    "root test",
    "comparison test",
    "integral test",
    "divergence test",
    "series test",
    "term test",
    "the test for",
    "the test is",
    "the test says",
    "the test tells",
    "the test fails",
];

const NOT_ON_EXAM: [&str; 14] = [
    "not part of the",
    "not part of your",
    "not be a part of",
    "not be part of",
    "not a part of your",
    "not relevant to the",
    "not relevant for the",
    "won't be on the",
    "will not be on the",
    "not going to be on the",
    "not on the exam",
    "not on the test",
    "don't need to know",
    "won't be tested",
];

const ON_EXAM: [&str; 12] = [
    "exam questions",
    "test questions",
    "on the exam",
    "on the test",
    "on the final",
    "on your exam",
    "on your test",
    "will be on",
    "going to be on",
    "be tested on",
    "exam will cover",
    "test will cover",
];

const DATES: [&str; 22] = [
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday", "next week", "after break",
    "the break", "tomorrow", "next class", "january", "february", "march", "april", "may", "august",
    "september", "october", "november", "december",
];

const MATERIAL: [&str; 11] = [
    "sample questions",
    "sample question",
    "practice",
    "review",
    "study guide",
    "solutions",
    "d2l",
    "canvas",
    "blackboard",
    "old exams",
    "formula sheet",
];

// how many words around the exam part to keep when a line runs on without punctuation
const LONGEST_SAID: usize = 30;

// lines this close together are one thing said
const JOIN_WITHIN: usize = 3;

// lowercase words with single spaces and a space on each end, so " exam " can't match "example"
fn said(text: &str) -> String {
    let lower = text.to_lowercase().replace('’', "'");
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|word| !word.is_empty()).collect();
    format!(" {} ", words.join(" "))
}

fn says(said: &str, phrase: &str) -> bool {
    said.contains(&format!(" {phrase} ")) || said.contains(&format!(" {phrase}s "))
}

fn about_the_exam(text: &str) -> bool {
    let mut said = said(text);
    for math in MATH_TESTS {
        said = said.replace(&format!(" {math} "), " ");
    }
    EXAM_WORDS.iter().any(|word| says(&said, word))
}

fn not_on_exam(text: &str) -> bool {
    let said = said(text);
    NOT_ON_EXAM.iter().any(|phrase| said.contains(&format!(" {phrase} ")))
}

fn mentions(text: &str, phrases: &[&str]) -> bool {
    let said = said(text);
    phrases.iter().any(|phrase| said.contains(&format!(" {phrase} ")))
}

// which exams a sentence names: "test four", "our fourth exam", "final exam", "midterm"
pub fn named_exams(text: &str) -> Vec<String> {
    const COUNTS: [(&str, &str, u32); 10] = [
        ("one", "first", 1),
        ("two", "second", 2),
        ("three", "third", 3),
        ("four", "fourth", 4),
        ("five", "fifth", 5),
        ("six", "sixth", 6),
        ("seven", "seventh", 7),
        ("eight", "eighth", 8),
        ("nine", "ninth", 9),
        ("ten", "tenth", 10),
    ];
    let said = said(text);
    let words: Vec<&str> = said.split_whitespace().collect();
    let mut names = Vec::new();
    let mut add = |name: String| {
        if !names.contains(&name) {
            names.push(name);
        }
    };
    for (i, word) in words.iter().enumerate() {
        let is_exam = |w: &str| matches!(w, "exam" | "test" | "exams" | "tests");
        // "test four", "exam 4"
        if is_exam(word) {
            if let Some(next) = words.get(i + 1) {
                let number = COUNTS.iter().find(|(said, _, _)| said == next).map(|c| c.2).or_else(|| next.parse().ok());
                if let Some(number) = number.filter(|n| (1..=10).contains(n)) {
                    add(format!("Test {number}"));
                }
            }
        }
        // "fourth exam", "our fourth test"
        if let Some((_, _, number)) = COUNTS.iter().find(|(_, ordinal, _)| ordinal == word) {
            if words.get(i + 1).is_some_and(|next| is_exam(next)) {
                add(format!("Test {number}"));
            }
        }
        // "final exam", and "studying for the final until" when exams are the subject
        if *word == "final" && (words.get(i + 1).is_some_and(|next| is_exam(next)) || words.get(i + 1).is_some_and(|next| matches!(*next, "until" | "is" | "will"))) {
            add("Final exam".into());
        }
        if *word == "midterm" || *word == "midterms" {
            add("Midterm".into());
        }
    }
    names
}

// the sentences that are about the exam. a line that runs on without punctuation
// gets cut down to the words around where the exam comes up
fn exam_part(text: &str) -> String {
    // "Dr." and "Mr." don't end a sentence
    let mut sentences: Vec<String> = Vec::new();
    let mut joining = false;
    for piece in text.split_inclusive(['.', '?', '!']).map(str::trim).filter(|piece| !piece.is_empty()) {
        match sentences.last_mut() {
            Some(last) if joining => {
                last.push(' ');
                last.push_str(piece);
            }
            _ => sentences.push(piece.to_string()),
        }
        let last_word = piece.split_whitespace().last().unwrap_or("").to_lowercase();
        joining = matches!(last_word.as_str(), "dr." | "mr." | "mrs." | "ms." | "prof." | "vs." | "e.g." | "i.e.");
    }
    let mut kept: Vec<String> = Vec::new();
    for sentence in &sentences {
        if !about_the_exam(sentence) && !mentions(sentence, &MATERIAL) {
            continue;
        }
        let words: Vec<&str> = sentence.split_whitespace().collect();
        if words.len() <= LONGEST_SAID {
            kept.push(sentence.clone());
            continue;
        }
        // a few words around each place the exam (or study material) comes up, joined where they overlap
        let mut windows: Vec<(usize, usize)> = Vec::new();
        for i in 0..words.len() {
            let near = words[i..(i + 3).min(words.len())].join(" ");
            if about_the_exam(&near) {
                let window = (i.saturating_sub(14), (i + 16).min(words.len()));
                match windows.last_mut() {
                    Some(last) if window.0 <= last.1 => last.1 = last.1.max(window.1),
                    _ => windows.push(window),
                }
            }
        }
        let parts: Vec<String> = windows.iter().map(|&(from, to)| words[from..to].join(" ")).collect();
        let mut part = parts.join(" … ");
        if windows.first().is_some_and(|w| w.0 > 0) {
            part = format!("…{part}");
        }
        if windows.last().is_some_and(|w| w.1 < words.len()) {
            part.push('…');
        }
        kept.push(part);
    }
    if kept.is_empty() {
        text.to_string()
    } else {
        kept.join(" ")
    }
}

pub fn find(lines: &[Segment]) -> Vec<ExamNote> {
    let mut notes: Vec<ExamNote> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.hidden || !about_the_exam(&line.text) {
            continue;
        }
        let not_on_exam = not_on_exam(&line.text);
        match notes.last_mut() {
            // after "that's not on the exam", the next thing said about the exam is something else
            // ("the exam questions are these problems"), so it gets its own note
            Some(last) if index <= last.last_line + JOIN_WITHIN && (!last.not_on_exam || not_on_exam) => {
                last.last_line = index;
                last.not_on_exam |= not_on_exam;
            }
            _ => notes.push(ExamNote {
                first_line: index,
                last_line: index,
                not_on_exam,
                on_exam: false,
                date: false,
                material: false,
                said: String::new(),
                exams: Vec::new(),
            }),
        }
    }

    for note in &mut notes {
        let text: Vec<&str> = lines[note.first_line..=note.last_line].iter().map(|line| line.text.as_str()).collect();
        let text = text.join(" ");
        note.said = exam_part(&text);
        note.on_exam = !note.not_on_exam && mentions(&note.said, &ON_EXAM);
        note.date = mentions(&note.said, &DATES);
        note.material = mentions(&note.said, &MATERIAL);
        note.exams = named_exams(&note.said);
    }
    notes
}

// --- which exam each note is about, and the course page that lists them all ---

#[derive(Serialize, Clone, Debug)]
pub struct CourseNote {
    pub path: String,
    pub recorded_at: u64,
    pub line: usize,
    pub start_ms: i64,
    pub said: String,
    pub not_on_exam: bool,
    pub on_exam: bool,
    pub date: bool,
    pub material: bool,
    // you put it here yourself (moved it, or added the line)
    pub chosen: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ExamGroup {
    pub name: String,
    // YYYY-MM-DD, when you added it on the course page
    pub date: Option<String>,
    // already happened, so the page folds it closed
    pub past: bool,
    pub notes: Vec<CourseNote>,
}

// what a lecture's own list shows: the note, and the exams it's under
#[derive(Serialize, Clone, Debug)]
pub struct SortedNote {
    #[serde(flatten)]
    pub note: ExamNote,
    pub start_ms: i64,
    pub chosen: bool,
}

pub const NEXT_EXAM: &str = "Next exam";

// how far a saved choice can be from the note it was made on and still find it
// (transcribing again moves line starts by a second or two)
const CHOICE_SLACK_MS: i64 = 3000;

// "Exam 4", "test four" and "Test 4" are the same exam
fn exam_key(name: &str) -> String {
    if let Some(named) = named_exams(name).into_iter().next() {
        return named;
    }
    let lower = name.to_lowercase();
    if lower.contains("final") {
        "Final exam".into()
    } else if lower.contains("midterm") {
        "Midterm".into()
    } else {
        name.trim().to_string()
    }
}

// the name you gave an exam on the course page, if this is one of yours
fn your_exam<'a>(name: &str, exams: &'a [Exam]) -> Option<&'a Exam> {
    let key = exam_key(name);
    exams.iter().find(|exam| exam_key(&exam.name) == key)
}

// the date a recording was made on, YYYY-MM-DD in your time zone
// (minutes_behind_utc is what the window's getTimezoneOffset gives, 300 for New York in winter)
pub fn day_of(recorded_at: u64, minutes_behind_utc: i64) -> String {
    let days = (recorded_at as i64 - minutes_behind_utc * 60).div_euclid(86_400);
    // days since 1970 to a calendar date (Howard Hinnant's civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 { month_part + 3 } else { month_part - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

// every exam note in a lecture and the exams it's under, your choices first:
// a note you moved goes where you put it, one you said isn't about an exam is left out,
// and a line you added is in even though Lecture didn't find it. a choice that points at an
// exam you've since removed is ignored, so that note goes back to being sorted on its own
pub fn sorted_notes(lines: &[Segment], file: &str, lecture_day: &str, course: &Course) -> Vec<SortedNote> {
    let choices: Vec<&ExamChoice> = course.exam_choices.iter().filter(|choice| choice.file == file).collect();
    let usable = |choice: &&ExamChoice| match &choice.exam {
        None => true,
        Some(name) => name == NEXT_EXAM || your_exam(name, &course.exams).is_some(),
    };
    let next_exam = || {
        course
            .exams
            .iter()
            .filter(|exam| exam.date.as_str() >= lecture_day)
            .min_by(|a, b| a.date.cmp(&b.date))
            .map_or(NEXT_EXAM.to_string(), |exam| exam.name.clone())
    };

    let mut sorted = Vec::new();
    let mut used = vec![false; choices.len()];
    for mut note in find(lines) {
        let from = lines[note.first_line].start_ms - CHOICE_SLACK_MS;
        let to = lines[note.last_line].end_ms + CHOICE_SLACK_MS;
        let choice = choices.iter().position(|choice| (from..=to).contains(&choice.at_ms) && usable(choice));
        if let Some(index) = choice {
            used[index] = true;
            match &choices[index].exam {
                None => continue,
                Some(name) => {
                    let name = your_exam(name, &course.exams).map_or(name.clone(), |exam| exam.name.clone());
                    note.exams = vec![name];
                }
            }
        } else if note.exams.is_empty() {
            note.exams = vec![next_exam()];
        } else {
            // "Test four" said out loud shows up under the name you gave it
            note.exams = note.exams.iter().map(|name| your_exam(name, &course.exams).map_or(name.clone(), |exam| exam.name.clone())).collect();
        }
        sorted.push(SortedNote { start_ms: lines[note.first_line].start_ms, chosen: choice.is_some(), note });
    }

    // lines you added yourself
    for (index, choice) in choices.iter().enumerate() {
        let Some(name) = choice.exam.as_ref().filter(|_| choice.added && !used[index] && usable(choice)) else { continue };
        let Some(line) = lines.iter().position(|line| !line.hidden && choice.at_ms >= line.start_ms - CHOICE_SLACK_MS && choice.at_ms <= line.end_ms) else { continue };
        let text = &lines[line].text;
        let name = your_exam(name, &course.exams).map_or(name.clone(), |exam| exam.name.clone());
        let note = ExamNote {
            first_line: line,
            last_line: line,
            not_on_exam: not_on_exam(text),
            on_exam: mentions(text, &ON_EXAM),
            date: mentions(text, &DATES),
            material: mentions(text, &MATERIAL),
            said: text.clone(),
            exams: vec![name],
        };
        sorted.push(SortedNote { start_ms: lines[line].start_ms, chosen: true, note });
    }
    sorted.sort_by_key(|sorted| sorted.start_ms);
    sorted
}

pub fn course_notes(folder: &std::path::Path, course: &Course, today: &str, minutes_behind_utc: i64) -> Vec<ExamGroup> {
    let mut wavs = crate::lectures::recordings_in(folder);
    for inner in crate::lectures::subfolders(folder) {
        wavs.extend(crate::lectures::recordings_in(&folder.join(inner)));
    }

    // your exams, then whatever else comes up
    let mut groups: Vec<ExamGroup> = course
        .exams
        .iter()
        .map(|exam| ExamGroup { name: exam.name.clone(), date: Some(exam.date.clone()), past: exam.date.as_str() < today, notes: Vec::new() })
        .collect();

    for wav in wavs {
        let Ok(transcript) = crate::lectures::load_transcript(&wav) else { continue };
        let recorded_at = crate::lectures::recorded_at(&wav);
        let file = wav.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let lecture_day = day_of(recorded_at, minutes_behind_utc);
        for sorted in sorted_notes(&transcript.lines, &file, &lecture_day, course) {
            let note = &sorted.note;
            for name in &note.exams {
                let index = match groups.iter().position(|group| exam_key(&group.name) == exam_key(name)) {
                    Some(index) => index,
                    None => {
                        groups.push(ExamGroup { name: name.clone(), date: None, past: false, notes: Vec::new() });
                        groups.len() - 1
                    }
                };
                groups[index].notes.push(CourseNote {
                    path: wav.display().to_string(),
                    recorded_at,
                    line: note.first_line,
                    start_ms: sorted.start_ms,
                    said: note.said.clone(),
                    not_on_exam: note.not_on_exam,
                    on_exam: note.on_exam,
                    date: note.date,
                    material: note.material,
                    chosen: sorted.chosen,
                });
            }
        }
    }

    for group in &mut groups {
        // newest lecture first, in the order it was said
        group.notes.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at).then(a.start_ms.cmp(&b.start_ms)));
    }
    groups.retain(|group| !group.notes.is_empty() || group.date.is_some());

    // coming up first (soonest on top), then ones without a date, then ones that already happened (latest on top)
    let (mut past, rest): (Vec<ExamGroup>, Vec<ExamGroup>) = groups.into_iter().partition(|group| group.past);
    let (mut upcoming, undated): (Vec<ExamGroup>, Vec<ExamGroup>) = rest.into_iter().partition(|group| group.date.is_some());
    upcoming.sort_by(|a, b| a.date.cmp(&b.date));
    past.sort_by(|a, b| b.date.cmp(&a.date));
    upcoming.into_iter().chain(undated).chain(past).collect()
}

// saves where you put one exam note. exam: none for "not about an exam". clear puts it back to automatic.
// a choice already saved for the same moment is replaced
pub fn choose(course: &mut Course, file: &str, at_ms: i64, exam: Option<String>, added: bool, clear: bool) {
    // keep "added" if this note was one you added, so moving it later doesn't lose it
    let was_added = course
        .exam_choices
        .iter()
        .any(|choice| choice.file == file && choice.added && (choice.at_ms - at_ms).abs() <= CHOICE_SLACK_MS);
    course.exam_choices.retain(|choice| !(choice.file == file && (choice.at_ms - at_ms).abs() <= CHOICE_SLACK_MS));
    if !clear {
        course.exam_choices.push(ExamChoice { file: file.to_string(), at_ms, exam, added: added || was_added });
    }
}

// renaming an exam keeps everything you put under it
pub fn rename_exam(course: &mut Course, old: &str, new: &str) {
    for choice in &mut course.exam_choices {
        if choice.exam.as_deref() == Some(old) {
            choice.exam = Some(new.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Segment {
        Segment { text: text.into(), ..Default::default() }
    }

    #[test]
    fn what_a_real_professor_said_about_the_exam() {
        for text in [
            "noting that our next exam will be the tuesday following the break",
            "I'll post into D2L some sample questions for you to start reviewing for the final exam.",
            "I don't expect most of you to look at that until after test four.",
            "In terms of the exam questions, it's getting the, and then again, doing the same process.",
            "So start organizing those thoughts for getting ready for your last test, as well as final exam.",
            "Not part of the exam, okay, but just talking about as discussion.",
        ] {
            assert!(about_the_exam(text), "{text}");
        }
        assert!(not_on_exam("because it's not relevant to the exam."));
        assert!(not_on_exam("That will not be a part of your final exam, just a piece of the puzzle."));
        assert!(!not_on_exam("our next exam will be the tuesday following the break"));
    }

    #[test]
    fn math_isnt_exam_talk() {
        for text in [
            "So the way we can test that is, let's pick some different numbers.",
            "Use the ratio test here.",
            "The test fails when the limit is one.",
            "So be careful of signs.",
            "But again, stress, very important part.",
            "And then the final answer is eight thirds.",
            "When we come back on Wednesday, three is an example of that.",
        ] {
            assert!(!about_the_exam(text), "{text}");
        }
    }

    #[test]
    fn the_wednesday_lecture() {
        let lines = [
            line("series review noting that our next exam will be the tuesday following the break so the monday following the break that'll be another review day just if you have any questions about some things that you haven't seen in a while or a little confused"),
            line("Uh and then the wednesday friday and the uh following monday following break those are just to be final review questions all right before Friday, one thing I'm going to do is I'll post into D2L some sample questions for you to start reviewing for the final exam."),
            line("I don't expect most of you to look at that until after test four, but some of you might want to start putting stuff together."),
            line("Start organizing your thoughts, but don't worry about studying for the final until after we take our fourth exam."),
        ];
        let notes = find(&lines);
        assert_eq!(notes.len(), 1);
        let note = &notes[0];
        assert!(note.date && note.material && !note.not_on_exam);
        assert!(note.exams.contains(&"Final exam".to_string()) && note.exams.contains(&"Test 4".to_string()));
        // the run-on first line is cut down to the part about the exam
        assert!(note.said.contains("our next exam will be the tuesday following the break"), "{}", note.said);
        assert!(note.said.contains("post into D2L some sample questions for you to start reviewing for the final exam."), "{}", note.said);

        let questions = find(&[line("In terms of the exam questions, it's these Maclaurin problems.")]);
        assert!(questions[0].on_exam && questions[0].exams.is_empty());
    }

    #[test]
    fn exam_names() {
        assert_eq!(named_exams("after test four"), ["Test 4"]);
        assert_eq!(named_exams("before our fourth exam"), ["Test 4"]);
        assert_eq!(named_exams("exam 3 is Friday"), ["Test 3"]);
        assert_eq!(named_exams("the final exam is cumulative"), ["Final exam"]);
        assert_eq!(named_exams("the midterm"), ["Midterm"]);
        assert!(named_exams("our next exam").is_empty());
        assert!(named_exams("the final answer is four").is_empty());
    }

    #[test]
    fn dates_in_your_time_zone() {
        // Nov 18 2026 15:57 in New York
        assert_eq!(day_of(1_795_017_470, 300), "2026-11-18");
        // 9pm in New York is already the next day in UTC
        assert_eq!(day_of(1_795_053_600, 300), "2026-11-18");
        assert_eq!(day_of(1_795_053_600, 0), "2026-11-19");
        assert_eq!(day_of(0, 0), "1970-01-01");
    }

    #[test]
    fn notes_land_under_the_right_exam() {
        let folder = std::env::temp_dir().join(format!("lecture-exams-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let save = |stamp: u64, lines: &[&str]| {
            let wav = folder.join(format!("lecture-{stamp}.wav"));
            std::fs::write(&wav, b"").unwrap();
            let transcript = serde_json::json!({
                "lines": lines.iter().enumerate().map(|(i, text)| serde_json::json!({"start_ms": i as i64 * 5000, "end_ms": i as i64 * 5000 + 4000, "text": text})).collect::<Vec<_>>(),
                "marks": [],
            });
            std::fs::write(wav.with_extension("json"), transcript.to_string()).unwrap();
        };
        // Nov 18: an unnamed exam, a named one, and something not on it
        save(1_795_017_470, &["Our next exam will be the Tuesday after break.", "lots of math", "more math", "more", "more", "Start studying for the final exam after that.", "x", "x", "x", "x", "That's not relevant to the exam."]);
        // Dec 3, after Test 4: "the exam" now means the final
        save(1_796_313_600, &["The exam is cumulative."]);

        let mut course = Course {
            exams: vec![
                Exam { name: "Test 4".into(), date: "2026-12-01".into() },
                Exam { name: "Final".into(), date: "2026-12-14".into() },
            ],
            ..Default::default()
        };
        let groups = course_notes(&folder, &course, "2026-12-05", 300);
        let summary: Vec<(String, bool, usize)> = groups.iter().map(|g| (g.name.clone(), g.past, g.notes.len())).collect();
        assert_eq!(summary, [("Final".to_string(), false, 2), ("Test 4".to_string(), true, 2)]);
        // newest lecture first
        assert_eq!(groups[0].notes[0].said, "The exam is cumulative.");

        let summary = |groups: &[ExamGroup]| -> Vec<(String, Vec<String>)> {
            groups.iter().map(|g| (g.name.clone(), g.notes.iter().map(|n| n.said.clone()).collect())).collect()
        };
        let nov18 = "lecture-1795017470.wav";
        let not_relevant = 50_000; // line 10, "That's not relevant to the exam."

        // 1. he said "the final" but meant Test 4: move it, and it stays
        choose(&mut course, nov18, 25_000, Some("Test 4".into()), false, false);
        let groups = course_notes(&folder, &course, "2026-11-20", 300);
        assert_eq!(
            summary(&groups),
            [
                ("Test 4".to_string(), vec!["Our next exam will be the Tuesday after break.".to_string(), "Start studying for the final exam after that.".to_string(), "That's not relevant to the exam.".to_string()]),
                ("Final".to_string(), vec!["The exam is cumulative.".to_string()]),
            ]
        );
        assert!(groups[0].notes[1].chosen);

        // 2. not about an exam: gone, and stays gone
        choose(&mut course, nov18, not_relevant, None, false, false);
        let groups = course_notes(&folder, &course, "2026-11-20", 300);
        assert_eq!(groups[0].notes.len(), 2);

        // 3. a line it missed, added by hand
        choose(&mut course, nov18, 10_000, Some("Final".into()), true, false);
        let groups = course_notes(&folder, &course, "2026-11-20", 300);
        assert!(groups[1].notes.iter().any(|n| n.said == "more math" && n.chosen));

        // 4. the exam moves: notes re-sort by date, and ones you placed stay put
        course.exams[0].date = "2026-12-20".into();
        let groups = course_notes(&folder, &course, "2026-11-20", 300);
        assert_eq!(groups[0].name, "Final");
        assert!(groups[1].notes.iter().any(|n| n.said == "Start studying for the final exam after that."));

        // renaming an exam keeps what you put under it
        rename_exam(&mut course, "Test 4", "Test 4 (moved)");
        course.exams[0].name = "Test 4 (moved)".into();
        let groups = course_notes(&folder, &course, "2026-11-20", 300);
        assert!(groups.iter().any(|g| g.name == "Test 4 (moved)" && g.notes.iter().any(|n| n.said.starts_with("Start studying"))));

        // 5. the exam is removed: what you moved there goes back to being sorted on its own
        course.exams.remove(0);
        let groups = course_notes(&folder, &course, "2026-11-20", 300);
        assert!(groups.iter().any(|g| g.name == "Final exam" && g.notes.iter().any(|n| n.said.starts_with("Start studying"))) || groups.iter().any(|g| g.name == "Final" && g.notes.iter().any(|n| n.said.starts_with("Start studying"))));

        // 6. transcribing again shifts lines a little: choices still find their note
        choose(&mut course, nov18, 25_000, None, false, false);
        let lines: Vec<Segment> = (0..11).map(|i| Segment { start_ms: i * 5000 + 1200, end_ms: i * 5000 + 5000, text: if i == 5 { "Start studying for the final exam after that.".into() } else { "math".into() }, ..Default::default() }).collect();
        assert!(sorted_notes(&lines, nov18, "2026-11-18", &course).iter().all(|n| !n.note.said.starts_with("Start studying")));

        // clearing a choice puts it back to automatic
        choose(&mut course, nov18, 25_000, None, false, true);
        assert!(sorted_notes(&lines, nov18, "2026-11-18", &course).iter().any(|n| n.note.said.starts_with("Start studying")));
        std::fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn nearby_mentions_are_one_note() {
        let lines = [
            line("Our next exam will be the Tuesday after break."),
            line("Monday will be a review day."),
            line("And I'll post sample questions for the final exam."),
            line("Okay, the Maclaurin series."),
            line("Lots of work here."),
            line("More work."),
            line("That's not part of the exam."),
            line("The exam questions are these Maclaurin problems."),
        ];
        let found: Vec<(usize, usize, bool)> =
            find(&lines).iter().map(|note| (note.first_line, note.last_line, note.not_on_exam)).collect();
        assert_eq!(found, [(0, 2, false), (6, 6, true), (7, 7, false)]);
    }
}
