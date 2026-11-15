use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use crate::live::PendingAudio;

// whisper works at 16k, so there's no point saving more than that.
// it also makes recordings a third of the size (about 145 MB for 75 minutes)
const SAVE_RATE: u32 = 16_000;

struct Output {
    wav: hound::WavWriter<BufWriter<File>>,
    downsampler: Downsampler,
    // a copy of the audio for transcribing during class
    pending: Option<PendingAudio>,
}

type SharedOutput = Arc<Mutex<Option<Output>>>;

// cpal's Stream can't be sent between threads on macOS, so each recording
// runs on its own thread and we just keep a way to tell it to stop
pub struct Recording {
    stop_sender: mpsc::Sender<()>,
    recording_thread: JoinHandle<Result<PathBuf, String>>,
    started_at: Instant,
    marks: Vec<u64>,
}

impl Recording {
    // pending gets a copy of every sample, so it can be transcribed while recording
    pub fn start(path: PathBuf, pending: PendingAudio) -> Result<Self, String> {
        let (stop_sender, stop_receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::channel();
        let recording_thread = thread::spawn(move || record(path, pending, stop_receiver, ready_sender));

        // wait until the mic is actually open before saying we're recording
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                stop_sender,
                recording_thread,
                started_at: Instant::now(),
                marks: Vec::new(),
            }),
            Ok(Err(error)) => {
                let _ = recording_thread.join();
                Err(error)
            }
            Err(_) => Err("recording thread exited early".into()),
        }
    }

    // saves how far into the recording we are, in milliseconds
    pub fn add_mark(&mut self) -> u64 {
        let mark_ms = self.started_at.elapsed().as_millis() as u64;
        self.marks.push(mark_ms);
        mark_ms
    }

    pub fn stop(self) -> Result<(PathBuf, Vec<u64>), String> {
        let _ = self.stop_sender.send(());
        let path = self
            .recording_thread
            .join()
            .map_err(|_| "recording thread panicked".to_string())??;
        Ok((path, self.marks))
    }
}

fn record(
    path: PathBuf,
    pending: PendingAudio,
    stop_receiver: mpsc::Receiver<()>,
    ready_sender: mpsc::Sender<Result<(), String>>,
) -> Result<PathBuf, String> {
    let (stream, output) = match open_mic(&path, pending) {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready_sender.send(Err(error.clone()));
            return Err(error);
        }
    };
    let _ = ready_sender.send(Ok(()));

    // keep recording until stop is pressed
    let _ = stop_receiver.recv();
    drop(stream);

    // finish the file so the WAV header has the right length
    if let Some(output) = output.lock().map_err(|e| e.to_string())?.take() {
        output.wav.finalize().map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn open_mic(path: &PathBuf, pending: PendingAudio) -> Result<(cpal::Stream, SharedOutput), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone found")?;
    let supported_config = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_format = supported_config.sample_format();
    let config: cpal::StreamConfig = supported_config.into();
    let channels = config.channels as usize;

    // save as mono at 16k (or the mic's own rate if it's somehow lower)
    let mic_rate = config.sample_rate.0;
    let save_rate = mic_rate.min(SAVE_RATE);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: save_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let wav = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let output: SharedOutput = Arc::new(Mutex::new(Some(Output {
        wav,
        downsampler: Downsampler::new(mic_rate, save_rate),
        // whisper needs 16k, so a (very unusual) slower mic gets transcribed from the file instead
        pending: (save_rate == SAVE_RATE).then_some(pending),
    })));

    let stream_writer = output.clone();
    let on_error = |error| eprintln!("stream error: {error}");

    // macs usually give f32 samples, but handle i16 just in case
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |samples: &[f32], _: &_| write_mono(&stream_writer, samples, channels, |sample| sample),
            on_error,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |samples: &[i16], _: &_| {
                write_mono(&stream_writer, samples, channels, |sample| sample as f32 / 32768.0)
            },
            on_error,
            None,
        ),
        other => return Err(format!("Unsupported sample format: {other:?}")),
    }
    .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, output))
}

// average all the channels into one sample, bring it down to 16k, and write it to the file
fn write_mono<T: Copy>(
    output: &SharedOutput,
    samples: &[T],
    channels: usize,
    to_float: impl Fn(T) -> f32,
) {
    let Ok(mut guard) = output.lock() else { return };
    let Some(output) = guard.as_mut() else { return };

    let mut new_samples = Vec::with_capacity(samples.len() / channels);
    for frame in samples.chunks(channels) {
        let average = frame.iter().map(|&sample| to_float(sample)).sum::<f32>() / channels as f32;
        if let Some(sample) = output.downsampler.push(average) {
            let sample = sample.clamp(-1.0, 1.0);
            let _ = output.wav.write_sample((sample * 32767.0) as i16);
            new_samples.push(sample);
        }
    }

    // hand the new audio to the live transcriber all at once
    if let Some(pending) = &output.pending {
        if let Ok(mut pending) = pending.lock() {
            pending.extend(new_samples);
        }
    }
}

// averages groups of samples to go from the mic's rate (usually 48k) down to 16k.
// cpal hands us audio in small pieces, so it remembers where it was between them
struct Downsampler {
    step: f64,
    position: f64,
    sum: f32,
    count: u32,
}

impl Downsampler {
    fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            step: (from_rate as f64 / to_rate as f64).max(1.0),
            position: 0.0,
            sum: 0.0,
            count: 0,
        }
    }

    // returns a sample every time enough input has come in
    fn push(&mut self, sample: f32) -> Option<f32> {
        self.sum += sample;
        self.count += 1;
        self.position += 1.0;
        if self.position < self.step {
            return None;
        }

        self.position -= self.step;
        let average = self.sum / self.count as f32;
        self.sum = 0.0;
        self.count = 0;
        Some(average)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_output(from_rate: u32, input_length: usize) -> usize {
        let mut downsampler = Downsampler::new(from_rate, SAVE_RATE);
        (0..input_length).filter_map(|i| downsampler.push((i as f32 * 0.01).sin())).count()
    }

    #[test]
    fn one_second_of_48k_becomes_one_second_of_16k() {
        assert_eq!(count_output(48_000, 48_000), 16_000);
    }

    #[test]
    fn works_for_44_1k_mics_too() {
        let output = count_output(44_100, 44_100);
        assert!((output as i64 - 16_000).abs() <= 1);
    }

    #[test]
    fn a_16k_mic_is_left_alone() {
        assert_eq!(count_output(16_000, 16_000), 16_000);
    }
}
