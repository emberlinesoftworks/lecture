use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

type SharedWriter = Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>;

// cpal::Stream isn't Send on macOS, so the stream lives on its own thread
// and we only keep a way to stop it.
pub struct Recording {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<Result<PathBuf, String>>,
}

impl Recording {
    pub fn start(path: PathBuf) -> Result<Self, String> {
        let (stop, stop_rx) = mpsc::channel();
        let (ready_tx, ready) = mpsc::channel();
        let thread = thread::spawn(move || run(path, stop_rx, ready_tx));

        match ready.recv() {
            Ok(Ok(())) => Ok(Self { stop, thread }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("recording thread exited early".into()),
        }
    }

    pub fn stop(self) -> Result<PathBuf, String> {
        let _ = self.stop.send(());
        self.thread
            .join()
            .map_err(|_| "recording thread panicked".to_string())?
    }
}

fn run(
    path: PathBuf,
    stop: mpsc::Receiver<()>,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<PathBuf, String> {
    let (stream, writer) = match open_mic(&path) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(()));

    let _ = stop.recv();
    drop(stream);

    if let Some(w) = writer.lock().map_err(|e| e.to_string())?.take() {
        w.finalize().map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn open_mic(path: &PathBuf) -> Result<(cpal::Stream, SharedWriter), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone found")?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;

    // mono, mic's native rate (whisper wants 16k, resample later)
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: config.sample_rate.0,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let wav = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let writer: SharedWriter = Arc::new(Mutex::new(Some(wav)));

    let w = writer.clone();
    let err_fn = |e| eprintln!("stream error: {e}");
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &_| write_mono(&w, data, channels, |s| s),
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &_| write_mono(&w, data, channels, |s| s as f32 / 32768.0),
            err_fn,
            None,
        ),
        other => return Err(format!("Unsupported sample format: {other:?}")),
    }
    .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, writer))
}

fn write_mono<T: Copy>(writer: &SharedWriter, data: &[T], channels: usize, to_f32: impl Fn(T) -> f32) {
    let Ok(mut guard) = writer.lock() else { return };
    let Some(w) = guard.as_mut() else { return };

    for frame in data.chunks(channels) {
        let avg = frame.iter().map(|&s| to_f32(s)).sum::<f32>() / channels as f32;
        let _ = w.write_sample((avg.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
}
