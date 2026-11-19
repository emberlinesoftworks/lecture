// spoken math written the way it's written on the board:
// "6 times the sine squared of 0" becomes "6 × sin²(0)", "8 thirds" becomes "8/3".
// it's only for reading (and exports), the transcript itself keeps the spoken words.
// a number word only turns into a digit when it's part of some math, so
// "which one's the outer function" stays the way it was said

// a word and whatever punctuation came right after it
#[derive(Clone, Debug)]
struct Piece {
    text: String,
    after: String,
    // already math (a digit, x, sin(x), 8/3...), so it can be part of more math
    math: bool,
}

const NUMBERS: [&str; 21] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty",
];

// "8 thirds", "one half"
const PARTS: [(&str, &str, u32); 9] = [
    ("half", "halves", 2),
    ("third", "thirds", 3),
    ("fourth", "fourths", 4),
    ("fifth", "fifths", 5),
    ("sixth", "sixths", 6),
    ("seventh", "sevenths", 7),
    ("eighth", "eighths", 8),
    ("ninth", "ninths", 9),
    ("tenth", "tenths", 10),
];

// "to the fourth", "to the second power"
const POWERS: [(&str, &str); 9] = [
    ("second", "2"),
    ("third", "3"),
    ("fourth", "4"),
    ("fifth", "5"),
    ("sixth", "6"),
    ("seventh", "7"),
    ("eighth", "8"),
    ("ninth", "9"),
    ("nth", "n"),
];

const FUNCTIONS: [(&str, &str); 7] = [
    ("sine", "sin"),
    ("cosine", "cos"),
    ("tangent", "tan"),
    ("secant", "sec"),
    ("cosecant", "csc"),
    ("cotangent", "cot"),
    ("log", "log"),
];

// letters that are almost always a variable when they stand alone ("a" and "I" are words)
const VARIABLES: [&str; 6] = ["x", "y", "z", "n", "k", "t"];

pub fn to_math(text: &str) -> String {
    let mut pieces = split(text);
    functions(&mut pieces);
    powers(&mut pieces);
    factorials(&mut pieces);
    fractions(&mut pieces);
    negatives(&mut pieces);
    coefficients(&mut pieces);
    operators(&mut pieces);
    join(&pieces)
}

fn split(text: &str) -> Vec<Piece> {
    text.split_whitespace()
        .map(|word| {
            let cut = word.trim_end_matches(|c: char| matches!(c, ',' | '.' | '?' | '!' | ';' | ':')).len();
            let (text, after) = word.split_at(cut);
            Piece { text: text.to_string(), after: after.to_string(), math: is_digits(text) }
        })
        .collect()
}

fn join(pieces: &[Piece]) -> String {
    let words: Vec<String> = pieces.iter().map(|piece| format!("{}{}", piece.text, piece.after)).collect();
    words.join(" ")
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn lower(piece: &Piece) -> String {
    piece.text.to_lowercase()
}

fn number_word(word: &str) -> Option<usize> {
    NUMBERS.iter().position(|number| *number == word)
}

// what a piece looks like as math, if it can be part of math at all
fn as_term(piece: &Piece) -> Option<String> {
    let word = lower(piece);
    if piece.math {
        Some(piece.text.clone())
    } else if let Some(value) = number_word(&word) {
        Some(value.to_string())
    } else if VARIABLES.contains(&word.as_str()) {
        Some(word)
    } else if word == "pi" {
        Some("π".into())
    } else {
        None
    }
}

// a term only joins up when nothing (like a comma) separates it from what follows
fn open(piece: &Piece) -> bool {
    piece.after.is_empty()
}

// replaces pieces[from..to] with one math piece, keeping the punctuation of the last
fn merge(pieces: &mut Vec<Piece>, from: usize, to: usize, text: String) {
    let after = pieces[to - 1].after.clone();
    pieces.splice(from..to, [Piece { text, after, math: true }]);
}

fn superscript(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            'n' => 'ⁿ',
            other => other,
        })
        .collect()
}

