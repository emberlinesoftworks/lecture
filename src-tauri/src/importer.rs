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

use crate::recorder::{Downsampler, SAVE_RATE};

// brings one file into a folder as a lecture. it's dated when the file was made (so a video of
// last Tuesday's class sorts with last Tuesday), moved a second later if another lecture already
// has that time, since a lecture's time is also its file name. returns the new recording and how long it is
pub fn import_into(source: &Path, folder: &Path, taken: &std::collections::HashSet<u64>) -> Result<(std::path::PathBuf, i64), String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let metadata = std::fs::metadata(source).map_err(|e| format!("Couldn't open that file: {e}"))?;
    let made = metadata
        .created()
        .or_else(|_| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .filter(|&seconds| seconds > 0 && seconds <= now)
        .unwrap_or(now);
    let mut stamp = made;
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

    let spec = hound::WavSpec { channels: 1, sample_rate: SAVE_RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(wav_path, spec).map_err(|e| e.to_string())?;
    let mut downsampler = Downsampler::new(rate, SAVE_RATE);
    let mut written: i64 = 0;
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
                let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                writer.write_sample(value).map_err(|e| e.to_string())?;
                written += 1;
            }
        }
    }
    writer.finalize().map_err(|e| e.to_string())?;
    if written == 0 {
        return Err(format!("{name} has no sound"));
    }
    Ok(written * 1000 / SAVE_RATE as i64)
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
            let made = std::fs::metadata(&source).unwrap().created().or_else(|_| std::fs::metadata(&source).unwrap().modified()).unwrap();
            let made = made.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            let taken: std::collections::HashSet<u64> = [made].into();
            let (first, length_ms) = import_into(&source, &lectures, &taken).unwrap();
            assert!((4800..=5200).contains(&length_ms));
            assert_eq!(first.file_name().unwrap().to_string_lossy(), format!("lecture-{}.wav", made + 1));
            let (second, _) = import_into(&source, &lectures, &taken).unwrap();
            assert_eq!(second.file_name().unwrap().to_string_lossy(), format!("lecture-{}.wav", made + 2));

            let text = folder.join("broken.mp4");
            std::fs::write(&text, "not really a video").unwrap();
            assert!(import_into(&text, &lectures, &taken).is_err());
            let leftovers = std::fs::read_dir(&lectures).unwrap().filter(|entry| entry.as_ref().unwrap().path().extension().is_some_and(|e| e == "importing")).count();
            assert_eq!(leftovers, 0);
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
