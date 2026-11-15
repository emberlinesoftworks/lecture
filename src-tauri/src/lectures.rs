use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::marks::MarkedRange;
use crate::transcriber::Segment;

#[derive(Serialize, Deserialize)]
pub struct Transcript {
    pub lines: Vec<Segment>,
    pub marks: Vec<MarkedRange>,
}

// what the lecture list shows for each recording
#[derive(Serialize)]
pub struct LectureSummary {
    pub path: String,
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
    Ok(Transcript { lines, marks: Vec::new() })
}

// every recording in the folder that has a transcript, newest first
pub fn list_lectures(folder: &Path) -> Vec<LectureSummary> {
    let mut lectures: Vec<LectureSummary> = lecture_files(folder)
        .into_iter()
        .filter_map(|wav_path| {
            let transcript = load_transcript(&wav_path).ok()?;
            Some(LectureSummary {
                path: wav_path.display().to_string(),
                recorded_at: recorded_at(&wav_path),
                duration_ms: transcript.lines.last().map(|line| line.end_ms).unwrap_or(0),
                preview: transcript.lines.first().map(|line| line.text.clone()).unwrap_or_default(),
                mark_count: transcript.marks.len(),
            })
        })
        .collect();

    lectures.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at));
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

fn lecture_files(folder: &Path) -> Vec<PathBuf> {
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
