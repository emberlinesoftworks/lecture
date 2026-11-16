use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::marks::MarkedRange;
use crate::transcriber::Segment;

#[derive(Serialize, Deserialize)]
pub struct Transcript {
    // a name you gave it in the sidebar. none means it shows the date it was recorded
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub lines: Vec<Segment>,
    pub marks: Vec<MarkedRange>,
}

// what the lecture list shows for each recording
#[derive(Serialize)]
pub struct LectureSummary {
    pub path: String,
    pub title: Option<String>,
    // the folder it's in, none if it's loose in the Lecture folder
    pub folder: Option<String>,
    pub recorded_at: u64,
    pub duration_ms: i64,
    pub preview: String,
    pub mark_count: usize,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub path: String,
    pub recorded_at: u64,
    pub line_index: usize,
    pub start_ms: i64,
    pub text: String,
}

// reads lecture-123.json next to lecture-123.wav. the first transcripts were saved
// as just a list of lines, so those still open (with no marks)
pub fn load_transcript(wav_path: &Path) -> Result<Transcript, String> {
    let json = fs::read_to_string(wav_path.with_extension("json"))
        .map_err(|e| format!("couldn't read transcript: {e}"))?;

    if let Ok(transcript) = serde_json::from_str::<Transcript>(&json) {
        return Ok(transcript);
    }
    let lines: Vec<Segment> = serde_json::from_str(&json).map_err(|e| format!("couldn't read transcript: {e}"))?;
    Ok(Transcript { title: None, lines, marks: Vec::new() })
}

// moves a lecture and everything saved with it (transcript, marks, exports) to the Trash,
// so it can still be put back from Finder if it was a mistake
pub fn delete_lecture(wav_path: &Path) -> Result<(), String> {
    for file in saved_with(wav_path) {
        move_to_trash(&file)?;
    }
    Ok(())
}

// every file that belongs to one lecture. the recording comes last, so if something
// goes wrong partway it still shows up in the list
fn saved_with(wav_path: &Path) -> Vec<PathBuf> {
    [
        wav_path.with_extension("json"),
        wav_path.with_extension("marks.json"),
        wav_path.with_extension("before.json"),
        wav_path.with_extension("md"),
        wav_path.to_path_buf(),
    ]
    .into_iter()
    .filter(|file| file.exists())
    .collect()
}

// --- organizing: names and folders ---
// folders are real folders inside Documents/Lecture, so Finder shows the same thing the app does

pub fn rename_lecture(wav_path: &Path, title: &str) -> Result<(), String> {
    let mut transcript = load_transcript(wav_path)?;
    let title = title.trim();
    // clearing the name goes back to showing the date
    transcript.title = (!title.is_empty()).then(|| title.chars().take(120).collect());
    let json = serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())?;
    fs::write(wav_path.with_extension("json"), json).map_err(|e| e.to_string())
}

// moves a lecture and everything saved with it into another folder, returns where the recording ended up
pub fn move_lecture(wav_path: &Path, to_folder: &Path) -> Result<PathBuf, String> {
    let file_name = wav_path.file_name().ok_or("That isn't a lecture")?;
    let new_path = to_folder.join(file_name);
    if new_path == wav_path {
        return Ok(new_path);
    }
    if new_path.exists() {
        return Err("A lecture with that name is already in there".into());
    }

    for file in saved_with(wav_path) {
        let name = file.file_name().ok_or("That isn't a lecture")?;
        fs::rename(&file, to_folder.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(new_path)
}

// the folders inside the Lecture folder, alphabetical
pub fn list_folders(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut folders: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    folders.sort_by_key(|name| name.to_lowercase());
    folders
}

// folder names become real folders in Finder, so keep them to things Finder is happy with
pub fn check_folder_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the folder a name".into());
    }
    if name.chars().count() > 60 {
        return Err("That name is a bit long, keep it under 60 characters".into());
    }
    if name.starts_with('.') || name.contains(['/', '\\', ':']) {
        return Err("Folder names can't start with a dot or have / or : in them".into());
    }
    Ok(name.to_string())
}

pub fn create_folder(root: &Path, name: &str) -> Result<String, String> {
    let name = check_folder_name(name)?;
    let path = root.join(&name);
    if path.exists() {
        return Err(format!("There's already a folder called {name}"));
    }
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(name)
}

// --- the order you dragged lectures into ---
// kept in a hidden file in the Lecture folder: for each folder ("" is the main list),
// the recordings' file names from top to bottom

fn order_path(root: &Path) -> PathBuf {
    root.join(".lecture-order.json")
}

