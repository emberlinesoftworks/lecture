import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Segment = {
  start_ms: number;
  end_ms: number;
  text: string;
  pause_before_ms: number;
};

type MarkedRange = {
  first_line: number;
  last_line: number;
  mark_ms: number;
};

type Transcript = {
  lines: Segment[];
  marks: MarkedRange[];
};

type StoppedRecording = {
  path: string;
  marks: number[];
  // already done if it was transcribed during the recording
  transcript: Transcript | null;
};

type LectureSummary = {
  path: string;
  recorded_at: number;
  duration_ms: number;
  preview: string;
  mark_count: number;
};

type SearchHit = {
  path: string;
  recorded_at: number;
  line_index: number;
  start_ms: number;
  text: string;
};

const recordButton = document.querySelector<HTMLButtonElement>("#record")!;
const buttonLabel = recordButton.querySelector<HTMLElement>(".label")!;
const markButton = document.querySelector<HTMLButtonElement>("#mark")!;
const timer = document.querySelector<HTMLElement>("#timer")!;
const statusText = document.querySelector<HTMLElement>("#status")!;
const progressText = document.querySelector<HTMLElement>("#progress")!;
const playback = document.querySelector<HTMLElement>("#playback")!;
const lectureTitle = document.querySelector<HTMLElement>("#lecture-title")!;
const exportButton = document.querySelector<HTMLButtonElement>("#export")!;
const player = document.querySelector<HTMLAudioElement>("#player")!;
const transcriptList = document.querySelector<HTMLOListElement>("#transcript")!;
const searchInput = document.querySelector<HTMLInputElement>("#search")!;
const lectureList = document.querySelector<HTMLUListElement>("#lecture-list")!;
const searchResults = document.querySelector<HTMLUListElement>("#search-results")!;
const sidebarEmpty = document.querySelector<HTMLElement>("#sidebar-empty")!;

let startTime = 0;
let timerInterval: number | undefined;
let markCount = 0;
let audioContext: AudioContext | undefined;
let segments: Segment[] = [];
let lineButtons: HTMLButtonElement[] = [];
let currentLine = -1;
let currentPath = "";
let searchTimeout: number | undefined;

