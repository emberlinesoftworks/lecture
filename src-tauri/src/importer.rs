// turns a video or audio file you drag in into a lecture recording: the sound only,
// mono at 16 kHz like everything the mic records. the file you dropped is only read, never changed

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::lectures::LectureSummary;
use crate::recorder::{Downsampler, SAVE_RATE};

// two recordings within this much of each other's length might be the same one
// (a video's listed length and the sound read out of it can be a fraction of a second apart)
const SAME_LENGTH_MS: i64 = 1500;
// how much of the start of a recording is compared
const COMPARE_SECONDS: usize = 30;
// the loudness is measured every tenth of a second
const STEP: usize = SAVE_RATE as usize / 10;
// how well the loudness has to line up to be the same recording (1.0 is perfect)
const SAME_SOUND: f32 = 0.9;

// how long a file's sound is, from what the file says about itself (without reading all of it)
pub fn sound_length_ms(source: &Path) -> Option<i64> {
    let file = File::open(source).ok()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe().format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let track = probed.format.tracks().iter().find(|track| track.codec_params.sample_rate.is_some())?;
    let frames = track.codec_params.n_frames?;
    let rate = track.codec_params.sample_rate? as i64;
    Some(frames as i64 * 1000 / rate)
}

// a lecture you already have with the same sound, whatever either file is called:
// one about the same length whose first 30 seconds get louder and quieter in the same places.
// that still matches after a rename, a copy, or the same class saved as a different kind of file
pub fn already_have<'a>(lectures: &'a [LectureSummary], source: &Path) -> Option<&'a LectureSummary> {
    let length = sound_length_ms(source)?;
    let close: Vec<&LectureSummary> = lectures.iter().filter(|lecture| (lecture.duration_ms - length).abs() <= SAME_LENGTH_MS).collect();
    if close.is_empty() {
        return None;
    }
    let dropped = loudness(&first_seconds(source).ok()?);
    close.into_iter().find(|lecture| {
        let Ok(reader) = hound::WavReader::open(&lecture.path) else { return false };
        let samples: Vec<f32> = reader
            .into_samples::<i16>()
            .take(COMPARE_SECONDS * SAVE_RATE as usize)
            .filter_map(Result::ok)
            .map(|sample| sample as f32 / i16::MAX as f32)
            .collect();
        same_sound(&dropped, &loudness(&samples))
    })
}

// how well a file's first 30 seconds line up with a lecture's recording, 0 to 1.
// only used by the duplicate_check example, to see real numbers
pub fn how_alike(source: &Path, wav_path: &Path) -> Option<f32> {
    let dropped = loudness(&first_seconds(source).ok()?);
    let reader = hound::WavReader::open(wav_path).ok()?;
    let samples: Vec<f32> = reader.into_samples::<i16>().take(COMPARE_SECONDS * SAVE_RATE as usize).filter_map(Result::ok).map(|s| s as f32 / i16::MAX as f32).collect();
    let theirs = loudness(&samples);
    (-5..=5isize)
        .map(|shift| {
            let (a, b) = if shift >= 0 { (&dropped[(shift as usize).min(dropped.len())..], &theirs[..]) } else { (&dropped[..], &theirs[((-shift) as usize).min(theirs.len())..]) };
            let length = a.len().min(b.len());
            if length < 30 { 0.0 } else { alike(&a[..length], &b[..length]) }
        })
        .reduce(f32::max)
}

// how loud each tenth of a second is
fn loudness(samples: &[f32]) -> Vec<f32> {
    samples.chunks_exact(STEP).map(|step| (step.iter().map(|s| s * s).sum::<f32>() / STEP as f32).sqrt()).collect()
}

// whether two loudness patterns line up. a re-saved file can start a hair earlier or later,
// so it tries shifting one by up to half a second either way
fn same_sound(a: &[f32], b: &[f32]) -> bool {
    (-5..=5).any(|shift: isize| {
        let (a, b) = if shift >= 0 { (&a[(shift as usize).min(a.len())..], b) } else { (a, &b[((-shift) as usize).min(b.len())..]) };
        let length = a.len().min(b.len());
        // at least a few seconds to go on
        length >= 30 && alike(&a[..length], &b[..length]) >= SAME_SOUND
    })
}

// how closely two lists rise and fall together (correlation, 1.0 is the same shape)
fn alike(a: &[f32], b: &[f32]) -> f32 {
    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    let (mean_a, mean_b) = (mean(a), mean(b));
    let mut together = 0.0;
    let (mut spread_a, mut spread_b) = (0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        together += (x - mean_a) * (y - mean_b);
        spread_a += (x - mean_a).powi(2);
        spread_b += (y - mean_b).powi(2);
    }
    // silence has no shape to compare
    if spread_a < 1e-9 || spread_b < 1e-9 {
        return 0.0;
    }
    together / (spread_a * spread_b).sqrt()
}

