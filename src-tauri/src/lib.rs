mod recorder;
mod transcriber;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use recorder::Recording;
use tauri::{Manager, State};
use transcriber::Segment;

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Recording>>,
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: State<AppState>) -> Result<String, String> {
    let mut recording = state.recording.lock().unwrap();
    if recording.is_some() {
        return Err("Already recording".into());
    }

    // make sure Documents/Lecture exists
    let lecture_folder = app.path().document_dir().map_err(|e| e.to_string())?.join("Lecture");
    std::fs::create_dir_all(&lecture_folder).map_err(|e| e.to_string())?;

    // TODO: name files by course and date once courses exist
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let path = lecture_folder.join(format!("lecture-{timestamp}.wav"));

    *recording = Some(Recording::start(path.clone())?);
    Ok(path.display().to_string())
}

#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<String, String> {
    let recording = state.recording.lock().unwrap().take().ok_or("Not recording")?;
    let path = recording.stop()?;
    Ok(path.display().to_string())
}

#[tauri::command]
async fn transcribe(app: tauri::AppHandle, path: String) -> Result<Vec<Segment>, String> {
    // TODO: download the model on first launch instead of copying it by hand
    let model_path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models")
        .join("ggml-large-v3-turbo.bin");
    if !model_path.exists() {
        return Err(format!("Model not found at {}", model_path.display()));
    }

    // whisper takes a while, so run it off the main thread to keep the window responsive
    let wav_path = PathBuf::from(&path);
    let segments = tauri::async_runtime::spawn_blocking(move || {
        transcriber::transcribe(&model_path, &wav_path)
    })
    .await
    .map_err(|e| e.to_string())??;

    // save the transcript next to the recording
    let json = serde_json::to_string_pretty(&segments).map_err(|e| e.to_string())?;
    std::fs::write(PathBuf::from(&path).with_extension("json"), json).map_err(|e| e.to_string())?;

    Ok(segments)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![start_recording, stop_recording, transcribe])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
