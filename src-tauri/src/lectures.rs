use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::marks::MarkedRange;
use crate::transcriber::Segment;

#[derive(Serialize, Deserialize, Clone)]
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
    // the course or folder it's in ("Math 172/Homework"), none if it's loose
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
        wav_path.with_extension("original.json"),
        wav_path.with_extension("md"),
        wav_path.to_path_buf(),
    ]
    .into_iter()
    .filter(|file| file.exists())
    .collect()
}

// runs a course's word fixes over every lecture already in its folder, returns how many changed.
// the first time a transcript gets changed this way its untouched version is kept as
// .original.json, so nothing whisper heard is ever lost
pub fn fix_words_in_folder(folder: &Path, course: &crate::courses::Course) -> Result<usize, String> {
    let mut changed = 0;
    let mut wavs = recordings_in(folder);
    for inner in subfolders(folder) {
        wavs.extend(recordings_in(&folder.join(inner)));
    }
    for wav_path in wavs {
        let Ok(transcript) = load_transcript(&wav_path) else { continue };
        if fix_lecture(&wav_path, transcript, course)?.1 {
            changed += 1;
        }
    }
    Ok(changed)
}

// fixes one lecture's words and saves it if anything changed.
// the first time, the untouched transcript is kept next to it as .original.json
pub fn fix_lecture(
    wav_path: &Path,
    mut transcript: Transcript,
    course: &crate::courses::Course,
) -> Result<(Transcript, bool), String> {
    let before: Vec<String> = transcript.lines.iter().map(|line| line.text.clone()).collect();
    crate::courses::fix_words(course, &mut transcript.lines);
    if transcript.lines.iter().map(|line| &line.text).eq(before.iter()) {
        return Ok((transcript, false));
    }

    let json_path = wav_path.with_extension("json");
    let original = wav_path.with_extension("original.json");
    if !original.exists() {
        fs::copy(&json_path, &original).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())?;
    fs::write(&json_path, json).map_err(|e| e.to_string())?;
    Ok((transcript, true))
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

// --- courses and folders ---
// both are real folders inside Documents/Lecture, so Finder shows the same thing the app does.
// a course is a folder with a .course.json in it (its subject and words). courses only sit at
// the top, and any top level course or folder can have folders inside it, one level deep

#[derive(Serialize, Debug, PartialEq)]
pub struct Place {
    // "Math 172" or "Math 172/Homework"
    pub path: String,
    pub name: String,
    pub course: bool,
}

pub fn is_course(folder: &Path) -> bool {
    folder.join(".course.json").exists()
}

// the course a lecture belongs to, if any (it can be in the course or a folder inside it)
pub fn course_for(wav_path: &Path) -> Option<PathBuf> {
    let folder = wav_path.parent()?;
    [Some(folder), folder.parent()].into_iter().flatten().find(|dir| is_course(dir)).map(Path::to_path_buf)
}

fn subfolders(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

// every course and folder, each top level one followed by the folders inside it
pub fn list_places(root: &Path) -> Vec<Place> {
    let mut places = Vec::new();
    for name in subfolders(root) {
        let folder = root.join(&name);
        places.push(Place { path: name.clone(), name: name.clone(), course: is_course(&folder) });
        for inner in subfolders(&folder) {
            places.push(Place { path: format!("{name}/{inner}"), name: inner, course: false });
        }
    }
    places
}

// folder names become real folders in Finder, so keep them to things Finder is happy with
pub fn check_folder_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give it a name".into());
    }
    if name.chars().count() > 60 {
        return Err("That name is a bit long, keep it under 60 characters".into());
    }
    if name.starts_with('.') || name.contains(['/', '\\', ':']) {
        return Err("Names can't start with a dot or have / or : in them".into());
    }
    Ok(name.to_string())
}

// a course or folder path like "Math 172/Homework", at most two deep
pub fn check_place(path: &str) -> Result<String, String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 2 {
        return Err("Folders only go one level deep".into());
    }
    let checked: Result<Vec<String>, String> = parts.into_iter().map(check_folder_name).collect();
    Ok(checked?.join("/"))
}

