// the speech models and the voice detector: which one you're using, and downloading them.
// they live in Application Support/com.emberlinesoftworks.lecture/models

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::transcriber::VOICE_MODEL_FILE;

// what people pick on the first launch screen
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Choice {
    // small.en: quick, a little less accurate
    Fast,
    // large-v3-turbo: barely slower, a lot fewer mistakes
    Accurate,
}

pub struct ModelFile {
    pub file_name: &'static str,
    pub url: &'static str,
    // from whisper.cpp's list of models, so a broken or tampered download gets caught
    pub sha1: Option<&'static str>,
}

const FAST: ModelFile = ModelFile {
    file_name: "ggml-small.en.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
    sha1: Some("db8a495a91d927739e50b3fc1cc4c6b8f6c2d022"),
};

const ACCURATE: ModelFile = ModelFile {
    file_name: "ggml-large-v3-turbo.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha1: Some("4af2b29d7ec73d781377bfd1758ca957a807e941"),
};

// the voice detector that skips silence (see silence.rs). there's no published
// checksum for it, so it's only checked for being the full size
const VOICE: ModelFile = ModelFile {
    file_name: VOICE_MODEL_FILE,
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
    sha1: None,
};

pub fn model_file(choice: Choice) -> &'static ModelFile {
    match choice {
        Choice::Fast => &FAST,
        Choice::Accurate => &ACCURATE,
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Settings {
    speech_model: Option<Choice>,
    // imports and Transcribe again without the cooling breaks (see cooling.rs). off unless you turn it on
    #[serde(default)]
    full_speed: bool,
}

fn settings_path(models_folder: &Path) -> PathBuf {
    models_folder.join("settings.json")
}

fn load_settings(models_folder: &Path) -> Settings {
    fs::read_to_string(settings_path(models_folder))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

// changes one setting and keeps the rest
fn change_settings(models_folder: &Path, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    fs::create_dir_all(models_folder).map_err(|e| e.to_string())?;
    let mut settings = load_settings(models_folder);
    change(&mut settings);
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(settings_path(models_folder), json).map_err(|e| e.to_string())
}

pub fn save_choice(models_folder: &Path, choice: Choice) -> Result<(), String> {
    change_settings(models_folder, |settings| settings.speech_model = Some(choice))
}

pub fn save_full_speed(models_folder: &Path, full_speed: bool) -> Result<(), String> {
    change_settings(models_folder, |settings| settings.full_speed = full_speed)
}

pub fn full_speed(models_folder: &Path) -> bool {
    load_settings(models_folder).full_speed
}

fn is_downloaded(models_folder: &Path, choice: Choice) -> bool {
    models_folder.join(model_file(choice).file_name).exists()
}

// Most accurate is a Pro model
pub fn allowed(choice: Choice, pro: bool) -> bool {
    pro || choice == Choice::Fast
}

// the model to use: whatever was picked, otherwise whichever one is there
// (most accurate first). without Pro it's always Fast, even if Most accurate is
// downloaded. None means nothing usable is downloaded yet
pub fn current_choice(models_folder: &Path, pro: bool) -> Option<Choice> {
    if let Some(choice) = load_settings(models_folder).speech_model {
        if allowed(choice, pro) && is_downloaded(models_folder, choice) {
            return Some(choice);
        }
    }
    [Choice::Accurate, Choice::Fast]
        .into_iter()
        .find(|&choice| allowed(choice, pro) && is_downloaded(models_folder, choice))
}

#[derive(Serialize)]
pub struct ModelStatus {
    pub current: Option<Choice>,
    pub fast_downloaded: bool,
    pub accurate_downloaded: bool,
    pub full_speed: bool,
    // false means Most accurate shows as Pro and can't be picked
    pub pro: bool,
    // shows "free during the beta" on Most accurate
    pub free_during_beta: bool,
}

pub fn status(models_folder: &Path, pro: bool) -> ModelStatus {
    ModelStatus {
        current: current_choice(models_folder, pro),
        fast_downloaded: is_downloaded(models_folder, Choice::Fast),
        accurate_downloaded: is_downloaded(models_folder, Choice::Accurate),
        full_speed: full_speed(models_folder),
        pro,
        free_during_beta: false,
    }
}

// downloads the speech model you picked and the voice detector (if they aren't there yet).
// on_progress gets (bytes so far, total bytes) across both files
pub fn download(models_folder: &Path, choice: Choice, on_progress: impl Fn(u64, u64)) -> Result<(), String> {
    fs::create_dir_all(models_folder).map_err(|e| e.to_string())?;

    let wanted: Vec<&ModelFile> = [model_file(choice), &VOICE]
        .into_iter()
        .filter(|model| !models_folder.join(model.file_name).exists())
        .collect();

    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .map_err(|e| e.to_string())?;

    // ask how big everything is first so there's one progress bar for both files
    let mut sizes = Vec::new();
    for model in &wanted {
        let response = client.head(model.url).send().map_err(|_| no_internet())?;
        // read the header itself, content_length() is always 0 for a HEAD request
        let size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        sizes.push(size);
    }
    let total: u64 = sizes.iter().sum();

    let mut done_before = 0;
    for (model, size) in wanted.iter().zip(&sizes) {
        download_file(&client, model, models_folder, |bytes| on_progress(done_before + bytes, total))?;
        done_before += size;
    }

    save_choice(models_folder, choice)
}

fn download_file(
    client: &reqwest::blocking::Client,
    model: &ModelFile,
    models_folder: &Path,
    on_progress: impl Fn(u64),
) -> Result<(), String> {
    let final_path = models_folder.join(model.file_name);
    // download next to it first, so quitting halfway never leaves a broken model behind
    let part_path = models_folder.join(format!("{}.part", model.file_name));

    let mut response = client.get(model.url).send().map_err(|_| no_internet())?;
    if !response.status().is_success() {
        return Err(format!("The download didn't start (the server said {}). Try again in a bit.", response.status()));
    }
    let expected_size = response.content_length();

    let mut file = File::create(&part_path).map_err(|e| e.to_string())?;
    let mut buffer = vec![0; 256 * 1024];
    let mut downloaded = 0;
    let mut last_reported = 0;

    loop {
        let read = response.read(&mut buffer).map_err(|_| no_internet())?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        downloaded += read as u64;

        // every couple of MB is plenty for the progress bar
        if downloaded - last_reported >= 2 * 1024 * 1024 {
            on_progress(downloaded);
            last_reported = downloaded;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    on_progress(downloaded);

    if let Some(expected_size) = expected_size {
        if downloaded != expected_size {
            let _ = fs::remove_file(&part_path);
            return Err("The download stopped partway. Check your internet and try again.".into());
        }
    }

    if let Some(expected) = model.sha1 {
        if sha1_of(&part_path)? != expected {
            let _ = fs::remove_file(&part_path);
            return Err("The download came through damaged. Try again.".into());
        }
    }

    fs::rename(&part_path, &final_path).map_err(|e| e.to_string())
}

fn no_internet() -> String {
    "Couldn't reach the download. Check your internet connection and try again.".into()
}

pub fn sha1_of(path: &Path) -> Result<String, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let mut hasher = Sha1::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn saving_one_setting_keeps_the_other() {
        let folder = std::env::temp_dir().join(format!("lecture-settings-{}", std::process::id()));
        save_full_speed(&folder, true).unwrap();
        save_choice(&folder, Choice::Fast).unwrap();
        assert!(full_speed(&folder));
        save_full_speed(&folder, false).unwrap();
        assert_eq!(load_settings(&folder).speech_model, Some(Choice::Fast));
        std::fs::remove_dir_all(folder).ok();
    }

    use super::*;

    fn empty_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("lecture-models-test-{name}"));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn nothing_downloaded_means_first_launch() {
        let folder = empty_folder("nothing");
        assert_eq!(current_choice(&folder, true), None);
    }

    #[test]
    fn uses_whatever_model_is_there() {
        let folder = empty_folder("whatever");
        fs::write(folder.join("ggml-small.en.bin"), "x").unwrap();
        assert_eq!(current_choice(&folder, true), Some(Choice::Fast));
    }

    #[test]
    fn picked_model_wins_when_both_are_there() {
        let folder = empty_folder("picked");
        fs::write(folder.join("ggml-small.en.bin"), "x").unwrap();
        fs::write(folder.join("ggml-large-v3-turbo.bin"), "x").unwrap();
        assert_eq!(current_choice(&folder, true), Some(Choice::Accurate));

        save_choice(&folder, Choice::Fast).unwrap();
        assert_eq!(current_choice(&folder, true), Some(Choice::Fast));
    }

    #[test]
    fn free_version_falls_back_to_fast() {
        let folder = empty_folder("free");
        fs::write(folder.join("ggml-large-v3-turbo.bin"), "x").unwrap();
        save_choice(&folder, Choice::Accurate).unwrap();
        // only Most accurate is there, so there's nothing it can use yet
        assert_eq!(current_choice(&folder, false), None);

        fs::write(folder.join("ggml-small.en.bin"), "x").unwrap();
        assert_eq!(current_choice(&folder, false), Some(Choice::Fast));

        // and Pro picks up right where it was
        assert_eq!(current_choice(&folder, true), Some(Choice::Accurate));
    }

    #[test]
    fn checksum_matches_a_known_one() {
        let folder = empty_folder("checksum");
        let path = folder.join("hello.txt");
        fs::write(&path, "hello").unwrap();
        assert_eq!(sha1_of(&path).unwrap(), "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }
}