// turns milliseconds into 03:25 (or 1:03:25 once it passes an hour)
function formatTime(milliseconds: number) {
  const totalSeconds = Math.floor(milliseconds / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = String(Math.floor(totalSeconds / 60) % 60).padStart(2, "0");
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${minutes}:${seconds}` : `${minutes}:${seconds}`;
}

// recordings are named by when they started, e.g. "Sat, Nov 14, 11:47 PM"
function formatDate(unixSeconds: number) {
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

// the lecture-123 part of the path is the start time
function recordedAtFromPath(path: string) {
  const match = path.match(/lecture-(\d+)\.wav$/);
  return match ? Number(match[1]) : 0;
}

function setRecording(isRecording: boolean) {
  recordButton.classList.toggle("recording", isRecording);
  recordButton.setAttribute("aria-pressed", String(isRecording));
  buttonLabel.textContent = isRecording ? "Stop" : "Record";
  markButton.hidden = !isRecording;
}

// a quiet tick so you know the mark worked without looking at the screen
function playTick() {
  if (!audioContext) return;
  const tone = audioContext.createOscillator();
  const volume = audioContext.createGain();
  const now = audioContext.currentTime;

  tone.frequency.value = 880;
  volume.gain.setValueAtTime(0.08, now);
  volume.gain.exponentialRampToValueAtTime(0.001, now + 0.12);

  tone.connect(volume).connect(audioContext.destination);
  tone.start(now);
  tone.stop(now + 0.12);
}

// fires for both the Mark button and the ⌥⌘M hotkey
listen<number>("mark-added", (event) => {
  markCount++;
  playTick();
  statusText.textContent = `Marked ${formatTime(event.payload)} (${markCount} ${markCount === 1 ? "mark" : "marks"})`;
});

// the transcriber works through the recording about a minute at a time during class
listen<number>("transcribed-up-to", (event) => {
  if (!recordButton.classList.contains("recording")) return;
  progressText.textContent = `Transcribed up to ${formatTime(event.payload)}`;
});

// each line is a button, clicking it jumps the audio to when that was said
function showTranscript(audioPath: string, transcript: Transcript) {
  currentPath = audioPath;
  segments = transcript.lines;
  lineButtons = [];
  currentLine = -1;
  transcriptList.replaceChildren();

  segments.forEach((segment, index) => {
    const item = document.createElement("li");
    const line = document.createElement("button");
    const time = document.createElement("time");
    const text = document.createElement("span");

    line.type = "button";
    line.className = "line";
    time.textContent = formatTime(segment.start_ms);
    text.textContent = segment.text;
    line.append(time, text);
    line.addEventListener("click", () => playFrom(index));

    item.append(line);
    transcriptList.append(item);
    lineButtons.push(line);
  });

  // rust works out which lines each mark covers (see marks.rs)
  for (const range of transcript.marks) {
    for (let index = range.first_line; index <= range.last_line; index++) {
      lineButtons[index]?.classList.add("in-mark");
    }
    lineButtons[range.first_line]?.classList.add("marked");
    lineButtons[range.first_line]?.setAttribute("title", `You marked this at ${formatTime(range.mark_ms)}`);
  }

  lectureTitle.textContent = formatDate(recordedAtFromPath(audioPath));
  player.src = convertFileSrc(audioPath);
  playback.hidden = segments.length === 0;
  highlightOpenLecture();
}

function playFrom(index: number) {
  player.currentTime = segments[index].start_ms / 1000;
  player.play();
}

// highlight whichever line is playing right now
player.addEventListener("timeupdate", () => {
  const nowMs = player.currentTime * 1000;
  const playing = segments.findIndex((segment) => nowMs >= segment.start_ms && nowMs < segment.end_ms);
  if (playing === currentLine) return;

  lineButtons[currentLine]?.classList.remove("current");
  currentLine = playing;

  const line = lineButtons[currentLine];
  if (line) {
    line.classList.add("current");
    line.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }
});

async function openLecture(path: string, jumpToLine?: number) {
  try {
    const transcript = await invoke<Transcript>("open_lecture", { path });
    showTranscript(path, transcript);
    statusText.textContent = "";

    if (jumpToLine !== undefined) {
      lineButtons[jumpToLine]?.scrollIntoView({ block: "center" });
      playFrom(jumpToLine);
    }
  } catch (error) {
    statusText.textContent = String(error);
  }
}

// --- sidebar: saved lectures and search ---

function highlightOpenLecture() {
  for (const button of lectureList.querySelectorAll<HTMLButtonElement>("button")) {
    button.classList.toggle("open", button.dataset.path === currentPath);
  }
}

async function refreshLectureList() {
  const lectures = await invoke<LectureSummary[]>("list_lectures");
  lectureList.replaceChildren();

  for (const lecture of lectures) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    const date = document.createElement("span");
    const preview = document.createElement("span");
    const details = document.createElement("span");

    button.type = "button";
    button.className = "lecture-item";
    button.dataset.path = lecture.path;
    date.className = "lecture-date";
    preview.className = "lecture-preview";
    details.className = "lecture-details";

    date.textContent = formatDate(lecture.recorded_at);
    preview.textContent = lecture.preview;
    details.textContent = formatTime(lecture.duration_ms);
    if (lecture.mark_count > 0) {
      details.textContent += ` · ${lecture.mark_count} ${lecture.mark_count === 1 ? "mark" : "marks"}`;
    }

    button.append(date, preview, details);
    button.addEventListener("click", () => openLecture(lecture.path));
    item.append(button);
    lectureList.append(item);
  }

  sidebarEmpty.textContent = "Your lectures will show up here after you record one.";
  sidebarEmpty.hidden = lectures.length > 0 || !searchResults.hidden;
  highlightOpenLecture();
}

// puts <mark> around every match without ever treating the text as HTML
function highlightMatches(text: string, query: string) {
  const fragment = document.createDocumentFragment();
  const lowerText = text.toLowerCase();
  const lowerQuery = query.toLowerCase();
  let position = 0;

  while (position < text.length) {
    const found = lowerText.indexOf(lowerQuery, position);
    if (found === -1) break;

    fragment.append(text.slice(position, found));
    const match = document.createElement("mark");
    match.textContent = text.slice(found, found + query.length);
    fragment.append(match);
    position = found + query.length;
  }
  fragment.append(text.slice(position));
  return fragment;
}

async function runSearch() {
  const query = searchInput.value.trim();

  // empty search box shows the normal lecture list again
  if (!query) {
    searchResults.hidden = true;
    lectureList.hidden = false;
    sidebarEmpty.hidden = lectureList.children.length > 0;
    return;
  }

  const hits = await invoke<SearchHit[]>("search_lectures", { query });
  searchResults.replaceChildren();

  for (const hit of hits) {
    const item = document.createElement("li");
    const button = document.createElement("button");
    const where = document.createElement("span");
    const text = document.createElement("span");

    button.type = "button";
    button.className = "search-hit";
    where.className = "lecture-date";
    text.className = "search-text";

    where.textContent = `${formatDate(hit.recorded_at)} · ${formatTime(hit.start_ms)}`;
    text.append(highlightMatches(hit.text, query));

    button.append(where, text);
    button.addEventListener("click", () => openLecture(hit.path, hit.line_index));
    item.append(button);
    searchResults.append(item);
  }

  lectureList.hidden = true;
  searchResults.hidden = false;
  sidebarEmpty.textContent = `Nothing found for "${query}"`;
  sidebarEmpty.hidden = hits.length > 0;
}

// wait until typing pauses so we don't search on every key
searchInput.addEventListener("input", () => {
  clearTimeout(searchTimeout);
  searchTimeout = setTimeout(() => {
    runSearch().catch((error) => (statusText.textContent = String(error)));
  }, 200);
});

exportButton.addEventListener("click", async () => {
  try {
    const exportedPath = await invoke<string>("export_lecture", {
      path: currentPath,
      title: `Lecture, ${lectureTitle.textContent}`,
    });
    statusText.textContent = `Exported to ${exportedPath}`;
  } catch (error) {
    statusText.textContent = String(error);
  }
});

// --- recording ---

// only needed if it couldn't transcribe during class (like if the model wasn't installed yet)
async function transcribe(path: string, marks: number[]) {
  statusText.textContent = "Transcribing...";
  const started = Date.now();

  const transcript = await invoke<Transcript>("transcribe", { path, marks });

  const seconds = Math.round((Date.now() - started) / 1000);
  statusText.textContent = `Transcribed in ${seconds}s`;
  showTranscript(path, transcript);
}

markButton.addEventListener("click", () => {
  invoke("mark_moment").catch((error) => (statusText.textContent = String(error)));
});

recordButton.addEventListener("click", async () => {
  const isRecording = recordButton.classList.contains("recording");
  // disable the button so it can't be double clicked while waiting on rust
  recordButton.disabled = true;

  try {
    if (isRecording) {
      // stop recording. most of it is already transcribed, this just finishes the last bit
      clearInterval(timerInterval);
      statusText.textContent = "Finishing the transcript...";
      const stopped = await invoke<StoppedRecording>("stop_recording");
      setRecording(false);
      progressText.textContent = "";

      if (stopped.transcript) {
        const markNote = stopped.marks.length > 0 ? `, ${stopped.marks.length} marked` : "";
        statusText.textContent = `Transcript ready${markNote}`;
        showTranscript(stopped.path, stopped.transcript);
      } else {
        await transcribe(stopped.path, stopped.marks);
      }
      await refreshLectureList();
    } else {
      // the tick sound needs this, and it has to be made after a click
      audioContext ??= new AudioContext();

      // start recording and the timer
      player.pause();
      await invoke("start_recording");
      startTime = Date.now();
      markCount = 0;
      timer.textContent = "00:00";
      timerInterval = setInterval(() => {
        timer.textContent = formatTime(Date.now() - startTime);
      }, 250);
      statusText.textContent = "Recording. Press ⌥⌘M anytime to mark a moment";
      progressText.textContent = "";
      playback.hidden = true;
      setRecording(true);
    }
  } catch (error) {
    statusText.textContent = String(error);
  } finally {
    recordButton.disabled = false;
  }
});

refreshLectureList().catch((error) => (statusText.textContent = String(error)));