// parent: none makes it at the top. returns its path
pub fn create_place(root: &Path, parent: Option<&str>, name: &str, course: bool) -> Result<String, String> {
    let name = check_folder_name(name)?;
    let path = match parent {
        Some(parent) => {
            let parent = check_place(parent)?;
            if course {
                return Err("A course can't go inside another course or folder".into());
            }
            if parent.contains('/') {
                return Err("Folders only go one level deep".into());
            }
            if !root.join(&parent).is_dir() {
                return Err(format!("{parent} isn't there anymore"));
            }
            format!("{parent}/{name}")
        }
        None => name.clone(),
    };

    let folder = root.join(&path);
    if folder.exists() {
        return Err(format!("There's already one called {name}"));
    }
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    if course {
        crate::courses::save_course(&folder, &crate::courses::Course::default())?;
    }
    Ok(path)
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

// renames a course or folder, returns its new path. its lectures keep their order
pub fn rename_place(root: &Path, path: &str, new_name: &str) -> Result<String, String> {
    let path = check_place(path)?;
    let new_name = check_folder_name(new_name)?;
    let new_path = match path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{new_name}"),
        None => new_name.clone(),
    };
    if new_path == path {
        return Ok(new_path);
    }
    if root.join(&new_path).exists() {
        return Err(format!("There's already one called {new_name}"));
    }
    fs::rename(root.join(&path), root.join(&new_path)).map_err(|e| e.to_string())?;

    let mut order = load_order(root);
    let moved: Vec<String> = order.keys().filter(|key| **key == path || key.starts_with(&format!("{path}/"))).cloned().collect();
    for key in moved {
        if let Some(files) = order.remove(&key) {
            order.insert(format!("{new_path}{}", &key[path.len()..]), files);
        }
    }
    write_order(root, &order)?;
    Ok(new_path)
}