// brings one file into a folder as a lecture, dated the day you add it, so it shows up at the top of
// your list. (a file's own date was tried first: a professor's posted video came out as January 2019,
// the day it was exported.) a second later if another lecture already has that time, since a lecture's
// time is also its file name. returns the new recording and how long it is
pub fn import_into(source: &Path, folder: &Path, taken: &std::collections::HashSet<u64>) -> Result<(std::path::PathBuf, i64), String> {
    if !source.is_file() {
        return Err("Couldn't open that file".into());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut stamp = now;
    while taken.contains(&stamp) || folder.join(format!("lecture-{stamp}.wav")).exists() {
        stamp += 1;
    }

    // written under another name first, so a half-imported file never shows up as a lecture
    let wav_path = folder.join(format!("lecture-{stamp}.wav"));
    let partial = folder.join(format!("lecture-{stamp}.importing"));
    match sound_to_wav(source, &partial) {
        Ok(length_ms) => {
            std::fs::rename(&partial, &wav_path).map_err(|e| e.to_string())?;
            Ok((wav_path, length_ms))
        }
        Err(error) => {
            std::fs::remove_file(&partial).ok();
            Err(error)
        }
    }
}

// how long the sound was, in ms
pub fn sound_to_wav(source: &Path, wav_path: &Path) -> Result<i64, String> {
    let spec = hound::WavSpec { channels: 1, sample_rate: SAVE_RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(wav_path, spec).map_err(|e| e.to_string())?;
    let mut written: i64 = 0;
    let mut failed = None;
    read_sound(source, |sample| {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        if let Err(e) = writer.write_sample(value) {
            failed = Some(e.to_string());
            return false;
        }
        written += 1;
        true
    })?;
    if let Some(error) = failed {
        return Err(error);
    }
    writer.finalize().map_err(|e| e.to_string())?;
    if written == 0 {
        let name = source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(format!("{name} has no sound"));
    }
    Ok(written * 1000 / SAVE_RATE as i64)
}

// just the first 30 seconds, for comparing
fn first_seconds(source: &Path) -> Result<Vec<f32>, String> {
    let wanted = COMPARE_SECONDS * SAVE_RATE as usize;
    let mut samples = Vec::with_capacity(wanted);
    read_sound(source, |sample| {
        samples.push(sample);
        samples.len() < wanted
    })?;
    Ok(samples)
}

// reads a file's sound as mono at 16 kHz, handing over one sample at a time until
// each_sample says to stop (returns false)
fn read_sound(source: &Path, mut each_sample: impl FnMut(f32) -> bool) -> Result<(), String> {
    let name = source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let cant_read = || format!("Lecture can't read {name}. Try an mp4, mov, m4a, mp3, wav or flac file");

    let file = File::open(source).map_err(|e| format!("Couldn't open {name}: {e}"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|_| cant_read())?;
    let mut format = probed.format;

    // a video has a picture track too, the sound is the one with a sample rate
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.sample_rate.is_some())
        .ok_or_else(|| format!("{name} has no sound"))?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(SAVE_RATE);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| cant_read())?;
    let mut downsampler = Downsampler::new(rate, SAVE_RATE);
    let mut samples: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            // the end of the file
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(_) => return Err(cant_read()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // one bad packet in a long video isn't worth giving up over
            Err(Error::DecodeError(_)) => continue,
            Err(_) => return Err(cant_read()),
        };
        let channels = decoded.spec().channels.count().max(1);
        let buffer = samples.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, *decoded.spec()));
        if buffer.capacity() < decoded.capacity() * channels {
            *buffer = SampleBuffer::new(decoded.capacity() as u64, *decoded.spec());
        }
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            if let Some(sample) = downsampler.push(mono) {
                if !each_sample(sample) {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // test files are made with ffmpeg when it's installed, and skipped when it isn't
    fn make(folder: &Path, name: &str, args: &[&str]) -> Option<std::path::PathBuf> {
        let path = folder.join(name);
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(args)
            .arg(&path)
            .status()
            .ok()?;
        made.success().then_some(path)
    }

    #[test]
    fn videos_and_audio_files_become_recordings() {
        let folder = std::env::temp_dir().join(format!("lecture-import-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        // 5 seconds of a 440 Hz tone in stereo at 44.1k, with a picture for the videos
        let tone = ["-f", "lavfi", "-i", "sine=frequency=440:duration=5:sample_rate=44100", "-ac", "2"];
        let picture = ["-f", "lavfi", "-i", "color=c=black:s=320x240:d=5"];
        let files = [
            ("talk.mp4", [&picture[..], &tone[..], &["-c:v", "libx264", "-c:a", "aac", "-shortest"]].concat()),
            ("talk.mov", [&picture[..], &tone[..], &["-c:v", "libx264", "-c:a", "aac", "-shortest"]].concat()),
            ("talk.m4a", [&tone[..], &["-c:a", "aac"]].concat()),
            ("talk.mp3", [&tone[..], &["-c:a", "libmp3lame"]].concat()),
            ("talk.wav", tone.to_vec()),
            ("talk.flac", tone.to_vec()),
        ];
        let mut tried = 0;
        for (name, args) in files {
            let Some(source) = make(&folder, name, &args) else { continue };
            tried += 1;
            let wav = folder.join(format!("{name}.wav"));
            let length_ms = sound_to_wav(&source, &wav).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!((4800..=5200).contains(&length_ms), "{name} came out {length_ms} ms");
            let reader = hound::WavReader::open(&wav).unwrap();
            assert_eq!((reader.spec().channels, reader.spec().sample_rate), (1, 16_000), "{name}");
            // the tone is still there, not silence (ffmpeg's test tone peaks around 4,000)
            let loudest = reader.into_samples::<i16>().map(|s| s.unwrap().unsigned_abs()).max().unwrap();
            assert!(loudest > 2000, "{name} is too quiet: {loudest}");
        }

        // into a folder: named by the file's date, never on top of another lecture, nothing left behind on failure
        if let Some(source) = make(&folder, "class.m4a", &[&tone[..], &["-c:a", "aac"]].concat()) {
            let lectures = folder.join("lectures");
            std::fs::create_dir_all(&lectures).unwrap();
            // dated now, and a second later when that time is already a lecture
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let taken: std::collections::HashSet<u64> = (now..now + 5).collect();
            let (first, length_ms) = import_into(&source, &lectures, &taken).unwrap();
            assert!((4800..=5200).contains(&length_ms));
            let stamp = |path: &std::path::Path| path.file_stem().unwrap().to_string_lossy().trim_start_matches("lecture-").parse::<u64>().unwrap();
            assert!((now + 5..now + 10).contains(&stamp(&first)), "{}", first.display());
            let (second, _) = import_into(&source, &lectures, &taken).unwrap();
            assert_eq!(stamp(&second), stamp(&first) + 1);

            let text = folder.join("broken.mp4");
            std::fs::write(&text, "not really a video").unwrap();
            assert!(import_into(&text, &lectures, &taken).is_err());
            let leftovers = std::fs::read_dir(&lectures).unwrap().filter(|entry| entry.as_ref().unwrap().path().extension().is_some_and(|e| e == "importing")).count();
            assert_eq!(leftovers, 0);
        }

        // the same class under another name, or saved as another kind of file, is noticed.
        // a different class of the same length isn't. (these sound like speech: a tone that
        // keeps getting louder and quieter, a different pattern for each "class")
        let talking = |pattern: &str| format!("aevalsrc='sin(300*2*PI*t)*abs(sin({pattern}*t)*sin(0.37*t))':s=44100:d=40");
        let picture = ["-f", "lavfi", "-i", "color=c=black:s=320x240:d=40"];
        let class = folder.join("Math172 6.1.V1.mp4");
        if let Some(original) = make(&folder, "Math172 6.1.V1.mp4", &[&picture[..], &["-f", "lavfi", "-i", &talking("1.3"), "-c:v", "libx264", "-c:a", "aac", "-shortest"]].concat()) {
            let lectures_folder = folder.join("have");
            std::fs::create_dir_all(&lectures_folder).unwrap();
            let (imported, length_ms) = import_into(&original, &lectures_folder, &Default::default()).unwrap();
            let have = [LectureSummary {
                path: imported.display().to_string(),
                title: Some("whatever you renamed it to".into()),
                folder: None,
                recorded_at: 0,
                duration_ms: length_ms,
                preview: String::new(),
                mark_count: 0,
            }];
            let copy = folder.join("math172_6.1.V1 copy.mp4");
            std::fs::copy(&class, &copy).unwrap();
            assert!(already_have(&have, &copy).is_some(), "a renamed copy");
            if let Some(as_mp3) = make(&folder, "calc video.mp3", &["-i", class.to_str().unwrap(), "-vn", "-c:a", "libmp3lame"]) {
                assert!(already_have(&have, &as_mp3).is_some(), "the same class as an mp3");
            }
            if let Some(other) = make(&folder, "Math172 6.2.mp4", &[&picture[..], &["-f", "lavfi", "-i", &talking("2.9"), "-c:v", "libx264", "-c:a", "aac", "-shortest"]].concat()) {
                assert!(already_have(&have, &other).is_none(), "a different class the same length");
            }
        }

        // a video with no sound, and a file that isn't media at all
        if let Some(silent) = make(&folder, "no-sound.mp4", &[&picture[..], &["-c:v", "libx264"]].concat()) {
            assert_eq!(sound_to_wav(&silent, &folder.join("x.wav")).unwrap_err(), "no-sound.mp4 has no sound");
        }
        let text = folder.join("notes.txt");
        std::fs::write(&text, "not a video").unwrap();
        assert!(sound_to_wav(&text, &folder.join("y.wav")).unwrap_err().starts_with("Lecture can't read notes.txt"));

        std::fs::remove_dir_all(&folder).ok();
        if tried == 0 {
            eprintln!("ffmpeg isn't installed, so the import test only checked the error messages");
        }
    }
}
