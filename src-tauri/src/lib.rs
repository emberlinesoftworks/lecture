mod recorder;

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use recorder::Recording;
use tauri::{Manager, State};

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

    let dir = app.path().document_dir().map_err(|e| e.to_string())?.join("Lecture");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // TODO: name files by course and date once courses exist
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let path = dir.join(format!("lecture-{ts}.wav"));

    *recording = Some(Recording::start(path.clone())?);
    Ok(path.display().to_string())
}

#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<String, String> {
    let recording = state.recording.lock().unwrap().take().ok_or("Not recording")?;
    let path = recording.stop()?;
    Ok(path.display().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![start_recording, stop_recording])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
