// words whisper gets wrong in lots of classes, so nobody has to add them one by one.
// a course's subject picks which list it gets, and every lecture gets the everyday one.
// only put mix-ups here that are almost never what the professor really said,
// they still get underlined so a wrong one can be changed back

use crate::courses::{Course, CourseWord};

// (how it should be written, what whisper writes instead)
type Words = &'static [(&'static str, &'static [&'static str])];

const EVERYDAY: Words = &[
    ("syllabus", &["silly bus"]),
    ("D2L", &["D to L", "D-to-L", "D-2-L"]),
    ("PowerPoint", &["power point"]),
];

const MATH: Words = &[
    ("Maclaurin", &["McLaurin", "Mclaurin", "mcloran", "McLoran", "McLaren", "McClellan", "McCord"]),
    ("Taylor polynomial", &["tailored polynomial", "Taylor polynova", "Taylor-Polynova", "Taylor Pollinger", "Taylor-Pollinger", "Taylor-polynomial"]),
    ("Taylor series", &["tailor series", "Taylor-Shazer", "Taylor Shazer", "Taylor-series"]),
    ("polynomial", &["polynova", "polynomail"]),
    ("denominator", &["terminator"]),
    ("chain rule", &["change rule"]),
    ("cosine", &["co-sign", "cosign"]),
    ("divided by", &["Nevada by"]),
    ("L'Hôpital", &["Lopital", "low pital", "lo pital", "L'Hopital"]),
    ("Riemann sum", &["Raymond sum", "Riemann some"]),
    ("Lagrange", &["la grange"]),
    ("binomial expansion", &["my noble expansion", "bi-nomial expansion"]),
    ("Pascal's triangle", &["Pascal triangle"]),
    ("curving", &["perving"]),
    ("waviness", &["weaviness"]),
    ("Maple", &[]),
];

const PHYSICS: Words = &[
    ("Coulomb", &["coolum", "cool um"]),
    ("Kirchhoff", &["Kirchoff", "Kirk off"]),
    ("Ohm's law", &["homes law", "Om's law", "Ohms law"]),
    ("kinematics", &["cinematics"]),
    ("torque", &["tork"]),
    ("joules", &["jewels"]),
];

const CHEMISTRY: Words = &[
    ("stoichiometry", &["stoke geometry", "stoichiometery"]),
    ("Avogadro's number", &["avocado's number", "avocado number"]),
    ("Le Chatelier", &["La Chatelier", "lechatelier"]),
    ("covalent", &["co-valent"]),
];

const BIOLOGY: Words = &[
    ("eukaryotic", &["you carry otic"]),
    ("prokaryotic", &["pro carry otic"]),
    ("DNA polymerase", &["DNA polymers"]),
];

const NURSING: Words = &[
    ("NCLEX", &["N-Clex", "en clex", "Enclex"]),
    ("SBAR", &["S-bar"]),
];

const COMPUTER_SCIENCE: Words = &[
    ("GitHub", &["get hub", "Git Hub"]),
    ("Boolean", &["bullion"]),
    ("JavaScript", &["Java script"]),
    ("linked list", &["link list"]),
];

fn for_subject(subject: &str) -> Words {
    match subject {
        "math" => MATH,
        "physics" => PHYSICS,
        "chemistry" => CHEMISTRY,
        "biology" => BIOLOGY,
        "nursing" => NURSING,
        "computer science" => COMPUTER_SCIENCE,
        _ => &[],
    }
}

// the course's own words plus the built in ones for its subject (and the everyday ones).
// the course's go first, so if you added a word yourself, yours wins
pub fn with_built_in(course: &Course) -> Course {
    let mut words = course.words.clone();
    let built_in = for_subject(course.subject.as_deref().unwrap_or("")).iter().chain(EVERYDAY);
    for (word, heard_as) in built_in {
        // something you already said should be left alone isn't fixed back
        let heard_as: Vec<String> = heard_as
            .iter()
            .filter(|heard| !course.words.iter().any(|mine| mine.word.eq_ignore_ascii_case(heard)))
            .map(|heard| heard.to_string())
            .collect();
        words.push(CourseWord { word: word.to_string(), heard_as });
    }
    Course { subject: course.subject.clone(), words }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::courses::fix_words;
    use crate::transcriber::Segment;

    fn fixed(subject: Option<&str>, text: &str) -> String {
        let course = with_built_in(&Course { subject: subject.map(str::to_string), words: Vec::new() });
        let mut lines = vec![Segment { text: text.to_string(), ..Default::default() }];
        fix_words(&course, &mut lines);
        lines.remove(0).text
    }

    #[test]
    fn a_math_course_gets_the_math_words() {
        assert_eq!(
            fixed(Some("math"), "there's our Taylor-Pollinger, and the mcloran side of things"),
            "there's our Taylor polynomial, and the Maclaurin side of things"
        );
        assert_eq!(fixed(Some("math"), "negative 2 Nevada by 15"), "negative 2 divided by 15");
    }

    #[test]
    fn other_subjects_dont_get_math_words() {
        assert_eq!(fixed(Some("history"), "the terminator came out in 1984"), "the terminator came out in 1984");
        assert_eq!(fixed(None, "check the silly bus"), "check the syllabus");
    }

    #[test]
    fn your_own_words_come_first() {
        let course = Course {
            subject: Some("math".into()),
            words: vec![CourseWord { word: "McCord".into(), heard_as: Vec::new() }],
        };
        let mut lines = vec![Segment { text: "Professor McCord said so".into(), ..Default::default() }];
        fix_words(&with_built_in(&course), &mut lines);
        assert_eq!(lines[0].text, "Professor McCord said so");
    }

    #[test]
    fn every_built_in_word_is_written_sensibly() {
        for words in [EVERYDAY, MATH, PHYSICS, CHEMISTRY, BIOLOGY, NURSING, COMPUTER_SCIENCE] {
            for (word, heard_as) in words {
                assert!(!word.trim().is_empty());
                for heard in *heard_as {
                    assert!(!heard.trim().is_empty() && heard != word, "{heard} -> {word}");
                }
            }
        }
    }
}
