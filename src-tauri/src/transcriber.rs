use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

use crate::silence::{original_ms, squeeze_silence};

pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

// the voice detector that finds where people are talking (about 1 MB).
// it sits next to the speech model, and if it's missing we just transcribe everything
pub const VOICE_MODEL_FILE: &str = "ggml-silero-v5.1.2.bin";

// a silence this long between words starts a new line
const PAUSE_BREAK_MS: i64 = 500;
// a silence this long starts a new line no matter what
const LONG_PAUSE_MS: i64 = 1500;
// lines longer than this get broken at the next word
const MAX_LINE_MS: i64 = 12_000;

#[derive(Serialize, Deserialize, Clone)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    // how long it was quiet right before this line started
    // (older transcripts don't have this, so it defaults to 0)
    #[serde(default)]
    pub pause_before_ms: i64,
}

// transcribes a whole recording from its file
pub fn transcribe(model_path: &Path, wav_path: &Path) -> Result<Vec<Segment>, String> {
    let audio = load_audio(wav_path)?;
    Transcriber::load(model_path)?.transcribe_audio(&audio, 0)
}

// keeps the model loaded so it can transcribe piece after piece during a class
pub struct Transcriber {
    context: WhisperContext,
    voice_detector: Option<Mutex<WhisperVadContext>>,
}

impl Transcriber {
    // loading the model takes about a second
    pub fn load(model_path: &Path) -> Result<Self, String> {
        let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|e| format!("couldn't load model: {e}"))?;
        let voice_detector = load_voice_detector(&model_path.with_file_name(VOICE_MODEL_FILE));
        Ok(Self { context, voice_detector })
    }

    // turns 16k mono audio into lines. start_ms is where this audio
    // begins in the recording, so the times line up with the whole file
    pub fn transcribe_audio(&self, audio: &[f32], start_ms: i64) -> Result<Vec<Segment>, String> {
        let mut lines = transcribe_audio(&self.context, self.voice_detector.as_ref(), audio)?;
        for line in &mut lines {
            line.start_ms += start_ms;
            line.end_ms += start_ms;
        }
        Ok(lines)
    }
}

fn load_voice_detector(path: &Path) -> Option<Mutex<WhisperVadContext>> {
    let path = path.to_str()?;
    if !Path::new(path).exists() {
        return None;
    }
    let mut params = WhisperVadContextParams::new();
    params.set_n_threads(2);
    // it's tiny, the cpu is plenty and leaves the gpu for whisper
    params.set_use_gpu(false);
    WhisperVadContext::new(path, params).ok().map(Mutex::new)
}

// where people are talking, as (start, end) sample positions.
// None means we couldn't tell, so transcribe all of it
fn find_speech(voice_detector: &Mutex<WhisperVadContext>, audio: &[f32]) -> Option<Vec<(usize, usize)>> {
    let mut params = WhisperVadParams::new();
    params.set_threshold(0.5);
    params.set_min_speech_duration(250);
    params.set_min_silence_duration(500);
    // a bit of room around each bit of talking so first and last words don't get clipped
    params.set_speech_pad(200);

    let mut detector = voice_detector.lock().ok()?;
    let segments = detector.segments_from_samples(params, audio).ok()?;

    // the detector gives centiseconds, 160 samples each at 16k
    let samples_per_cs = WHISPER_SAMPLE_RATE as f32 / 100.0;
    let speech = segments
        .map(|segment| ((segment.start * samples_per_cs) as usize, (segment.end * samples_per_cs) as usize))
        .collect();
    Some(speech)
}

