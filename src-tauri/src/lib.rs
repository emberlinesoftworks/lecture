pub mod cooling;
pub mod courses;
pub mod crosscheck;
pub mod dictionary;
pub mod lectures;
pub mod mathtext;
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
// puts a new transcript together: the course's word fixes (if the lecture is in a course
// folder), then where the marks land
fn make_transcript(
    wav_path: &std::path::Path,
    title: Option<String>,
    mut lines: Vec<transcriber::Segment>,
    marks: &[u64],
) -> Transcript {
    courses::fix_words(&words_for(wav_path), &mut lines);
    Transcript { title, marks: marks::marked_ranges(&lines, marks), lines }
}

// the words to fix in a lecture: its course's own, plus the built in ones for the course's subject
fn words_for(wav_path: &std::path::Path) -> courses::Course {
    let course = lectures::course_for(wav_path).map(|folder| courses::load_course(&folder)).unwrap_or_default();
    dictionary::with_built_in(&course)
}

fn save_transcript(wav_path: &std::path::Path, transcript: &Transcript) -> Result<(), String> {
    let json = serde_json::to_string_pretty(transcript).map_err(|e| e.to_string())?;
    std::fs::write(wav_path.with_extension("json"), json).map_err(|e| e.to_string())
}

// where every recording, transcript and export lives
fn lecture_folder(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().document_dir().map_err(|e| e.to_string())?.join("Lecture"))
}

// only touch files in the Lecture folder, whatever path the window sends
// (lectures can be up to two folders down, like Lecture/Math 172/Homework/lecture-123.wav)
fn lecture_file(app: &tauri::AppHandle, path: &str) -> Result<PathBuf, String> {
    let folder = lecture_folder(app)?;
    let path = PathBuf::from(path);
    let sneaky = path.components().any(|part| matches!(part, std::path::Component::ParentDir));
    let depth = path.strip_prefix(&folder).map(|inside| inside.components().count()).unwrap_or(0);
    if sneaky || !(1..=3).contains(&depth) {
        return Err("That file isn't in the Lecture folder".into());
    }
    Ok(path)
}

// a course or folder inside the Lecture folder, like "Math 172/Homework".
// none means the Lecture folder itself
fn lecture_subfolder(app: &tauri::AppHandle, place: Option<&str>) -> Result<PathBuf, String> {
    let root = lecture_folder(app)?;
    match place {
        Some(place) => Ok(root.join(lectures::check_place(place)?)),
        None => Ok(root),
    }
}

// a course's own folder (courses are always at the top)
fn course_folder(app: &tauri::AppHandle, course: &str) -> Result<PathBuf, String> {
    let folder = lecture_folder(app)?.join(lectures::check_folder_name(course)?);
    if !lectures::is_course(&folder) {
        return Err(format!("{course} isn't a course"));
    }
    Ok(folder)
}

#[tauri::command]
fn start_recording(app: tauri::AppHandle, state: State<AppState>, folder: Option<String>) -> Result<String, String> {
    let mut recording = state.recording.lock().unwrap();
    if recording.is_some() {
        return Err("Already recording".into());
    }

    // make sure the folder exists (Documents/Lecture, or a course folder in it)
    let lecture_folder = lecture_subfolder(&app, folder.as_deref())?;
    std::fs::create_dir_all(&lecture_folder).map_err(|e| e.to_string())?;

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
            let transcript = make_transcript(&path, None, lines, &marks);
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

    let transcript = make_transcript(&lecture_file(&app, &path)?, None, lines, &marks);

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

    // keep the name you gave it
    let title = lectures::load_transcript(&wav_path).ok().and_then(|old| old.title);

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

    let transcript = make_transcript(&wav_path, title, lines, &marks);
    save_transcript(&wav_path, &transcript)?;
    Ok(transcript)
}

#[tauri::command]
fn delete_lecture(app: tauri::AppHandle, path: String) -> Result<(), String> {
    lectures::delete_lecture(&lecture_file(&app, &path)?)
}

#[tauri::command]
fn rename_lecture(app: tauri::AppHandle, path: String, title: String) -> Result<(), String> {
    lectures::rename_lecture(&lecture_file(&app, &path)?, &title)
}

// folder: none moves it back out to the main list. returns where it ended up
#[tauri::command]
fn move_lecture(app: tauri::AppHandle, path: String, folder: Option<String>) -> Result<String, String> {
    let to_folder = lecture_subfolder(&app, folder.as_deref())?;
    let new_path = lectures::move_lecture(&lecture_file(&app, &path)?, &to_folder)?;
    Ok(new_path.display().to_string())
}

// the subject and word list for a course folder
#[tauri::command]
fn course_settings(app: tauri::AppHandle, folder: String) -> Result<courses::Course, String> {
    Ok(courses::load_course(&course_folder(&app, &folder)?))
}

// saves it, then fixes the words in the lectures already in the folder. returns how many changed
#[tauri::command]
fn save_course(app: tauri::AppHandle, folder: String, course: courses::Course) -> Result<usize, String> {
    let folder = course_folder(&app, &folder)?;
    courses::save_course(&folder, &course)?;
    lectures::fix_words_in_folder(&folder, &dictionary::with_built_in(&courses::load_course(&folder)))
}

