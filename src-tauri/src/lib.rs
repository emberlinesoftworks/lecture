pub mod cooling;
pub mod lectures;
pub mod live;
pub mod marks;
pub mod models;
mod recorder;
pub mod silence;
pub mod transcriber;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use live::LiveTranscriber;
use recorder::Recording;
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use lectures::{LectureSummary, SearchHit, Transcript};

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<ActiveRecording>>,
}

// the recording plus the transcriber working through it during class
// (no transcriber if the model isn't installed yet)
struct ActiveRecording {
    recording: Recording,
    live: Option<LiveTranscriber>,
}

#[derive(Serialize)]
struct StoppedRecording {
    path: String,
    marks: Vec<u64>,
    // missing if it couldn't be transcribed during class, then the window asks for it separately
    transcript: Option<Transcript>,
}

fn models_folder(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("models"))
}

// the speech model in use. if nothing is downloaded yet this path won't exist,
// and the window shows the first launch screen
fn model_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let folder = models_folder(app)?;
    let choice = models::current_choice(&folder).unwrap_or(models::Choice::Accurate);
    Ok(folder.join(models::model_file(choice).file_name))
}

// saves the transcript next to the recording
fn save_transcript(wav_path: &std::path::Path, transcript: &Transcript) -> Result<(), String> {
    let json = serde_json::to_string_pretty(transcript).map_err(|e| e.to_string())?;
    std::fs::write(wav_path.with_extension("json"), json).map_err(|e| e.to_string())
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

    // the recorder shares its audio with the transcriber as it goes
    let pending = Arc::new(Mutex::new(Vec::new()));
    let new_recording = Recording::start(path.clone(), pending.clone())?;

    let model_path = model_path(&app)?;
    let live = model_path.exists().then(|| {
        let progress_app = app.clone();
        let resting_app = app.clone();
        LiveTranscriber::start(
            model_path,
            pending,
            move |done_ms| {
                let _ = progress_app.emit("transcribed-up-to", done_ms);
            },
            move |resting| {
                let _ = resting_app.emit("transcription-resting", resting);
            },
        )
    });

    *recording = Some(ActiveRecording { recording: new_recording, live });
    Ok(path.display().to_string())
}

#[tauri::command]
async fn stop_recording(state: State<'_, AppState>) -> Result<StoppedRecording, String> {
    let active = state.recording.lock().unwrap().take().ok_or("Not recording")?;

    // finishing the last minute of transcription takes a few seconds, so do it off the main thread
    let (path, marks, lines) = tauri::async_runtime::spawn_blocking(move || {
        let (path, marks) = active.recording.stop()?;
        let lines = active.live.and_then(|live| live.finish().ok());
        Ok::<_, String>((path, marks, lines))
    })
    .await
    .map_err(|e| e.to_string())??;

    // keep the marks next to the recording so they survive restarts
    let marks_json = serde_json::to_string(&marks).map_err(|e| e.to_string())?;
    std::fs::write(path.with_extension("marks.json"), marks_json).map_err(|e| e.to_string())?;

    let transcript = match lines {
        Some(lines) => {
            let transcript = Transcript {
                marks: marks::marked_ranges(&lines, &marks),
                lines,
            };
            save_transcript(&path, &transcript)?;
            Some(transcript)
        }
        None => None,
    };

    Ok(StoppedRecording {
        path: path.display().to_string(),
        marks,
        transcript,
    })
}

// the Mark button and the hotkey both end up here
fn add_mark(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut recording = state.recording.lock().unwrap();
    let mark_ms = recording.as_mut().ok_or("Not recording")?.recording.add_mark();

    // let the window know so it can show the mark
    app.emit("mark-added", mark_ms).map_err(|e| e.to_string())
}

// the window asks for this a few times a second to draw the level meter
#[tauri::command]
fn mic_level(state: State<AppState>) -> f32 {
    state
        .recording
        .lock()
        .unwrap()
        .as_ref()
        .map(|active| active.recording.level())
        .unwrap_or(0.0)
}

#[tauri::command]
fn mark_moment(app: tauri::AppHandle) -> Result<(), String> {
    add_mark(&app)
}

// sends how far whisper has gotten to the window, for the progress in the status line
fn progress_reporter(app: &tauri::AppHandle) -> transcriber::ProgressReport {
    let app = app.clone();
    std::sync::Arc::new(move |done_ms| {
        let _ = app.emit("transcribe-progress", done_ms);
    })
}

#[tauri::command]
async fn transcribe(app: tauri::AppHandle, path: String, marks: Vec<u64>) -> Result<Transcript, String> {
    let model_path = model_path(&app)?;
    if !model_path.exists() {
        return Err(format!("Model not found at {}", model_path.display()));
    }

    // whisper takes a while, so run it off the main thread to keep the window responsive
    let wav_path = lecture_file(&app, &path)?;
    let progress_app = app.clone();
    let lines = tauri::async_runtime::spawn_blocking(move || {
        transcriber::transcribe(&model_path, &wav_path, progress_reporter(&progress_app))
    })
    .await
    .map_err(|e| e.to_string())??;

    let transcript = Transcript {
        marks: marks::marked_ranges(&lines, &marks),
        lines,
    };

    save_transcript(&lecture_file(&app, &path)?, &transcript)?;
    Ok(transcript)
}

// runs a saved lecture through the current model again (after a fix to transcription,
// or after switching to the more accurate model). the old transcript is kept
// next to it as .before.json in case the new one is worse
#[tauri::command]
async fn retranscribe(app: tauri::AppHandle, path: String) -> Result<Transcript, String> {
    let model_path = model_path(&app)?;
    if !model_path.exists() {
        return Err("Download a speech model first".into());
    }

    let wav_path = lecture_file(&app, &path)?;
    let marks: Vec<u64> = std::fs::read_to_string(wav_path.with_extension("marks.json"))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let transcript_path = wav_path.with_extension("json");
    if transcript_path.exists() {
        std::fs::copy(&transcript_path, wav_path.with_extension("before.json")).map_err(|e| e.to_string())?;
    }

    let audio_path = wav_path.clone();
    let progress_app = app.clone();
    let lines = tauri::async_runtime::spawn_blocking(move || {
        transcriber::transcribe(&model_path, &audio_path, progress_reporter(&progress_app))
    })
        .await
        .map_err(|e| e.to_string())??;

    let transcript = Transcript {
        marks: marks::marked_ranges(&lines, &marks),
        lines,
    };
    save_transcript(&wav_path, &transcript)?;
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

#[tauri::command]
fn model_status(app: tauri::AppHandle) -> Result<models::ModelStatus, String> {
    Ok(models::status(&models_folder(&app)?))
}

// downloads the model you picked (and the voice detector), then switches to it
#[tauri::command]
async fn download_model(app: tauri::AppHandle, choice: models::Choice) -> Result<(), String> {
    let folder = models_folder(&app)?;
    let progress_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        models::download(&folder, choice, |done, total| {
            let _ = progress_app.emit("download-progress", (done, total));
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// switches between models that are already downloaded
#[tauri::command]
fn use_model(app: tauri::AppHandle, choice: models::Choice) -> Result<(), String> {
    models::save_choice(&models_folder(&app)?, choice)
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
            mic_level,
            transcribe,
            retranscribe,
            list_lectures,
            open_lecture,
            search_lectures,
            export_lecture,
            model_status,
            download_model,
            use_model
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
