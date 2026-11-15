pub mod lectures;
pub mod marks;
mod recorder;
pub mod transcriber;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use recorder::Recording;
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use lectures::{LectureSummary, SearchHit, Transcript};

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Recording>>,
}

#[derive(Serialize)]
struct StoppedRecording {
    path: String,
    marks: Vec<u64>,
}

// where every recording, transcript and export lives
fn lecture_folder(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().document_dir().map_err(|e| e.to_string())?.join("Lecture"))
}

// only touch files in the Lecture folder, whatever path the window sends
fn lecture_file(app: &tauri::AppHandle, path: &str) -> Result<PathBuf, String> {
    let folder = lecture_folder(app)?;
    let path = PathBuf::from(path);
    if path.parent() != Some(folder.as_path()) {
        return Err("That file isn't in the Lecture folder".into());
    }
    Ok(path)
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: State<AppState>) -> Result<String, String> {
    let mut recording = state.recording.lock().unwrap();
    if recording.is_some() {
        return Err("Already recording".into());
    }

    // make sure Documents/Lecture exists
    let lecture_folder = lecture_folder(&app)?;
    std::fs::create_dir_all(&lecture_folder).map_err(|e| e.to_string())?;

    // TODO: name files by course and date once courses exist
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let path = lecture_folder.join(format!("lecture-{timestamp}.wav"));

    *recording = Some(Recording::start(path.clone())?);
    Ok(path.display().to_string())
}

#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<StoppedRecording, String> {
    let recording = state.recording.lock().unwrap().take().ok_or("Not recording")?;
    let (path, marks) = recording.stop()?;

    // keep the marks next to the recording so they survive restarts
    let marks_json = serde_json::to_string(&marks).map_err(|e| e.to_string())?;
    std::fs::write(path.with_extension("marks.json"), marks_json).map_err(|e| e.to_string())?;

    Ok(StoppedRecording {
        path: path.display().to_string(),
        marks,
    })
}

// the Mark button and the hotkey both end up here
fn add_mark(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut recording = state.recording.lock().unwrap();
    let mark_ms = recording.as_mut().ok_or("Not recording")?.add_mark();

    // let the window know so it can show the mark
    app.emit("mark-added", mark_ms).map_err(|e| e.to_string())
}

#[tauri::command]
fn mark_moment(app: tauri::AppHandle) -> Result<(), String> {
    add_mark(&app)
}

#[tauri::command]
async fn transcribe(app: tauri::AppHandle, path: String, marks: Vec<u64>) -> Result<Transcript, String> {
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
    let wav_path = lecture_file(&app, &path)?;
    let json_path = wav_path.with_extension("json");
    let lines = tauri::async_runtime::spawn_blocking(move || {
        transcriber::transcribe(&model_path, &wav_path)
    })
    .await
    .map_err(|e| e.to_string())??;

    let transcript = Transcript {
        marks: marks::marked_ranges(&lines, &marks),
        lines,
    };

    // save the transcript next to the recording
    let json = serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())?;
    std::fs::write(json_path, json).map_err(|e| e.to_string())?;

    Ok(transcript)
}

#[tauri::command]
fn list_lectures(app: tauri::AppHandle) -> Result<Vec<LectureSummary>, String> {
    Ok(lectures::list_lectures(&lecture_folder(&app)?))
}

#[tauri::command]
fn open_lecture(app: tauri::AppHandle, path: String) -> Result<Transcript, String> {
    lectures::load_transcript(&lecture_file(&app, &path)?)
}

#[tauri::command]
fn search_lectures(app: tauri::AppHandle, query: String) -> Result<Vec<SearchHit>, String> {
    Ok(lectures::search(&lecture_folder(&app)?, &query))
}

// saves a Markdown copy next to the recording and shows it in Finder
#[tauri::command]
fn export_lecture(app: tauri::AppHandle, path: String, title: String) -> Result<String, String> {
    let markdown_path = lectures::export_markdown(&lecture_file(&app, &path)?, &title)?;
    tauri_plugin_opener::reveal_item_in_dir(&markdown_path).map_err(|e| e.to_string())?;
    Ok(markdown_path.display().to_string())
}

// ⌥⌘M
fn mark_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyM)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed && shortcut == &mark_shortcut() {
                        // pressing it when not recording does nothing
                        let _ = add_mark(app);
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .setup(|app| {
            // works even when Lecture isn't the front window
            if let Err(error) = app.global_shortcut().register(mark_shortcut()) {
                eprintln!("couldn't register the mark shortcut: {error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            mark_moment,
            transcribe,
            list_lectures,
            open_lecture,
            search_lectures,
            export_lecture
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