// folder: none is the main list. files: the recordings' file names, top to bottom
#[tauri::command]
fn save_order(app: tauri::AppHandle, folder: Option<String>, files: Vec<String>) -> Result<(), String> {
    if let Some(place) = &folder {
        lectures::check_place(place)?;
    }
    lectures::save_order(&lecture_folder(&app)?, folder.as_deref(), files)
}

#[tauri::command]
fn list_places(app: tauri::AppHandle) -> Result<Vec<lectures::Place>, String> {
    Ok(lectures::list_places(&lecture_folder(&app)?))
}

// a new course (always at the top) or folder (at the top, or inside a course or folder)
#[tauri::command]
fn create_place(app: tauri::AppHandle, name: String, parent: Option<String>, course: bool) -> Result<String, String> {
    let root = lecture_folder(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    lectures::create_place(&root, parent.as_deref(), &name, course)
}

// turns a top level folder into a course (gives it a subject and word fixes)
#[tauri::command]
fn make_course(app: tauri::AppHandle, folder: String) -> Result<(), String> {
    let path = lecture_folder(&app)?.join(lectures::check_folder_name(&folder)?);
    if !path.is_dir() {
        return Err(format!("{folder} isn't there anymore"));
    }
    if !lectures::is_course(&path) {
        courses::save_course(&path, &courses::Course::default())?;
    }
    Ok(())
}

// turns a course back into a plain folder. its lectures stay put, the word fixes
// already made to them stay too (the originals are still in .original.json)
#[tauri::command]
fn make_folder(app: tauri::AppHandle, folder: String) -> Result<(), String> {
    let path = course_folder(&app, &folder)?;
    std::fs::remove_file(path.join(".course.json")).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_place(app: tauri::AppHandle, path: String, new_name: String) -> Result<String, String> {
    lectures::rename_place(&lecture_folder(&app)?, &path, &new_name)
}

#[tauri::command]
fn delete_place(app: tauri::AppHandle, path: String) -> Result<(), String> {
    lectures::delete_place(&lecture_folder(&app)?, &path)
}

// what fixing a word from the transcript did
#[derive(serde::Serialize)]
struct FixedWord {
    transcript: Transcript,
    // the course it went into (none when the lecture isn't in one, then only this lecture is fixed)
    course: Option<String>,
    // how many lectures changed
    lectures: usize,
}

// double clicking a word in the transcript: fix it here, and in every lecture of the course if there is one
#[tauri::command]
fn fix_word(app: tauri::AppHandle, path: String, heard: String, word: String) -> Result<FixedWord, String> {
    let wav_path = lecture_file(&app, &path)?;
    let mut one_word = courses::Course::default();
    courses::add_word(&mut one_word, &word, &heard)?;

    // you're asking for it now, so a time you changed it back in this lecture doesn't count anymore
    let mut transcript = lectures::load_transcript(&wav_path)?;
    let heard_lower = heard.trim().to_lowercase();
    for line in transcript.lines.iter_mut() {
        line.kept.retain(|kept| *kept != heard_lower);
    }
    save_transcript(&wav_path, &transcript)?;

    let Some(course_folder) = lectures::course_for(&wav_path) else {
        let (transcript, changed) = lectures::fix_lecture(&wav_path, transcript, &one_word)?;
        return Ok(FixedWord { transcript, course: None, lectures: changed as usize });
    };

    let mut course = courses::load_course(&course_folder);
    courses::add_word(&mut course, &word, &heard)?;
    courses::save_course(&course_folder, &course)?;
    let lectures = lectures::fix_words_in_folder(&course_folder, &dictionary::with_built_in(&courses::load_course(&course_folder)))?;
    Ok(FixedWord {
        transcript: lectures::load_transcript(&wav_path)?,
        course: course_folder.file_name().map(|name| name.to_string_lossy().into_owned()),
        lectures,
    })
}

// a word a course fix changed: put back what was heard (and leave it that way in this line)
#[tauri::command]
fn change_back_fix(app: tauri::AppHandle, path: String, line: usize, fix: usize) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let mut transcript = lectures::load_transcript(&wav_path)?;
    let segment = transcript.lines.get_mut(line).ok_or("That line isn't there anymore")?;
    courses::change_back(segment, fix)?;
    save_transcript(&wav_path, &transcript)?;
    Ok(transcript)
}

// the second opinion on unsure words, run after a transcript is ready.
// uses whichever model you aren't using, if it's downloaded
#[tauri::command]
async fn cross_check(app: tauri::AppHandle, state: State<'_, AppState>, path: String) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let transcript = lectures::load_transcript(&wav_path)?;
    // not during class, one model running is plenty
    if state.recording.lock().unwrap().is_some() {
        return Ok(transcript);
    }
    let folder = models_folder(&app)?;
    let other = match models::current_choice(&folder) {
        Some(models::Choice::Fast) => models::Choice::Accurate,
        _ => models::Choice::Fast,
    };
    let other_model = folder.join(models::model_file(other).file_name);
    if !other_model.exists() || transcript.lines.iter().all(|line| line.unsure.is_empty()) {
        return Ok(transcript);
    }

    let lines = transcript.lines.clone();
    let audio_path = wav_path.clone();
    let results = tauri::async_runtime::spawn_blocking(move || crosscheck::second_opinion(&other_model, &audio_path, &lines))
        .await
        .map_err(|e| e.to_string())??;

    // you might have fixed words while it was listening, so only lines that haven't changed get updated
    let mut latest = lectures::load_transcript(&wav_path)?;
    for ((now, before), result) in latest.lines.iter_mut().zip(&transcript.lines).zip(results) {
        if let Some(unsure) = result {
            if now.text == before.text {
                now.unsure = unsure;
            }
        }
    }
    save_transcript(&wav_path, &latest)?;
    Ok(latest)
}

// that word is right after all, so it doesn't need the gray underline
#[tauri::command]
fn sure_now(app: tauri::AppHandle, path: String, line: usize, word: usize) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let mut transcript = lectures::load_transcript(&wav_path)?;
    let segment = transcript.lines.get_mut(line).ok_or("That line isn't there anymore")?;
    if word < segment.unsure.len() {
        segment.unsure.remove(word);
    }
    save_transcript(&wav_path, &transcript)?;
    Ok(transcript)
}

