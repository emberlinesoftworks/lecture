// prints every line of a transcript that math formatting would change, before and after
// cargo run --example math_preview -- <lecture.json>

fn main() {
    let path = std::env::args().nth(1).expect("which transcript?");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let lines = json["lines"].as_array().unwrap();
    let mut changed = 0;
    for line in lines {
        let text = line["text"].as_str().unwrap_or_default();
        let math = lecture_lib::mathtext::to_math(text);
        if math != text {
            changed += 1;
            println!("- {text}\n+ {math}\n");
        }
    }
    println!("{changed} of {} lines changed", lines.len());
}