fn load_order(root: &Path) -> HashMap<String, Vec<String>> {
    fs::read_to_string(order_path(root))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn write_order(root: &Path, order: &HashMap<String, Vec<String>>) -> Result<(), String> {
    let json = serde_json::to_string_pretty(order).map_err(|e| e.to_string())?;
    fs::write(order_path(root), json).map_err(|e| e.to_string())
}

// folder: none is the main list. files: recording file names, top to bottom
pub fn save_order(root: &Path, folder: Option<&str>, files: Vec<String>) -> Result<(), String> {
    let mut order = load_order(root);
    order.insert(folder.unwrap_or("").to_string(), files);
    write_order(root, &order)
}

pub fn rename_folder(root: &Path, old_name: &str, new_name: &str) -> Result<String, String> {
    let old_name = check_folder_name(old_name)?;
    let new_name = check_folder_name(new_name)?;
    if old_name == new_name {
        return Ok(new_name);
    }
    if root.join(&new_name).exists() {
        return Err(format!("There's already a folder called {new_name}"));
    }
    fs::rename(root.join(&old_name), root.join(&new_name)).map_err(|e| e.to_string())?;

    // the folder's lectures keep their order
    let mut order = load_order(root);
    if let Some(files) = order.remove(&old_name) {
        order.insert(new_name.clone(), files);
        write_order(root, &order)?;
    }
    Ok(new_name)
}

// the lectures inside go back to the main list first, nothing gets deleted
pub fn delete_folder(root: &Path, name: &str) -> Result<(), String> {
    let name = check_folder_name(name)?;
    let folder = root.join(&name);
    for wav_path in recordings_in(&folder) {
        move_lecture(&wav_path, root)?;
    }
    fs::remove_dir(&folder).map_err(|_| format!("{name} has other files in it, so it was left alone in Finder"))
}

#[cfg(target_os = "macos")]
fn move_to_trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|_| format!("Couldn't move {} to the Trash", file_name(path)))
}

// no Trash to use yet off the Mac (Windows comes later), so just delete
#[cfg(not(target_os = "macos"))]
fn move_to_trash(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|_| format!("Couldn't delete {}", file_name(path)))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

// how long the recording really is, from the wav header (only reads a few bytes).
// the last line's end time can be off by a few seconds since nothing is said at the very end
fn audio_length_ms(wav_path: &Path) -> Option<i64> {
    let reader = hound::WavReader::open(wav_path).ok()?;
    let rate = reader.spec().sample_rate as i64;
    (rate > 0).then(|| reader.duration() as i64 * 1000 / rate)
}

// every recording in the folder that has a transcript, newest first
pub fn list_lectures(folder: &Path) -> Vec<LectureSummary> {
    let mut lectures: Vec<LectureSummary> = lecture_files(folder)
        .into_iter()
        .filter_map(|wav_path| {
            let transcript = load_transcript(&wav_path).ok()?;
            let in_folder = wav_path.parent().filter(|parent| *parent != folder);
            Some(LectureSummary {
                path: wav_path.display().to_string(),
                title: transcript.title.clone(),
                folder: in_folder.and_then(|parent| parent.file_name()).map(|name| name.to_string_lossy().into_owned()),
                recorded_at: recorded_at(&wav_path),
                duration_ms: audio_length_ms(&wav_path)
                    .unwrap_or_else(|| transcript.lines.last().map(|line| line.end_ms).unwrap_or(0)),
                preview: transcript.lines.first().map(|line| line.text.clone()).unwrap_or_default(),
                mark_count: transcript.marks.len(),
            })
        })
        .collect();

    // newest first, then whatever order you dragged them into. anything not placed
    // yet (a new recording) goes on top
    lectures.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at));
    let order = load_order(folder);
    lectures.sort_by_key(|lecture| {
        let file = Path::new(&lecture.path).file_name().map(|name| name.to_string_lossy().into_owned());
        order
            .get(lecture.folder.as_deref().unwrap_or(""))
            .and_then(|names| names.iter().position(|name| Some(name) == file.as_ref()))
            .map_or(0, |position| position + 1)
    });
    lectures
}

// finds the words in every transcript (not case sensitive), newest lectures first
pub fn search(folder: &Path, query: &str) -> Vec<SearchHit> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut hits = Vec::new();
    for lecture in list_lectures(folder) {
        let Ok(transcript) = load_transcript(Path::new(&lecture.path)) else { continue };

        for (line_index, line) in transcript.lines.iter().enumerate() {
            if line.text.to_lowercase().contains(&query) {
                hits.push(SearchHit {
                    path: lecture.path.clone(),
                    recorded_at: lecture.recorded_at,
                    line_index,
                    start_ms: line.start_ms,
                    text: line.text.clone(),
                });
            }
        }
        // plenty for a list, and keeps it fast
        if hits.len() >= 200 {
            break;
        }
    }
    hits
}