// "(the) cosine (squared) of zero" -> "cos²(0)"
fn functions(pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while i < pieces.len() {
        let name = FUNCTIONS.iter().find(|(said, _)| *said == lower(&pieces[i])).map(|(_, short)| *short);
        let Some(name) = name else {
            i += 1;
            continue;
        };
        let mut next = i + 1;
        let mut power = "";
        if open(&pieces[i]) {
            match pieces.get(next).map(lower).as_deref() {
                Some("squared") => (power, next) = ("²", next + 1),
                Some("cubed") => (power, next) = ("³", next + 1),
                _ => {}
            }
        }
        let has_of = next > i && open(&pieces[next - 1]) && pieces.get(next).map(lower).as_deref() == Some("of");
        let argument = pieces.get(next + 1).filter(|_| has_of && open(&pieces[next])).and_then(as_term);
        let Some(argument) = argument else {
            i += 1;
            continue;
        };
        // "the cosine of zero" reads as just cos(0)
        let from = if i > 0 && lower(&pieces[i - 1]) == "the" && open(&pieces[i - 1]) { i - 1 } else { i };
        merge(pieces, from, next + 2, format!("{name}{power}({argument})"));
        i = from + 1;
    }
}

// "x squared", "x to the fourth", "x to the power of 5"
fn powers(pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while i < pieces.len() {
        let Some(base) = as_term(&pieces[i]).filter(|_| open(&pieces[i])) else {
            i += 1;
            continue;
        };
        let word = |k: usize| pieces.get(k).map(lower).unwrap_or_default();
        let (power, length) = match word(i + 1).as_str() {
            "squared" => (Some("2".to_string()), 2),
            "cubed" => (Some("3".to_string()), 2),
            "to" if word(i + 2) == "the" => {
                if word(i + 3) == "power" && word(i + 4) == "of" {
                    (pieces.get(i + 5).and_then(as_term), 6)
                } else {
                    let ordinal = POWERS.iter().find(|(said, _)| *said == word(i + 3)).map(|(_, digit)| digit.to_string());
                    // "to the fourth power" says power at the end
                    let length = if word(i + 4) == "power" && open(&pieces[i + 3]) { 5 } else { 4 };
                    (ordinal, length)
                }
            }
            _ => (None, 0),
        };
        // the words in between can't have punctuation, or it isn't one phrase
        if let Some(power) = power {
            let joined = (i..i + length - 1).all(|k| pieces.get(k).is_some_and(open));
            if joined && i + length <= pieces.len() {
                // "x minus 7 to the seventh" is (x − 7)⁷, the way it always is in a Taylor series
                let shifted = i >= 2 && open(&pieces[i - 2]) && open(&pieces[i - 1]);
                let sign = if shifted { sign_of(&pieces[i - 1]) } else { None };
                // a plain letter, not something that's already math like x⁴
                let letter = shifted && !pieces[i - 2].math && VARIABLES.contains(&lower(&pieces[i - 2]).as_str());
                let variable = if letter { Some(lower(&pieces[i - 2])) } else { None };
                match (sign, variable) {
                    (Some(sign), Some(variable)) => {
                        merge(pieces, i - 2, i + length, format!("({variable} {sign} {base}){}", superscript(&power)));
                        i -= 1;
                    }
                    _ => merge(pieces, i, i + length, format!("{base}{}", superscript(&power))),
                }
            }
        }
        i += 1;
    }
}

fn sign_of(piece: &Piece) -> Option<&'static str> {
    match lower(piece).as_str() {
        "plus" => Some("+"),
        "minus" => Some("−"),
        _ => None,
    }
}

fn is_variable(term: &str) -> bool {
    VARIABLES.iter().any(|letter| term.starts_with(letter)) || term.starts_with("(x")
}

// "−3 x²" -> "−3x²", a number right before a variable is how many of it
fn coefficients(pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while i + 1 < pieces.len() {
        let number = as_term(&pieces[i]).filter(|term| open(&pieces[i]) && is_digits(term.trim_start_matches('−')));
        let variable = as_term(&pieces[i + 1]).filter(|term| pieces[i + 1].math && is_variable(term));
        if let (Some(number), Some(variable)) = (number, variable) {
            // one x⁴ is just x⁴
            let number = match number.as_str() {
                "1" => String::new(),
                "−1" => "−".into(),
                _ => number,
            };
            merge(pieces, i, i + 2, format!("{number}{variable}"));
        }
        i += 1;
    }
}

// "n factorial" -> "n!"
fn factorials(pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while i + 1 < pieces.len() {
        if lower(&pieces[i + 1]) == "factorial" && open(&pieces[i]) {
            if let Some(term) = as_term(&pieces[i]) {
                merge(pieces, i, i + 2, format!("{term}!"));
            }
        }
        i += 1;
    }
}

