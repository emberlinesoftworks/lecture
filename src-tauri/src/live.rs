use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::cooling::{current_heat, hurry_up, rest, run_in_background, Heat};
use crate::transcriber::{ends_sentence, Segment, Transcriber, WHISPER_SAMPLE_RATE};

// transcribe about a minute at a time, so a 75 minute class is
// almost all done by the time you press stop
const CHUNK_SECONDS: usize = 60;
// look for a quiet spot in the last few seconds of a chunk to cut at,
// so a word doesn't get split in half between two chunks
const CUT_SEARCH_SECONDS: usize = 5;
// when the mac is warm, wait this long between chunks
const WARM_REST: Duration = Duration::from_secs(20);
// when it's hot, check again this often until it cools down
const HOT_REST: Duration = Duration::from_secs(10);

// audio the recorder has captured that hasn't been transcribed yet (16k mono)
pub type PendingAudio = Arc<Mutex<Vec<f32>>>;

pub struct LiveTranscriber {
    finished: Arc<AtomicBool>,
    worker: JoinHandle<Result<Vec<Segment>, String>>,
}

impl LiveTranscriber {
    // on_progress gets called with how far into the recording is transcribed, in ms.
    // on_resting gets called with true when it takes a break to let the mac cool down,
    // and false when it starts again
    pub fn start(
        model_path: PathBuf,
        pending: PendingAudio,
        on_progress: impl Fn(i64) + Send + 'static,
        on_resting: impl Fn(bool) + Send + 'static,
    ) -> Self {
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let worker = thread::spawn(move || run(&model_path, &pending, &worker_finished, on_progress, on_resting));
        Self { finished, worker }
    }

    // call after the recording has stopped: transcribes whatever is left and returns every line
    pub fn finish(self) -> Result<Vec<Segment>, String> {
        self.finished.store(true, Ordering::SeqCst);
        self.worker.join().map_err(|_| "transcription thread panicked".to_string())?
    }
}

fn run(
    model_path: &PathBuf,
    pending: &PendingAudio,
    finished: &AtomicBool,
    on_progress: impl Fn(i64),
    on_resting: impl Fn(bool),
) -> Result<Vec<Segment>, String> {
    run_in_background();
    let transcriber = Transcriber::load(model_path)?;
    let chunk_length = CHUNK_SECONDS * WHISPER_SAMPLE_RATE as usize;

    let mut lines = Vec::new();
    let mut done_samples = 0;
    let mut resting = false;
    let mut hurrying = false;

    loop {
        // check finished before looking at the audio, so nothing recorded
        // right at the end gets missed
        let recording_stopped = finished.load(Ordering::SeqCst);

        if recording_stopped && !hurrying {
            hurry_up();
            hurrying = true;
        }

        // too hot: don't transcribe anything until it cools down. the recorder keeps
        // adding to pending the whole time, so nothing is lost, it just catches up later
        if !recording_stopped && current_heat() == Heat::Hot {
            if !resting {
                on_resting(true);
                resting = true;
            }
            rest(HOT_REST, finished);
            continue;
        }

        let chunk: Vec<f32> = {
            let mut audio = pending.lock().unwrap();
            if recording_stopped {
                // take everything that's left
                audio.drain(..).collect()
            } else if audio.len() >= chunk_length {
                let cut = best_cut(&audio[..chunk_length]);
                audio.drain(..cut).collect()
            } else {
                Vec::new()
            }
        };

        if chunk.is_empty() {
            if recording_stopped {
                break;
            }
            // not a full minute yet, check again in a bit
            thread::sleep(Duration::from_millis(500));
            continue;
        }

        if resting {
            on_resting(false);
            resting = false;
        }

        let start_ms = samples_to_ms(done_samples);
        let new_lines = transcriber.transcribe_audio(&chunk, start_ms)?;
        join_pieces(&mut lines, new_lines);
        done_samples += chunk.len();
        on_progress(samples_to_ms(done_samples));

        if recording_stopped {
            break;
        }

        // warm: still keep going, just with a break between chunks
        if current_heat() == Heat::Warm {
            on_resting(true);
            resting = true;
            rest(WARM_REST, finished);
        }
    }

    Ok(lines)
}

