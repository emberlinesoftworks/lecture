// prints everything a transcript says about the exam
// cargo run --example flags_preview -- <lecture.json>

fn main() {
    let path = std::env::args().nth(1).expect("which transcript?");
    let json = std::fs::read_to_string(path).unwrap();
    let lines: Vec<lecture_lib::transcriber::Segment> =
        serde_json::from_value(serde_json::from_str::<serde_json::Value>(&json).unwrap()["lines"].clone()).unwrap();
    for note in lecture_lib::emphasis::find(&lines) {
        let seconds = lines[note.first_line].start_ms / 1000;
        let mut labels = Vec::new();
        if note.not_on_exam { labels.push("not on the exam") }
        if note.on_exam { labels.push("on the exam") }
        if note.date { labels.push("date") }
        if note.material { labels.push("study material") }
        println!("{:02}:{:02}  [{}]  {:?}", seconds / 60, seconds % 60, labels.join(", "), note.exams);
        println!("    {}", note.said);
    }
}
