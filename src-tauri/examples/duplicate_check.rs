// drops a file "on paper": shows whether Lecture would say you already have it,
// and how well its sound lines up with every lecture you have (1.0 is the same recording)
// cargo run --release --example duplicate_check -- <file> <your Lecture folder>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (source, folder) = (std::path::Path::new(&args[1]), std::path::Path::new(&args[2]));
    let lectures = lecture_lib::lectures::list_lectures(folder);
    match lecture_lib::importer::already_have(&lectures, source) {
        Some(lecture) => println!("already have it: {}\n", lecture.title.as_deref().unwrap_or(&lecture.path)),
        None => println!("new, would be imported\n"),
    }
    for lecture in &lectures {
        let score = lecture_lib::importer::how_alike(source, std::path::Path::new(&lecture.path)).unwrap_or(0.0);
        println!("{score:.3}  {}", lecture.title.as_deref().unwrap_or("Untitled lecture"));
    }
}