// each line with its math written out, for lectures in a math course (none for anything else)
#[tauri::command]
fn math_view(app: tauri::AppHandle, path: String) -> Result<Option<Vec<String>>, String> {
    let wav_path = lecture_file(&app, &path)?;
    if !lectures::is_math(&wav_path) {
        return Ok(None);
    }
    let transcript = lectures::load_transcript(&wav_path)?;
    Ok(Some(transcript.lines.iter().map(|line| mathtext::to_math(&line.text)).collect()))
}

// hides lines (a side conversation, a name) or brings them back
#[tauri::command]
fn hide_lines(app: tauri::AppHandle, path: String, first: usize, last: usize, hidden: bool) -> Result<Transcript, String> {
    lectures::set_hidden(&lecture_file(&app, &path)?, first, last, hidden)
}

// every fix that became this word was right
#[tauri::command]
fn keep_all_fixes(app: tauri::AppHandle, path: String, word: String) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let mut transcript = lectures::load_transcript(&wav_path)?;
    courses::keep_all(&mut transcript.lines, &word);
    save_transcript(&wav_path, &transcript)?;
    Ok(transcript)
}

// changes just the spot you double clicked, nothing else
#[tauri::command]
fn change_one_word(
    app: tauri::AppHandle,
    path: String,
    line: usize,
    near: usize,
    heard: String,
    word: String,
) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let mut transcript = lectures::load_transcript(&wav_path)?;
    let segment = transcript.lines.get_mut(line).ok_or("That line isn't there anymore")?;
    courses::change_one(segment, near, &heard, &word)?;
    save_transcript(&wav_path, &transcript)?;
    Ok(transcript)
}

// the fix was right, stop underlining it
#[tauri::command]
fn keep_fix(app: tauri::AppHandle, path: String, line: usize, fix: usize) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let mut transcript = lectures::load_transcript(&wav_path)?;
    let segment = transcript.lines.get_mut(line).ok_or("That line isn't there anymore")?;
    courses::keep_fix(segment, fix)?;
    save_transcript(&wav_path, &transcript)?;
    Ok(transcript)
}

#[tauri::command]
fn list_lectures(app: tauri::AppHandle) -> Result<Vec<LectureSummary>, String> {
    Ok(lectures::list_lectures(&lecture_folder(&app)?))
}

#[tauri::command]
fn open_lecture(app: tauri::AppHandle, path: String) -> Result<Transcript, String> {
    let wav_path = lecture_file(&app, &path)?;
    let transcript = lectures::load_transcript(&wav_path)?;
    // the built in words can grow with updates, so older lectures catch up when they're opened
    match lectures::fix_lecture(&wav_path, transcript.clone(), &words_for(&wav_path)) {
        Ok((fixed, _)) => Ok(fixed),
        Err(_) => Ok(transcript),
    }
}

#[tauri::command]
fn search_lectures(app: tauri::AppHandle, query: String) -> Result<Vec<SearchHit>, String> {
    Ok(lectures::search(&lecture_folder(&app)?, &query))
}

// saves a Markdown copy next to the recording and shows it in Finder
#[tauri::command]
fn export_lecture(app: tauri::AppHandle, path: String, title: String, math: bool) -> Result<String, String> {
    let markdown_path = lectures::export_markdown(&lecture_file(&app, &path)?, &title, math)?;
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
            delete_lecture,
            change_back_fix,
            fix_word,
            keep_all_fixes,
            hide_lines,
            math_view,
            cross_check,
            sure_now,
            change_one_word,
            keep_fix,
            rename_lecture,
            move_lecture,
            save_order,
            course_settings,
            save_course,
            list_places,
            create_place,
            make_course,
            make_folder,
            rename_place,
            delete_place,
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