// adds a new piece's lines onto the transcript. if the cut landed in the middle of a
// sentence ("a force that will" / "Basically, propel it"), the two halves get joined back
// together, since whisper starts every piece with a capital letter
fn join_pieces(lines: &mut Vec<Segment>, mut new_lines: Vec<Segment>) {
    if let Some(last) = lines.last_mut() {
        if !new_lines.is_empty() && !ends_sentence(&last.text) {
            let first = new_lines.remove(0);
            last.text = format!("{} {}", last.text, lowercase_first_word(&first.text));
            last.end_ms = first.end_ms;
        }
    }
    lines.extend(new_lines);
}

// "Basically, propel" -> "basically, propel", but leaves "I", "Q2" and "DNA" alone
fn lowercase_first_word(text: &str) -> String {
    let first_word = text.split_whitespace().next().unwrap_or("");
    let mut letters = first_word.chars().filter(|c| c.is_alphabetic());
    let looks_normal = letters.next().is_some_and(|c| c.is_uppercase())
        && first_word.chars().skip(1).all(|c| !c.is_uppercase())
        && first_word.chars().filter(|c| c.is_alphabetic()).count() > 1
        && !first_word.starts_with("I'");

    if !looks_normal {
        return text.to_string();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

// finds the quietest 20ms in the last few seconds of the chunk and cuts there
fn best_cut(audio: &[f32]) -> usize {
    let frame_length = WHISPER_SAMPLE_RATE as usize / 50;
    let search_start = audio.len().saturating_sub(CUT_SEARCH_SECONDS * WHISPER_SAMPLE_RATE as usize);

    let mut quietest = audio.len();
    let mut quietest_level = f32::MAX;
    let mut frame_start = search_start;
    while frame_start + frame_length <= audio.len() {
        let frame = &audio[frame_start..frame_start + frame_length];
        let level = frame.iter().map(|s| s * s).sum::<f32>();
        if level < quietest_level {
            quietest_level = level;
            // cut in the middle of the quiet frame
            quietest = frame_start + frame_length / 2;
        }
        frame_start += frame_length;
    }
    quietest
}

fn samples_to_ms(samples: usize) -> i64 {
    (samples as i64 * 1000) / WHISPER_SAMPLE_RATE as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_at_the_quiet_spot_near_the_end() {
        let rate = WHISPER_SAMPLE_RATE as usize;
        // a minute of "speech" with a short silence 2 seconds before the end
        let mut audio: Vec<f32> = (0..rate * 60).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        for sample in &mut audio[rate * 58..rate * 58 + rate / 4] {
            *sample = 0.0;
        }

        let cut = best_cut(&audio);
        assert!(cut >= rate * 58 && cut <= rate * 58 + rate / 4);
    }

    fn line(start_ms: i64, text: &str) -> Segment {
        Segment { start_ms, end_ms: start_ms + 2000, text: text.to_string(), ..Default::default() }
    }

    #[test]
    fn joins_a_sentence_split_between_pieces() {
        let mut lines = vec![line(0, "Q1 is going to feel a force that will")];
        join_pieces(&mut lines, vec![line(55_000, "Basically, propel it towards Q2."), line(58_000, "And Q2 will too.")]);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Q1 is going to feel a force that will basically, propel it towards Q2.");
        assert_eq!(lines[0].end_ms, 57_000);
    }

    #[test]
    fn leaves_finished_sentences_alone() {
        let mut lines = vec![line(0, "That's the first part.")];
        join_pieces(&mut lines, vec![line(55_000, "Now the second part.")]);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn keeps_capitals_that_matter() {
        assert_eq!(lowercase_first_word("I think so"), "I think so");
        assert_eq!(lowercase_first_word("I'm sure"), "I'm sure");
        assert_eq!(lowercase_first_word("Q2 feels it"), "Q2 feels it");
        assert_eq!(lowercase_first_word("DNA strands"), "DNA strands");
        assert_eq!(lowercase_first_word("Basically, yes"), "basically, yes");
    }

    #[test]
    fn converts_samples_to_milliseconds() {
        assert_eq!(samples_to_ms(16_000), 1000);
        assert_eq!(samples_to_ms(16_000 * 90), 90_000);
    }
}
