use std::path::Path;

use serde::Serialize;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const WHISPER_SAMPLE_RATE: u32 = 16_000;

#[derive(Serialize, Clone)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

pub fn transcribe(model_path: &Path, wav_path: &Path) -> Result<Vec<Segment>, String> {
    let audio = load_audio(wav_path)?;

    // load the model (takes about a second)
    let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| format!("couldn't load model: {e}"))?;
    let mut state = context.create_state().map_err(|e| e.to_string())?;

    // same settings we tested with whisper-cli: english, beam search of 5
    let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
    params.set_language(Some("en"));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);

    state.full(params, &audio).map_err(|e| format!("transcription failed: {e}"))?;

    let mut segments = Vec::new();
    for segment in state.as_iter() {
        let text = segment.to_str_lossy().map_err(|e| e.to_string())?;
        // whisper gives timestamps in centiseconds, convert to milliseconds
        segments.push(Segment {
            start_ms: segment.start_timestamp() * 10,
            end_ms: segment.end_timestamp() * 10,
            text: text.trim().to_string(),
        });
    }
    Ok(segments)
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
