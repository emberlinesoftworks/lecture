// prints which words whisper wasn't sure about in a few minutes of a recording,
// and what the other model heard there. for tuning the gray underlines
// cargo run --example unsure -- <model> <other model> <recording.wav> <start minute> <minutes>

use std::path::Path;

use lecture_lib::crosscheck::second_opinion;
use lecture_lib::transcriber::{load_audio, Transcriber, WHISPER_SAMPLE_RATE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model, other, wav) = (Path::new(&args[1]), Path::new(&args[2]), Path::new(&args[3]));
    let start_minute: i64 = args[4].parse().unwrap();
    let minutes: i64 = args[5].parse().unwrap();

    let audio = load_audio(wav).unwrap();
    let per_minute = WHISPER_SAMPLE_RATE as usize * 60;
    let from = (start_minute as usize * per_minute).min(audio.len());
    let to = (from + minutes as usize * per_minute).min(audio.len());

    let lines = Transcriber::load(model).unwrap().transcribe_audio(&audio[from..to], start_minute * 60_000).unwrap();
    let total_words: usize = lines.iter().map(|line| line.text.split_whitespace().count()).sum();
    let total_unsure: usize = lines.iter().map(|line| line.unsure.len()).sum();
    println!("{total_unsure} unsure out of {total_words} words\n");

    let results = second_opinion(other, wav, &lines).unwrap();
    for (line, result) in lines.iter().zip(results) {
        if line.unsure.is_empty() {
            continue;
        }
        let chars: Vec<char> = line.text.chars().collect();
        let marked: Vec<String> = line.unsure.iter().map(|word| chars[word.start..word.end].iter().collect()).collect();
        println!("{}\n  unsure: {}", line.text, marked.join(", "));
        for word in result.unwrap_or_default() {
            let heard: String = chars[word.start..word.end].iter().collect();
            println!("  after check: {heard} -> {:?}", word.options);
        }
    }
}