fn transcribe_audio(
    context: &WhisperContext,
    voice_detector: Option<&Mutex<WhisperVadContext>>,
    audio: &[f32],
) -> Result<Vec<Segment>, String> {
    // cut long silences down before whisper sees the audio.
    // the times get mapped back to the real recording below
    let speech = voice_detector.and_then(|detector| find_speech(detector, audio));
    let (squeezed, pieces) = match &speech {
        Some(speech) if speech.is_empty() => return Ok(Vec::new()),  // nobody talking, nothing to do
        Some(speech) => squeeze_silence(audio, speech),
        None => (audio.to_vec(), Vec::new()),
    };
    let to_recording_time = |ms: i64| if pieces.is_empty() { ms } else { original_ms(&pieces, ms) };

    let mut state = context.create_state().map_err(|e| e.to_string())?;

    // same settings we tested with whisper-cli: english, beam search of 5
    let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
    params.set_language(Some("en"));
    params.set_token_timestamps(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);

    state.full(params, &squeezed).map_err(|e| format!("transcription failed: {e}"))?;

    // collect every word piece with its timing
    let mut tokens = Vec::new();
    for segment in state.as_iter() {
        for i in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(i) else { continue };
            let text = token.to_str_lossy().map_err(|e| e.to_string())?.to_string();

            // skip whisper's special tokens like [_BEG_] and [_TT_150]
            if text.starts_with("[_") || text.starts_with("<|") {
                continue;
            }

            // whisper gives timestamps in centiseconds, convert to milliseconds
            let timing = token.token_data();
            tokens.push(Token {
                text,
                start_ms: to_recording_time(timing.t0 * 10),
                end_ms: to_recording_time(timing.t1 * 10),
            });
        }
    }

    let mut lines = build_lines(&tokens);

    // whisper's word timings stretch over pauses, and a line after a pause often
    // gets a start time from the beginning of the silence instead of the first word.
    // so look at the audio itself: find the real pause and start the line after it
    let quiet_frames = find_quiet_frames(audio);
    // (the first line counts too, since during class this audio is a piece cut
    // at a quiet spot, and that pause matters for marks)
    for i in 0..lines.len() {
        let previous_start = if i == 0 { -1000 } else { lines[i - 1].start_ms };
        let next_start = lines.get(i + 1).map(|line| line.start_ms).unwrap_or(i64::MAX);

        if let Some((pause_start, pause_end)) = longest_pause_near(&quiet_frames, previous_start, lines[i].start_ms) {
            lines[i].pause_before_ms = pause_end - pause_start;
            // never move a line past where the next one starts
            if pause_end > lines[i].start_ms && pause_end < next_start {
                lines[i].start_ms = pause_end;
            }
        }
    }

    // whisper sometimes "hears" a few words in silence (the classic one is "Thank you.").
    // a short line that starts in silence was made up, so drop it
    lines.retain(|line| !(word_count(&line.text) <= 3 && mostly_quiet(&quiet_frames, line.start_ms, 800)));

    Ok(lines)
}

const FRAME_MS: usize = 20;