// writes lecture-123.md next to the recording and returns its path
pub fn export_markdown(wav_path: &Path, title: &str) -> Result<PathBuf, String> {
    let transcript = load_transcript(wav_path)?;
    let mut markdown = format!("# {title}\n\n");

    if !transcript.marks.is_empty() {
        markdown.push_str("## Your marks\n\n");
        for mark in &transcript.marks {
            if let Some(line) = transcript.lines.get(mark.first_line) {
                markdown.push_str(&format!("- **[{}]** {}\n", format_time(line.start_ms), line.text));
            }
        }
        markdown.push('\n');
    }

    markdown.push_str("## Transcript\n\n");
    for (index, line) in transcript.lines.iter().enumerate() {
        // marked stretches are bold so they stand out when skimming
        let marked = transcript
            .marks
            .iter()
            .any(|mark| index >= mark.first_line && index <= mark.last_line);
        let text = if marked { format!("**{}**", line.text) } else { line.text.clone() };
        markdown.push_str(&format!("[{}] {}\n\n", format_time(line.start_ms), text));
    }

    let markdown_path = wav_path.with_extension("md");
    fs::write(&markdown_path, markdown).map_err(|e| format!("couldn't save export: {e}"))?;
    Ok(markdown_path)
}

// lectures sit in the Lecture folder itself or one folder down (Math 172/lecture-123.wav)
fn lecture_files(folder: &Path) -> Vec<PathBuf> {
    let mut files = recordings_in(folder);
    for name in list_folders(folder) {
        files.extend(recordings_in(&folder.join(name)));
    }
    files
}

fn recordings_in(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect()
}

// recordings are named lecture-<unix seconds>.wav
fn recorded_at(wav_path: &Path) -> u64 {
    wav_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_prefix("lecture-"))
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
}

