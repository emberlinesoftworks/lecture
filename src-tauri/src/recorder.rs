use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

type SharedWriter = Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>;

// cpal's Stream can't be sent between threads on macOS, so each recording
// runs on its own thread and we just keep a way to tell it to stop
pub struct Recording {
    stop_sender: mpsc::Sender<()>,
    recording_thread: JoinHandle<Result<PathBuf, String>>,
    started_at: Instant,
    marks: Vec<u64>,
}

impl Recording {
    pub fn start(path: PathBuf) -> Result<Self, String> {
        let (stop_sender, stop_receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::channel();
        let recording_thread = thread::spawn(move || record(path, stop_receiver, ready_sender));

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
    stop_receiver: mpsc::Receiver<()>,
    ready_sender: mpsc::Sender<Result<(), String>>,
) -> Result<PathBuf, String> {
    let (stream, writer) = match open_mic(&path) {
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
    if let Some(writer) = writer.lock().map_err(|e| e.to_string())?.take() {
        writer.finalize().map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn open_mic(path: &PathBuf) -> Result<(cpal::Stream, SharedWriter), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone found")?;
    let supported_config = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_format = supported_config.sample_format();
    let config: cpal::StreamConfig = supported_config.into();
    let channels = config.channels as usize;

    // save as mono at the mic's sample rate (whisper wants 16k, we'll resample later)
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: config.sample_rate.0,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let wav_file = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let writer: SharedWriter = Arc::new(Mutex::new(Some(wav_file)));

    let stream_writer = writer.clone();
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
    Ok((stream, writer))
}

// average all the channels into one sample and write it to the file
fn write_mono<T: Copy>(
    writer: &SharedWriter,
    samples: &[T],
    channels: usize,
    to_float: impl Fn(T) -> f32,
) {
    let Ok(mut guard) = writer.lock() else { return };
    let Some(writer) = guard.as_mut() else { return };

    for frame in samples.chunks(channels) {
        let average = frame.iter().map(|&sample| to_float(sample)).sum::<f32>() / channels as f32;
        let _ = writer.write_sample((average.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
}