// "8 thirds" -> "8/3", "one half" -> "1/2", "x over 2" -> "x/2", "2 divided by 15" -> "2/15"
fn fractions(pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while i + 1 < pieces.len() {
        let top = as_term(&pieces[i]).filter(|_| open(&pieces[i]));
        let next = lower(&pieces[i + 1]);
        let part = PARTS.iter().find(|(one, many, _)| *many == next || (*one == next && top.as_deref() == Some("1")));
        match (top, part) {
            (Some(top), Some((_, _, bottom))) if is_digits(&top) => {
                merge(pieces, i, i + 2, format!("{top}/{bottom}"));
            }
            (Some(top), None) => {
                let (bottom_at, length) = match next.as_str() {
                    "over" => (i + 2, 3),
                    "divided" if pieces.get(i + 2).map(lower).as_deref() == Some("by") && open(&pieces[i + 1]) => (i + 3, 4),
                    _ => (0, 0),
                };
                let bottom = pieces.get(bottom_at).filter(|_| length > 0 && open(&pieces[bottom_at - 1])).and_then(as_term);
                if let Some(bottom) = bottom {
                    merge(pieces, i, i + length, format!("{top}/{bottom}"));
                }
            }
            _ => {}
        }
        i += 1;
    }
}

// "negative six" -> "−6"
fn negatives(pieces: &mut Vec<Piece>) {
    let mut i = 0;
    while i + 1 < pieces.len() {
        if lower(&pieces[i]) == "negative" && open(&pieces[i]) {
            if let Some(term) = as_term(&pieces[i + 1]) {
                merge(pieces, i, i + 2, format!("−{term}"));
            }
        }
        i += 1;
    }
}

// "6 times cos(x)" -> "6 × cos(x)", only between two pieces of math
fn operators(pieces: &mut Vec<Piece>) {
    let mut i = 1;
    while i + 1 < pieces.len() {
        let symbol = match lower(&pieces[i]).as_str() {
            "times" => Some("×"),
            "plus" => Some("+"),
            "minus" => Some("−"),
            "equals" => Some("="),
            _ => None,
        };
        let left = as_term(&pieces[i - 1]).filter(|_| open(&pieces[i - 1]) && open(&pieces[i]));
        let right = as_term(&pieces[i + 1]);
        if let (Some(symbol), Some(left), Some(right)) = (symbol, left, right) {
            merge(pieces, i - 1, i + 2, format!("{left} {symbol} {right}"));
            // the result can be the left side of the next one: 6 × sin²(0) − 6 × cos²(0)
            continue;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::to_math;

    #[test]
    fn lines_from_a_real_class() {
        assert_eq!(
            to_math("So 6 times the sine squared of 0 minus 6 times the cosine squared of 0."),
            "So 6 × sin²(0) − 6 × cos²(0)."
        );
        assert_eq!(
            to_math("Now we're doing negative six times the cosine of zero."),
            "Now we're doing −6 × cos(0)."
        );
        assert_eq!(
            to_math("So we are going to have n factorial on the bottom, meaning zero factorial goes first."),
            "So we are going to have n! on the bottom, meaning 0! goes first."
        );
        assert_eq!(to_math("negative 2 divided by 15"), "−2/15");
        assert_eq!(to_math("and the cosine of zero equals one."), "and cos(0) = 1.");
    }

    #[test]
    fn fractions_and_powers() {
        assert_eq!(to_math("that gives us 8 thirds"), "that gives us 8/3");
        assert_eq!(to_math("one half of it"), "1/2 of it");
        assert_eq!(to_math("x to the fourth plus x squared"), "x⁴ + x²");
        assert_eq!(to_math("x to the fourth power"), "x⁴");
        assert_eq!(to_math("x to the power of n"), "xⁿ");
        assert_eq!(to_math("cosine of x times sine of x"), "cos(x) × sin(x)");
        assert_eq!(to_math("x over 2"), "x/2");
        assert_eq!(
            to_math("there's going to be x minus 7 to the seventh power, or x plus 2 to the third power."),
            "there's going to be (x − 7)⁷, or (x + 2)³."
        );
        assert_eq!(to_math("Half of negative six, produce negative three x squared."), "Half of −6, produce −3x².");
        assert_eq!(to_math("leaving us with one x to the fourth."), "leaving us with x⁴.");
    }

    #[test]
    fn ordinary_talk_stays_the_way_it_was_said() {
        for line in [
            "Which one's the outer function.",
            "Let's go to the next one.",
            "Remember cosine first or power first?",
            "We're going to find our third derivative.",
            "As long as we find non-zero terms, I got to get four of them.",
            "What was the derivative of cosine?",
            "Plus the first.",
            "A third of the class was gone.",
            "Two, three, four.",
        ] {
            assert_eq!(to_math(line), line);
        }
    }
}