// removes a course or folder. its lectures (and the ones in folders inside it) go up a level
// first, so nothing gets deleted
pub fn delete_place(root: &Path, path: &str) -> Result<(), String> {
    let path = check_place(path)?;
    let folder = root.join(&path);
    let up = folder.parent().ok_or("That isn't a folder")?.to_path_buf();

    let mut inner: Vec<PathBuf> = subfolders(&folder).into_iter().map(|name| folder.join(name)).collect();
    inner.push(folder.clone());
    for dir in &inner {
        for wav_path in recordings_in(dir) {
            move_lecture(&wav_path, &up)?;
        }
    }

    let _ = fs::remove_file(folder.join(".course.json"));
    let _ = fs::remove_file(folder.join(".DS_Store"));
    for dir in &inner {
        let _ = fs::remove_file(dir.join(".DS_Store"));
        fs::remove_dir(dir).map_err(|_| format!("{} has other files in it, so it was left alone in Finder", file_name(dir)))?;
    }

    let mut order = load_order(root);
    order.retain(|key, _| *key != path && !key.starts_with(&format!("{path}/")));
    write_order(root, &order)
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
            Some(LectureSummary {
                path: wav_path.display().to_string(),
                title: transcript.title.clone(),
                folder: place_of(folder, &wav_path),
                recorded_at: recorded_at(&wav_path),
                duration_ms: audio_length_ms(&wav_path)
                    .unwrap_or_else(|| transcript.lines.last().map(|line| line.end_ms).unwrap_or(0)),
                preview: transcript.lines.iter().find(|line| !line.hidden).map(|line| line.text.clone()).unwrap_or_default(),
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
            if !line.hidden && line.text.to_lowercase().contains(&query) {
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
// a lecture in a course whose subject is math
pub fn is_math(wav_path: &Path) -> bool {
    course_for(wav_path).is_some_and(|folder| crate::courses::load_course(&folder).subject.as_deref() == Some("math"))
}

// hides or brings back lines first to last
pub fn set_hidden(wav_path: &Path, first: usize, last: usize, hidden: bool) -> Result<Transcript, String> {
    let mut transcript = load_transcript(wav_path)?;
    if first > last || last >= transcript.lines.len() {
        return Err("Those lines aren't there anymore".into());
    }
    for line in &mut transcript.lines[first..=last] {
        line.hidden = hidden;
    }
    let json = serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())?;
    fs::write(wav_path.with_extension("json"), json).map_err(|e| e.to_string())?;
    Ok(transcript)
}

// export_markdown with math on writes spoken math the way it's written ("6 × sin²(0)")
pub fn export_markdown(wav_path: &Path, title: &str, math: bool) -> Result<PathBuf, String> {
    let mut transcript = load_transcript(wav_path)?;
    if math {
        for line in &mut transcript.lines {
            line.text = crate::mathtext::to_math(&line.text);
        }
    }
    let mut markdown = format!("# {title}\n\n");

    if !transcript.marks.is_empty() {
        markdown.push_str("## Your marks\n\n");
        for mark in &transcript.marks {
            let first_shown = (mark.first_line..=mark.last_line).find_map(|index| transcript.lines.get(index).filter(|line| !line.hidden));
            if let Some(line) = first_shown {
                markdown.push_str(&format!("- **[{}]** {}\n", format_time(line.start_ms), line.text));
            }
        }
        markdown.push('\n');
    }

    markdown.push_str("## Transcript\n\n");
    for (index, line) in transcript.lines.iter().enumerate() {
        if line.hidden {
            continue;
        }
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

// lectures sit in the Lecture folder, in a course or folder, or in a folder inside one
// (Math 172/Homework/lecture-123.wav)
fn lecture_files(root: &Path) -> Vec<PathBuf> {
    let mut files = recordings_in(root);
    for place in list_places(root) {
        files.extend(recordings_in(&root.join(&place.path)));
    }
    files
}

// "Math 172/Homework" for a lecture in there, none if it's loose
fn place_of(root: &Path, wav_path: &Path) -> Option<String> {
    let relative = wav_path.parent()?.strip_prefix(root).ok()?;
    let parts: Vec<String> = relative.iter().map(|part| part.to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub fn recordings_in(folder: &Path) -> Vec<PathBuf> {
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
        create_place(&root, None, "Math 172", false).unwrap();
        save_lecture(&root.join("Math 172"), 200);

        let lectures = list_lectures(&root);
        assert_eq!(lectures.len(), 2);
        assert_eq!(lectures[0].folder.as_deref(), Some("Math 172"));
        assert_eq!(lectures[1].folder, None);
        assert_eq!(list_places(&root), [Place { path: "Math 172".into(), name: "Math 172".into(), course: false }]);
    }

    #[test]
    fn moving_a_lecture_takes_its_transcript_and_marks_along() {
        let root = empty_folder("move");
        let wav = save_lecture(&root, 100);
        create_place(&root, None, "Biology", false).unwrap();

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
        create_place(&root, None, "Bio", false).unwrap();
        save_lecture(&root.join("Bio"), 100);
        save_lecture(&root.join("Bio"), 200);
        save_order(&root, Some("Bio"), vec!["lecture-100.wav".into(), "lecture-200.wav".into()]).unwrap();

        rename_place(&root, "Bio", "Biology").unwrap();
        let order: Vec<u64> = list_lectures(&root).iter().map(|l| l.recorded_at).collect();
        assert_eq!(order, [100, 200]);
    }

    #[test]
    fn course_words_fix_old_lectures_and_keep_the_original() {
        use crate::courses::{Course, CourseWord};
        let root = empty_folder("course-fix");
        create_place(&root, None, "Math", false).unwrap();
        let folder = root.join("Math");
        let wav = folder.join("lecture-100.wav");
        fs::write(&wav, b"").unwrap();
        let transcript = Transcript { title: None, lines: vec![line(0, "The Gordon series.")], marks: Vec::new() };
        fs::write(wav.with_extension("json"), serde_json::to_string(&transcript).unwrap()).unwrap();

        let course = Course { subject: None, words: vec![CourseWord { word: "Maclaurin".into(), heard_as: vec!["Gordon".into()] }] };
        assert_eq!(fix_words_in_folder(&folder, &course).unwrap(), 1);
        assert_eq!(load_transcript(&wav).unwrap().lines[0].text, "The Maclaurin series.");
        assert!(fs::read_to_string(wav.with_extension("original.json")).unwrap().contains("Gordon"));

        // nothing left to fix the second time
        assert_eq!(fix_words_in_folder(&folder, &course).unwrap(), 0);
    }

    #[test]
    fn courses_hold_folders_but_not_other_courses() {
        let root = empty_folder("places");
        assert_eq!(create_place(&root, None, "Math 172", true).unwrap(), "Math 172");
        assert_eq!(create_place(&root, Some("Math 172"), "Homework", false).unwrap(), "Math 172/Homework");
        create_place(&root, None, "Old notes", false).unwrap();

        assert!(create_place(&root, Some("Math 172"), "Calc", true).is_err());
        assert!(create_place(&root, Some("Math 172/Homework"), "Deeper", false).is_err());

        let places = list_places(&root);
        let summary: Vec<(&str, bool)> = places.iter().map(|p| (p.path.as_str(), p.course)).collect();
        assert_eq!(summary, [("Math 172", true), ("Math 172/Homework", false), ("Old notes", false)]);
    }

    #[test]
    fn lectures_in_a_folder_inside_a_course_belong_to_the_course() {
        let root = empty_folder("course-inner");
        create_place(&root, None, "Math 172", true).unwrap();
        create_place(&root, Some("Math 172"), "Homework", false).unwrap();
        let wav = save_lecture(&root.join("Math 172/Homework"), 100);

        assert_eq!(course_for(&wav), Some(root.join("Math 172")));
        assert_eq!(list_lectures(&root)[0].folder.as_deref(), Some("Math 172/Homework"));
        assert_eq!(course_for(&save_lecture(&root, 200)), None);
    }

    #[test]
    fn deleting_a_course_moves_everything_up_and_keeps_it() {
        let root = empty_folder("delete-course");
        create_place(&root, None, "Bio", true).unwrap();
        create_place(&root, Some("Bio"), "Labs", false).unwrap();
        save_lecture(&root.join("Bio"), 100);
        save_lecture(&root.join("Bio/Labs"), 200);

        delete_place(&root, "Bio").unwrap();
        assert!(!root.join("Bio").exists());
        assert!(root.join("lecture-100.wav").exists());
        assert!(root.join("lecture-200.wav").exists());
    }

    #[test]
    fn renaming_a_course_keeps_the_order_of_its_folders() {
        let root = empty_folder("rename-course");
        create_place(&root, None, "Bio", true).unwrap();
        create_place(&root, Some("Bio"), "Labs", false).unwrap();
        save_order(&root, Some("Bio/Labs"), vec!["lecture-1.wav".into()]).unwrap();

        assert_eq!(rename_place(&root, "Bio", "Biology").unwrap(), "Biology");
        assert!(is_course(&root.join("Biology")));
        assert_eq!(load_order(&root).get("Biology/Labs"), Some(&vec!["lecture-1.wav".to_string()]));
    }

    #[test]
    fn deleting_a_folder_keeps_its_lectures() {
        let root = empty_folder("delete-folder");
        create_place(&root, None, "Physics", false).unwrap();
        save_lecture(&root.join("Physics"), 100);

        delete_place(&root, "Physics").unwrap();
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
        Segment { start_ms, end_ms: start_ms + 2000, text: text.to_string(), ..Default::default() }
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
    fn hidden_lines_stay_out_of_search_exports_and_the_preview() {
        let folder = empty_folder("hidden");
        let wav = folder.join("lecture-1700000000.wav");
        fs::write(&wav, b"").unwrap();
        let transcript = Transcript {
            title: None,
            lines: vec![line(0, "Did you see the game last night?"), line(2000, "Okay, the Maclaurin series.")],
            marks: Vec::new(),
        };
        fs::write(wav.with_extension("json"), serde_json::to_string(&transcript).unwrap()).unwrap();

        set_hidden(&wav, 0, 0, true).unwrap();
        assert!(search(&folder, "game").is_empty());
        assert_eq!(list_lectures(&folder)[0].preview, "Okay, the Maclaurin series.");
        let markdown = fs::read_to_string(export_markdown(&wav, "Test", false).unwrap()).unwrap();
        assert!(!markdown.contains("game") && markdown.contains("Maclaurin"));

        // and it comes back
        set_hidden(&wav, 0, 0, false).unwrap();
        assert_eq!(search(&folder, "game").len(), 1);
        assert!(set_hidden(&wav, 0, 5, true).is_err());
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
        let markdown = fs::read_to_string(export_markdown(&wav, "Physics", false).unwrap()).unwrap();
        assert!(markdown.starts_with("# Physics"));
        assert!(markdown.contains("## Your marks\n\n- **[00:03]** Here it is."));
        assert!(markdown.contains("[00:00] Today we talk about Coulomb's law."));
        assert!(markdown.contains("[00:06] **F equals k q1 q2 over r squared.**"));
        fs::remove_dir_all(folder).ok();
    }
}