// splits the audio into 20ms frames and marks which ones are quiet,
// compared to how loud this recording's speech and background noise are
fn find_quiet_frames(audio: &[f32]) -> Vec<bool> {
    let frame_length = WHISPER_SAMPLE_RATE as usize * FRAME_MS / 1000;
    let loudness: Vec<f32> = audio
        .chunks(frame_length)
        .map(|frame| (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt())
        .collect();
    if loudness.is_empty() {
        return Vec::new();
    }

    let mut sorted = loudness.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let background = sorted[sorted.len() / 10];
    let speech = sorted[sorted.len() * 9 / 10];
    let threshold = background + (speech - background) * 0.1;

    loudness.iter().map(|&level| level < threshold).collect()
}

// longest quiet stretch around where whisper thinks a line starts: from up to two seconds
// before it to a second and a half after (but after the previous line started, so a short line
// doesn't borrow its pause). returns (start, end) in ms, if it's at least 300ms long
fn longest_pause_near(quiet_frames: &[bool], previous_start_ms: i64, start_ms: i64) -> Option<(i64, i64)> {
    let window_start = (start_ms - 2000).max(previous_start_ms + 300).max(0) as usize / FRAME_MS;
    let window_end = ((start_ms + 1500).max(0) as usize / FRAME_MS).min(quiet_frames.len());

    let mut best: Option<(usize, usize)> = None;
    let mut run_start = None;
    for frame in window_start..=window_end {
        let quiet = frame < window_end && quiet_frames[frame];
        match (quiet, run_start) {
            (true, None) => run_start = Some(frame),
            (false, Some(start)) => {
                if best.map_or(true, |(a, b)| frame - start > b - a) {
                    best = Some((start, frame));
                }
                run_start = None;
            }
            _ => {}
        }
    }

    let (start, end) = best?;
    let length_ms = (end - start) * FRAME_MS;
    (length_ms >= 300).then(|| ((start * FRAME_MS) as i64, (end * FRAME_MS) as i64))
}

struct Token {
    text: String,
    start_ms: i64,
    end_ms: i64,
}

// whisper's own chunks can cut a sentence in half ("every cell" / "of your body"),
// so rebuild the lines from each word's timing. a new line starts at the end of a
// sentence, or at a pause, since whisper sometimes leaves out all the punctuation
fn build_lines(tokens: &[Token]) -> Vec<Segment> {
    let has_punctuation = tokens.iter().any(|token| ends_sentence(&token.text));

    let mut lines = Vec::new();
    let mut text = String::new();
    let mut start_ms = None;
    let mut end_ms = 0;
    let mut sentence_ended = true;
    let mut starts_sentence = true;

    for token in tokens {
        // only break before a new word (tokens can be pieces of words),
        // which also keeps numbers like 3.5 from splitting at the dot
        if token.text.starts_with(' ') {
            if let Some(line_start) = start_ms {
                let pause = token.start_ms - end_ms;
                let words_so_far = word_count(&text);
                let too_long = token.start_ms - line_start > MAX_LINE_MS;

                // a short pause only ends a line if the line already has a few words,
                // otherwise we get lines like "A distance" / "Of 3 meters"
                let pause_break = (pause >= PAUSE_BREAK_MS && words_so_far >= 4) || pause >= LONG_PAUSE_MS;

                if sentence_ended || pause_break || too_long {
                    lines.push(make_line(&text, start_ms.take().unwrap(), end_ms, starts_sentence || !has_punctuation));
                    text.clear();
                    starts_sentence = sentence_ended;
                }
            }
        }

        start_ms.get_or_insert(token.start_ms);
        end_ms = token.end_ms;
        text.push_str(&token.text);
        sentence_ended = ends_sentence(&token.text);
    }

    if let Some(start_ms) = start_ms {
        lines.push(make_line(&text, start_ms, end_ms, starts_sentence || !has_punctuation));
    }
    lines.retain(|line| !line.text.is_empty());
    lines
}

fn make_line(text: &str, start_ms: i64, end_ms: i64, capitalize_it: bool) -> Segment {
    let text = text.trim();
    Segment {
        start_ms,
        end_ms,
        // only capitalize real sentence starts, not the second half of a split sentence
        text: if capitalize_it { capitalize(text) } else { text.to_string() },
        pause_before_ms: 0,
    }
}

pub(crate) fn ends_sentence(text: &str) -> bool {
    let text = text.trim_end();
    text.ends_with('.') || text.ends_with('?') || text.ends_with('!')
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

// whisper occasionally writes everything in lowercase
fn capitalize(line: &str) -> String {
    let mut chars = line.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// read the WAV we recorded and turn it into 16k mono floats for whisper
fn load_audio(wav_path: &Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(wav_path).map_err(|e| format!("couldn't open recording: {e}"))?;
    let sample_rate = reader.spec().sample_rate;
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|sample| sample.map(|s| s as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    Ok(downsample(&samples, sample_rate, WHISPER_SAMPLE_RATE))
}

// averages each group of input samples into one output sample,
// which also smooths out the high frequencies whisper doesn't use
fn downsample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate <= to_rate {
        return samples.to_vec();
    }

    let step = from_rate as f64 / to_rate as f64;
    let output_length = (samples.len() as f64 / step) as usize;
    let mut output = Vec::with_capacity(output_length);

    for i in 0..output_length {
        let start = (i as f64 * step) as usize;
        let end = (((i + 1) as f64 * step) as usize).min(samples.len());
        let group = &samples[start..end.max(start + 1)];
        output.push(group.iter().sum::<f32>() / group.len() as f32);
    }
    output
}

// true if most of the audio right after start_ms is silence
fn mostly_quiet(quiet_frames: &[bool], start_ms: i64, length_ms: i64) -> bool {
    let first = start_ms.max(0) as usize / FRAME_MS;
    let last = ((start_ms + length_ms).max(0) as usize / FRAME_MS).min(quiet_frames.len());
    let Some(frames) = quiet_frames.get(first..last) else { return false };
    if frames.is_empty() {
        return false;
    }
    frames.iter().filter(|&&quiet| quiet).count() * 10 >= frames.len() * 8
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2s of "speech", 1.5s of silence, then 2s more speech
    fn speech_pause_speech() -> Vec<f32> {
        let rate = WHISPER_SAMPLE_RATE as usize;
        let mut audio = Vec::new();
        audio.extend((0..rate * 2).map(|i| (i as f32 * 0.05).sin() * 0.3));
        audio.extend(std::iter::repeat(0.001).take(rate * 3 / 2));
        audio.extend((0..rate * 2).map(|i| (i as f32 * 0.05).sin() * 0.3));
        audio
    }

    #[test]
    fn finds_the_pause_when_whisper_starts_the_line_too_early() {
        let quiet_frames = find_quiet_frames(&speech_pause_speech());
        // whisper says the line starts at 2.1s, really the words start at 3.5s
        let (pause_start, pause_end) = longest_pause_near(&quiet_frames, 0, 2100).unwrap();
        assert!(pause_end - pause_start >= 1300);
        assert!((pause_end - 3500).abs() <= 40);
    }

    #[test]
    fn finds_the_pause_when_whisper_starts_the_line_on_time() {
        let quiet_frames = find_quiet_frames(&speech_pause_speech());
        let (pause_start, pause_end) = longest_pause_near(&quiet_frames, 0, 3500).unwrap();
        assert!(pause_end - pause_start >= 1300);
    }

    #[test]
    fn no_pause_in_the_middle_of_speech() {
        let quiet_frames = find_quiet_frames(&speech_pause_speech());
        assert!(longest_pause_near(&quiet_frames, 4000, 5000).is_none());
    }

    fn words(list: &[(&str, i64, i64)]) -> Vec<Token> {
        list.iter()
            .map(|&(text, start_ms, end_ms)| Token { text: text.to_string(), start_ms, end_ms })
            .collect()
    }

    #[test]
    fn splits_at_sentence_ends() {
        let lines = build_lines(&words(&[(" Hello", 0, 300), (" there.", 300, 600), (" Next", 700, 900), (" one.", 900, 1200)]));
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["Hello there.", "Next one."]);
    }

    #[test]
    fn short_pause_doesnt_split_a_short_phrase() {
        // "A distance" [pause] "of 3 meters" should stay one line
        let lines = build_lines(&words(&[(" A", 0, 200), (" distance", 200, 600), (" of", 1300, 1400), (" 3", 1400, 1500), (" meters.", 1500, 1900)]));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "A distance of 3 meters.");
    }

    #[test]
    fn second_half_of_a_split_sentence_isnt_capitalized() {
        // a long pause splits it, but "to happen" isn't a new sentence
        let lines = build_lines(&words(&[(" So,", 0, 200), (" what's", 200, 400), (" going", 400, 600), (" to", 3000, 3100), (" happen?", 3100, 3500)]));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "to happen?");
    }

    #[test]
    fn no_punctuation_still_splits_at_pauses_and_capitalizes() {
        let lines = build_lines(&words(&[
            (" today", 0, 200), (" we", 200, 300), (" talk", 300, 500), (" about", 500, 700), (" cells", 700, 900),
            (" here's", 1600, 1800), (" the", 1800, 1900), (" important", 1900, 2200), (" part", 2200, 2500),
        ]));
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["Today we talk about cells", "Here's the important part"]);
    }

    #[test]
    fn made_up_words_in_silence_are_quiet() {
        let quiet_frames = find_quiet_frames(&speech_pause_speech());
        // 2.2s is in the middle of the silence, 0.5s is in the middle of speech
        assert!(mostly_quiet(&quiet_frames, 2200, 800));
        assert!(!mostly_quiet(&quiet_frames, 500, 800));
    }
}