// 03:25, or 1:03:25 once it passes an hour (same as the window)
fn format_time(ms: i64) -> String {
    let total_seconds = ms.max(0) / 1000;
    let (hours, minutes, seconds) = (total_seconds / 3600, total_seconds / 60 % 60, total_seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("lecture-organize-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn save_lecture(folder: &Path, seconds: u64) -> PathBuf {
        let wav = folder.join(format!("lecture-{seconds}.wav"));
        fs::write(&wav, b"").unwrap();
        let transcript = Transcript { title: None, lines: vec![line(0, "Hello.")], marks: Vec::new() };
        fs::write(wav.with_extension("json"), serde_json::to_string(&transcript).unwrap()).unwrap();
        fs::write(wav.with_extension("marks.json"), "[]").unwrap();
        wav
    }

    #[test]
    fn lectures_in_folders_show_up_with_their_folder() {
        let root = empty_folder("in-folders");
        save_lecture(&root, 100);
        create_folder(&root, "Math 172").unwrap();
        save_lecture(&root.join("Math 172"), 200);

        let lectures = list_lectures(&root);
        assert_eq!(lectures.len(), 2);
        assert_eq!(lectures[0].folder.as_deref(), Some("Math 172"));
        assert_eq!(lectures[1].folder, None);
        assert_eq!(list_folders(&root), ["Math 172"]);
    }

    #[test]
    fn moving_a_lecture_takes_its_transcript_and_marks_along() {
        let root = empty_folder("move");
        let wav = save_lecture(&root, 100);
        create_folder(&root, "Biology").unwrap();

        let moved = move_lecture(&wav, &root.join("Biology")).unwrap();
        assert!(moved.exists());
        assert!(moved.with_extension("json").exists());
        assert!(moved.with_extension("marks.json").exists());
        assert!(!wav.exists());
    }

    #[test]
    fn renaming_keeps_the_transcript_and_clearing_goes_back_to_the_date() {
        let root = empty_folder("rename");
        let wav = save_lecture(&root, 100);

        rename_lecture(&wav, "  Taylor series  ").unwrap();
        let transcript = load_transcript(&wav).unwrap();
        assert_eq!(transcript.title.as_deref(), Some("Taylor series"));
        assert_eq!(transcript.lines.len(), 1);

        rename_lecture(&wav, "").unwrap();
        assert_eq!(load_transcript(&wav).unwrap().title, None);
    }

    #[test]
    fn dragged_order_is_kept_and_new_recordings_go_on_top() {
        let root = empty_folder("order");
        save_lecture(&root, 100);
        save_lecture(&root, 200);
        save_lecture(&root, 300);
        let names = |root: &Path| list_lectures(root).iter().map(|l| l.recorded_at).collect::<Vec<_>>();

        // newest first by default
        assert_eq!(names(&root), [300, 200, 100]);

        // drag the oldest to the top
        save_order(&root, None, vec!["lecture-100.wav".into(), "lecture-300.wav".into(), "lecture-200.wav".into()]).unwrap();
        assert_eq!(names(&root), [100, 300, 200]);

        // a new recording shows up above them
        save_lecture(&root, 400);
        assert_eq!(names(&root), [400, 100, 300, 200]);
    }

    #[test]
    fn renaming_a_folder_keeps_its_order() {
        let root = empty_folder("order-rename");
        create_folder(&root, "Bio").unwrap();
        save_lecture(&root.join("Bio"), 100);
        save_lecture(&root.join("Bio"), 200);
        save_order(&root, Some("Bio"), vec!["lecture-100.wav".into(), "lecture-200.wav".into()]).unwrap();

        rename_folder(&root, "Bio", "Biology").unwrap();
        let order: Vec<u64> = list_lectures(&root).iter().map(|l| l.recorded_at).collect();
        assert_eq!(order, [100, 200]);
    }

    #[test]
    fn deleting_a_folder_keeps_its_lectures() {
        let root = empty_folder("delete-folder");
        create_folder(&root, "Physics").unwrap();
        save_lecture(&root.join("Physics"), 100);

        delete_folder(&root, "Physics").unwrap();
        assert!(!root.join("Physics").exists());
        assert!(root.join("lecture-100.wav").exists());
        assert!(root.join("lecture-100.json").exists());
    }

    #[test]
    fn folder_names_finder_wouldnt_like_are_refused() {
        assert!(check_folder_name("  ").is_err());
        assert!(check_folder_name("a/b").is_err());
        assert!(check_folder_name(".hidden").is_err());
        assert_eq!(check_folder_name("  Math 172 ").unwrap(), "Math 172");
    }

    #[test]
    fn deleting_takes_everything_saved_with_the_lecture() {
        let folder = std::env::temp_dir().join("lecture-delete-test");
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();

        let wav = folder.join("lecture-100.wav");
        for name in ["lecture-100.wav", "lecture-100.json", "lecture-100.marks.json", "lecture-100.md", "lecture-200.wav"] {
            fs::write(folder.join(name), "x").unwrap();
        }

        delete_lecture(&wav).unwrap();

        assert!(!wav.exists());
        assert!(!folder.join("lecture-100.json").exists());
        assert!(!folder.join("lecture-100.marks.json").exists());
        assert!(!folder.join("lecture-100.md").exists());
        // a different lecture is left alone
        assert!(folder.join("lecture-200.wav").exists());
    }

    fn line(start_ms: i64, text: &str) -> Segment {
        Segment { start_ms, end_ms: start_ms + 2000, text: text.to_string(), pause_before_ms: 0 }
    }

    // a throwaway folder with one lecture in it
    fn folder_with_lecture(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("lecture-test-{name}-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let wav = folder.join("lecture-1700000000.wav");
        fs::write(&wav, b"").unwrap();

        let transcript = Transcript {
            title: None,
            lines: vec![line(0, "Today we talk about Coulomb's law."), line(3000, "Here it is."), line(6000, "F equals k q1 q2 over r squared.")],
            marks: vec![MarkedRange { first_line: 1, last_line: 2, mark_ms: 9000 }],
        };
        fs::write(wav.with_extension("json"), serde_json::to_string(&transcript).unwrap()).unwrap();
        folder
    }

    #[test]
    fn lists_lectures_with_their_details() {
        let folder = folder_with_lecture("list");
        let lectures = list_lectures(&folder);
        assert_eq!(lectures.len(), 1);
        assert_eq!(lectures[0].recorded_at, 1_700_000_000);
        assert_eq!(lectures[0].mark_count, 1);
        assert_eq!(lectures[0].preview, "Today we talk about Coulomb's law.");
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn search_ignores_case() {
        let folder = folder_with_lecture("search");
        let hits = search(&folder, "coulomb");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_index, 0);
        assert!(search(&folder, "mitochondria").is_empty());
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn old_transcripts_without_marks_still_open() {
        let folder = std::env::temp_dir().join(format!("lecture-test-old-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let wav = folder.join("lecture-1.wav");
        fs::write(wav.with_extension("json"), r#"[{"start_ms":0,"end_ms":1000,"text":"Hello."}]"#).unwrap();
        let transcript = load_transcript(&wav).unwrap();
        assert_eq!(transcript.lines.len(), 1);
        assert!(transcript.marks.is_empty());
        fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn export_bolds_marked_lines() {
        let folder = folder_with_lecture("export");
        let wav = folder.join("lecture-1700000000.wav");
        let markdown = fs::read_to_string(export_markdown(&wav, "Physics").unwrap()).unwrap();
        assert!(markdown.starts_with("# Physics"));
        assert!(markdown.contains("## Your marks\n\n- **[00:03]** Here it is."));
        assert!(markdown.contains("[00:00] Today we talk about Coulomb's law."));
        assert!(markdown.contains("[00:06] **F equals k q1 q2 over r squared.**"));
        fs::remove_dir_all(folder).ok();
    }
}
