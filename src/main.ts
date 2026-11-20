import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// a word a course fix changed. start and end count characters in the line's text
type WordFix = { start: number; end: number; heard: string };

type Segment = {
  start_ms: number;
  end_ms: number;
  text: string;
  pause_before_ms: number;
  fixes?: WordFix[];
  hidden?: boolean;
  unsure?: Unsure[];
};

// a word whisper wasn't sure about, and what the other model heard there
type Unsure = { start: number; end: number; options?: string[] };

type MarkedRange = {
  first_line: number;
  last_line: number;
  mark_ms: number;
};

type Transcript = {
  title?: string | null;
  lines: Segment[];
  marks: MarkedRange[];
};

type StoppedRecording = {
  path: string;
  marks: number[];
  // already done if it was transcribed during the recording
  transcript: Transcript | null;
};

// a course or a folder. path is like "Math 172" or "Math 172/Homework"
type Place = { path: string; name: string; course: boolean };

type LectureSummary = {
  path: string;
  title: string | null;
  folder: string | null;
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
const lectureWhen = document.querySelector<HTMLElement>("#lecture-when")!;
const exportButton = document.querySelector<HTMLButtonElement>("#export")!;
const retranscribeButton = document.querySelector<HTMLButtonElement>("#retranscribe")!;
const editListButton = document.querySelector<HTMLButtonElement>("#edit-list")!;
const recordIntoLabel = document.querySelector<HTMLElement>("#record-into-label")!;
const recordInto = document.querySelector<HTMLSelectElement>("#record-into")!;
const coursePage = document.querySelector<HTMLElement>("#course-page")!;
const courseName = document.querySelector<HTMLElement>("#course-name")!;
const courseBack = document.querySelector<HTMLButtonElement>("#course-back")!;
const courseSubject = document.querySelector<HTMLSelectElement>("#course-subject")!;
const wordForm = document.querySelector<HTMLFormElement>("#word-form")!;
const wordInput = document.querySelector<HTMLInputElement>("#word-input")!;
const heardInput = document.querySelector<HTMLInputElement>("#heard-input")!;
const wordList = document.querySelector<HTMLUListElement>("#word-list")!;
const courseStatus = document.querySelector<HTMLElement>("#course-status")!;
const editBar = document.querySelector<HTMLElement>("#edit-bar")!;
const newFolderButton = document.querySelector<HTMLButtonElement>("#new-folder")!;
const newCourseButton = document.querySelector<HTMLButtonElement>("#new-course")!;
const player = document.querySelector<HTMLAudioElement>("#player")!;
const transcriptList = document.querySelector<HTMLOListElement>("#transcript")!;
const playButton = document.querySelector<HTMLButtonElement>("#play")!;
const backButton = document.querySelector<HTMLButtonElement>("#back")!;
const forwardButton = document.querySelector<HTMLButtonElement>("#forward")!;
const scrubber = document.querySelector<HTMLInputElement>("#scrub")!;
const scrubMarks = document.querySelector<HTMLElement>("#scrub-marks")!;
const currentTimeText = document.querySelector<HTMLElement>("#current-time")!;
const totalTimeText = document.querySelector<HTMLElement>("#total-time")!;
const speedButton = document.querySelector<HTMLButtonElement>("#speed")!;
const searchInput = document.querySelector<HTMLInputElement>("#search")!;
const lectureList = document.querySelector<HTMLUListElement>("#lecture-list")!;
const searchResults = document.querySelector<HTMLUListElement>("#search-results")!;
const sidebarEmpty = document.querySelector<HTMLElement>("#sidebar-empty")!;
const layout = document.querySelector<HTMLElement>("#layout")!;
const collapseButton = document.querySelector<HTMLButtonElement>("#collapse")!;
const expandButton = document.querySelector<HTMLButtonElement>("#expand")!;
const meter = document.querySelector<HTMLElement>("#meter")!;
const recorder = document.querySelector<HTMLElement>("#recorder")!;
const setup = document.querySelector<HTMLElement>("#setup")!;
const setupTitle = document.querySelector<HTMLElement>("#setup-title")!;
const modelChoices = document.querySelectorAll<HTMLButtonElement>(".model-choice");
const downloadButton = document.querySelector<HTMLButtonElement>("#download")!;
const setupBackButton = document.querySelector<HTMLButtonElement>("#setup-back")!;
const downloadProgress = document.querySelector<HTMLElement>("#download-progress")!;
const downloadAmount = document.querySelector<HTMLElement>("#download-amount")!;
const downloadFill = document.querySelector<HTMLElement>("#download-fill")!;
const setupError = document.querySelector<HTMLElement>("#setup-error")!;
const modelButton = document.querySelector<HTMLButtonElement>("#model-button")!;
const modelName = document.querySelector<HTMLElement>("#model-name")!;

let startTime = 0;
let timerInterval: number | undefined;
let markCount = 0;
let audioContext: AudioContext | undefined;
let segments: Segment[] = [];
let lineButtons: HTMLButtonElement[] = [];
let markStarts: number[] = [];

let scrubbing = false;
let speedIndex = 0;
const playbackSpeeds = [1, 1.25, 1.5, 1.75, 2, 0.75];
let currentLine = -1;
let currentPath = "";
let searchTimeout: number | undefined;
let meterInterval: number | undefined;
const meterBars: HTMLSpanElement[] = [];

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
  silentSince = 0;
  micWarningShowing = false;
  statusText.classList.remove("warning");
  document.body.classList.toggle("recording-active", isRecording);
  recordButton.classList.toggle("recording", isRecording);
  recordButton.setAttribute("aria-pressed", String(isRecording));
  buttonLabel.textContent = isRecording ? "Stop" : "Record";
  markButton.hidden = !isRecording;
  recordInto.disabled = isRecording;

  if (isRecording) {
    meterInterval = setInterval(updateMeter, 70);
  } else {
    clearInterval(meterInterval);
    for (const bar of meterBars) bar.style.transform = "";
  }
}

// --- level meter ---
// bars scroll left, newest level on the right

for (let i = 0; i < 40; i++) {
  const bar = document.createElement("span");
  meter.append(bar);
  meterBars.push(bar);
}

// if the mic hands over pure silence for this long, something's wrong (muted, unplugged,
// or macOS stopped giving the app the mic). a quiet room still has a little noise in it
const SILENT_WARNING_MS = 8000;
let silentSince = 0;
let micWarningShowing = false;

function checkForSilence(level: number) {
  if (level > 0.0005) {
    silentSince = 0;
    if (micWarningShowing) {
      micWarningShowing = false;
      statusText.classList.remove("warning");
      statusText.textContent = "Recording. Press ⌥⌘M anytime to mark a moment";
    }
    return;
  }

  silentSince ||= Date.now();
  if (!micWarningShowing && Date.now() - silentSince > SILENT_WARNING_MS) {
    micWarningShowing = true;
    statusText.classList.add("warning");
    statusText.textContent =
      "Lecture isn't hearing anything. Check your microphone in System Settings → Sound, and that this app is allowed in Privacy & Security → Microphone";
  }
}

async function updateMeter() {
  const level = await invoke<number>("mic_level");
  checkForSilence(level);
  // square root so quiet speech still moves the bars
  const height = Math.max(0.08, Math.min(1, Math.sqrt(level) * 1.3));

  for (let i = 0; i < meterBars.length - 1; i++) {
    meterBars[i].style.transform = meterBars[i + 1].style.transform;
  }
  meterBars[meterBars.length - 1].style.transform = `scaleY(${height})`;
}

// --- recorder size ---
// while you're reading a lecture the recorder shrinks to one line at the top,
// so the transcript gets the room. recording makes it big again

function updateRecorderSize() {
  const reviewing = !playback.hidden && !recordButton.classList.contains("recording");
  document.body.classList.toggle("reviewing", reviewing);
}

new MutationObserver(updateRecorderSize).observe(playback, { attributes: true, attributeFilter: ["hidden"] });
new MutationObserver(updateRecorderSize).observe(recordButton, { attributes: true, attributeFilter: ["class"] });

// --- hiding the sidebar ---

function setSidebarHidden(hidden: boolean) {
  layout.classList.toggle("collapsed", hidden);
  expandButton.hidden = !hidden;
  try {
    localStorage.setItem("sidebarHidden", String(hidden));
  } catch {
    // not being able to remember this isn't a problem
  }
}

collapseButton.addEventListener("click", () => setSidebarHidden(true));
expandButton.addEventListener("click", () => setSidebarHidden(false));

// ⌘\ toggles it, same as a lot of Mac apps
document.addEventListener("keydown", (event) => {
  if (event.metaKey && event.key === "\\") {
    event.preventDefault();
    setSidebarHidden(!layout.classList.contains("collapsed"));
  }
});

try {
  if (localStorage.getItem("sidebarHidden") === "true") setSidebarHidden(true);
} catch {
  // start with it showing
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

// the transcriber takes breaks when the mac gets warm. the recording keeps going either way
listen<boolean>("transcription-resting", (event) => {
  if (!recordButton.classList.contains("recording") || !event.payload) return;
  progressText.textContent = "Letting your Mac cool down. Still recording, the transcript will catch up";
});

// each line is a button, clicking it jumps the audio to when that was said
function showTranscript(audioPath: string, transcript: Transcript) {
  rememberSpot();
  currentPath = audioPath;
  resumeSpot = true;
  mathLines = null;
  spokenLine = -1;
  examNotesBox.hidden = true;
  drawLines(transcript);
  transcriptList.scrollTop = 0;
  following = true;
  lastHidden = -1;
  loadMath(audioPath);
  loadExamNotes(audioPath);

  // a recording with nothing in it would otherwise just leave a blank screen
  if (segments.length === 0) {
    statusText.textContent =
      "Nothing was heard in this recording, so there's no transcript. If someone was talking, check your microphone in System Settings → Sound";
  }

  showLectureName(transcript.title);
  lectureWhen.textContent = formatDate(recordedAtFromPath(audioPath));
  player.src = convertFileSrc(audioPath);
  scrubber.value = "0";
  updateScrubFill();
  playback.hidden = segments.length === 0;
  highlightOpenLecture();
}

// --- each lecture picks up where you stopped listening ---
// kept by the recording's file name, so moving or renaming a lecture doesn't lose it

let resumeSpot = false;
let lastRemembered = 0;

function spotKey(path: string) {
  return `listened:${path.slice(path.lastIndexOf("/") + 1)}`;
}

function rememberSpot() {
  if (!currentPath || !player.duration) return;
  const seconds = player.currentTime;
  try {
    // the very start or the very end isn't worth coming back to
    if (seconds < 5 || seconds > player.duration - 5) {
      localStorage.removeItem(spotKey(currentPath));
    } else {
      localStorage.setItem(spotKey(currentPath), String(Math.floor(seconds)));
    }
  } catch {
    // nowhere to keep it, it'll just start from the top
  }
}

function savedSpot(path: string) {
  try {
    return Number(localStorage.getItem(spotKey(path))) || 0;
  } catch {
    return 0;
  }
}

player.addEventListener("loadedmetadata", () => {
  if (!resumeSpot) return;
  resumeSpot = false;
  const seconds = savedSpot(currentPath);
  if (seconds > 0 && seconds < player.duration - 5) player.currentTime = seconds;
});
player.addEventListener("pause", rememberSpot);
player.addEventListener("ended", rememberSpot);
player.addEventListener("timeupdate", () => {
  if (Date.now() - lastRemembered < 5000) return;
  lastRemembered = Date.now();
  rememberSpot();
});
window.addEventListener("pagehide", rememberSpot);

// the lines themselves. kept apart from showTranscript so changing one word back
// doesn't restart the audio
function drawLines(transcript: Transcript) {
  // a fix changed the words, so the math needs working out again
  const wordsChanged = mathLines !== null && shownTranscript !== undefined && transcript !== shownTranscript;
  shownTranscript = transcript;
  segments = transcript.lines;
  lineButtons = [];
  currentLine = -1;
  closeFixPopup();
  transcriptList.replaceChildren();

  segments.forEach((segment, index) => {
    const item = document.createElement("li");
    const line = document.createElement("button");
    const time = document.createElement("time");
    const text = document.createElement("span");

    line.type = "button";
    line.className = "line";
    time.textContent = formatTime(segment.start_ms);
    // just the words, so an exam tag in front of them isn't counted as part of the line
    const words = document.createElement("span");
    words.className = "line-text";
    const math = index === spokenLine ? undefined : mathLines?.[index];
    if (math !== undefined) {
      words.textContent = math;
    } else {
      words.append(...lineText(segment, index));
    }
    text.append(words);
    line.classList.toggle("spoken", index === spokenLine);
    line.append(time, text);
    line.addEventListener("click", () => playFrom(index));
    line.addEventListener("dblclick", fixWordAt);

    const hide = document.createElement("button");
    hide.type = "button";
    hide.className = "hide-line";
    hide.textContent = "•••";
    hide.setAttribute("aria-label", "More for this line");
    hide.addEventListener("click", (event) => {
      event.stopPropagation();
      openLineMenu(hide, index);
    });

    // a stretch of hidden lines shows up as one row you can open back up
    if (segment.hidden && !segments[index - 1]?.hidden) {
      transcriptList.append(hiddenRow(index));
    }
    item.hidden = Boolean(segment.hidden);
    item.className = "transcript-line";
    item.append(line, hide);
    transcriptList.append(item);
    lineButtons.push(line);
  });

  // rust works out which lines each mark covers (see marks.rs)
  markStarts = [];
  for (const range of transcript.marks) {
    const firstLine = segments[range.first_line];
    if (firstLine) markStarts.push(firstLine.start_ms);
    for (let index = range.first_line; index <= range.last_line; index++) {
      lineButtons[index]?.classList.add("in-mark");
    }
    lineButtons[range.first_line]?.classList.add("marked");
    lineButtons[range.first_line]?.setAttribute("title", `You marked this at ${formatTime(range.mark_ms)}`);
  }
  drawScrubMarks();

  // done fixing that line: it goes back to math once the new math is worked out
  if (wordsChanged) {
    spokenLine = -1;
    loadMath(currentPath);
  }
}

// --- math lectures: "6 times the sine squared of 0" shown as 6 × sin²(0) ---
// in a math course the transcript shows math. the spoken words are still underneath
// (search and word fixes use them), and double clicking a line shows them to fix a word

let mathLines: string[] | null = null;
let shownTranscript: Transcript | undefined;
// the line showing its spoken words right now, -1 for none
let spokenLine = -1;

function showingMath() {
  return mathLines !== null;
}

// rust only sends math lines for lectures in a math course
async function loadMath(path: string) {
  try {
    const lines = await invoke<string[] | null>("math_view", { path });
    if (currentPath !== path) return;
    mathLines = lines;
    if (lines && shownTranscript) redrawKeepingScroll(shownTranscript);
  } catch {
    // stays as spoken words
  }
}

function redrawKeepingScroll(transcript: Transcript) {
  const scrolled = transcriptList.scrollTop;
  drawLines(transcript);
  transcriptList.scrollTop = scrolled;
}

function showSpoken(index: number) {
  spokenLine = index;
  if (shownTranscript) redrawKeepingScroll(shownTranscript);
}

// --- about the exam: everything the professor said about it, found by rust (emphasis.rs) ---
// dates, review days, sample questions, what's on it and what isn't. a list you can open
// above the transcript, each one plays from where it was said

type ExamNote = {
  first_line: number;
  last_line: number;
  not_on_exam: boolean;
  on_exam: boolean;
  date: boolean;
  material: boolean;
  // just the part about the exam
  said: string;
  // the exams it's under, like "Test 4"
  exams: string[];
  start_ms: number;
  // you put it there yourself
  chosen: boolean;
};

const NEXT_EXAM = "Next exam";
const NOT_ABOUT_AN_EXAM = "not-about-an-exam";
const SORT_AUTOMATICALLY = "sort-automatically";

// the menu on each exam note: which exam it's about. a wrong guess gets fixed once and stays fixed
function moveMenu(path: string, atMs: number, current: string, chosen: boolean, exams: CourseExam[], done: () => void) {
  const menu = document.createElement("select");
  menu.className = "exam-move";
  menu.setAttribute("aria-label", "Which exam this is about");
  const add = (value: string, label: string) => menu.append(new Option(label, value));
  for (const exam of exams) add(exam.name, exam.name);
  if (!exams.some((exam) => exam.name === current)) add(current, current);
  if (current !== NEXT_EXAM) add(NEXT_EXAM, NEXT_EXAM);
  add(NOT_ABOUT_AN_EXAM, "Not about an exam");
  if (chosen) add(SORT_AUTOMATICALLY, "Sort automatically");
  menu.value = current;
  menu.addEventListener("change", async () => {
    const value = menu.value;
    try {
      await invoke("choose_exam", {
        path,
        atMs,
        exam: value === NOT_ABOUT_AN_EXAM || value === SORT_AUTOMATICALLY ? null : value,
        added: false,
        clear: value === SORT_AUTOMATICALLY,
      });
      done();
    } catch (error) {
      statusText.textContent = String(error);
    }
  });
  return menu;
}

// the little labels on an exam note
function examTags(note: { not_on_exam: boolean; on_exam: boolean; date: boolean; material: boolean }, exams: string[] = []) {
  const names = [
    ...exams,
    ...(note.not_on_exam ? ["Not on the exam"] : []),
    ...(note.on_exam ? ["On the exam"] : []),
    ...(note.date ? ["Date"] : []),
    ...(note.material ? ["Study material"] : []),
  ];
  if (names.length === 0) return null;
  const tags = document.createElement("span");
  tags.className = "exam-note-tags";
  for (const name of names) {
    const tag = document.createElement("span");
    tag.className = "exam-note-tag";
    tag.textContent = name;
    tags.append(tag);
  }
  return tags;
}

const examNotesBox = document.querySelector<HTMLElement>("#exam-notes")!;
const examNotesToggle = document.querySelector<HTMLButtonElement>("#exam-notes-toggle")!;
const examNotesCount = document.querySelector<HTMLElement>("#exam-notes-count")!;
const examNotesList = document.querySelector<HTMLOListElement>("#exam-notes-list")!;

// the course the open lecture is in, so its exam notes can be moved between your exams
let lectureCourse: { folder: string; exams: CourseExam[] } | null = null;

async function loadExamNotes(path: string) {
  try {
    const folder = placeOfLecture(path)?.split("/")[0];
    const inCourse = folder !== undefined && places.some((place) => place.path === folder && place.course);
    const [notes, course] = await Promise.all([
      invoke<ExamNote[]>("exam_notes", { path, minutesBehindUtc: new Date().getTimezoneOffset() }),
      inCourse ? invoke<Course>("course_settings", { folder }) : Promise.resolve(null),
    ]);
    if (currentPath !== path) return;
    lectureCourse = course && folder ? { folder, exams: course.exams ?? [] } : null;
    showExamNotes(notes);
  } catch {
    examNotesBox.hidden = true;
  }
}

function showExamNotes(notes: ExamNote[]) {
  examNotesBox.hidden = notes.length === 0;
  examNotesCount.textContent = String(notes.length);
  examNotesList.replaceChildren(
    ...notes.map((note) => {
      const item = document.createElement("li");
      const button = document.createElement("button");
      button.type = "button";
      const time = document.createElement("time");
      time.textContent = formatTime(segments[note.first_line]?.start_ms ?? 0);
      const said = document.createElement("span");
      said.className = "exam-note-text";
      said.textContent = note.said;
      // in a course the menu already says which exam, outside one the name is a tag
      // ("Next exam" just means it isn't under one of yours, so no tag for that)
      const names = lectureCourse ? [] : note.exams.filter((name) => name !== NEXT_EXAM);
      const tags = examTags(note, names);
      button.append(time, ...(tags ? [tags] : []), said);
      button.addEventListener("click", () => {
        lineButtons[note.first_line]?.scrollIntoView({ block: "center", behavior: "smooth" });
        playFrom(note.first_line);
      });
      item.append(button);
      if (lectureCourse) {
        const path = currentPath;
        item.append(moveMenu(path, note.start_ms, note.exams[0] ?? NEXT_EXAM, note.chosen, lectureCourse.exams, () => loadExamNotes(path)));
      }
      return item;
    }),
  );
  setExamNotesOpen(savedExamNotesOpen());
}

function savedExamNotesOpen() {
  try {
    return localStorage.getItem("examNotesOpen") === "true";
  } catch {
    return false;
  }
}

function setExamNotesOpen(open: boolean) {
  examNotesList.hidden = !open;
  examNotesToggle.setAttribute("aria-expanded", String(open));
  try {
    localStorage.setItem("examNotesOpen", String(open));
  } catch {
    // it just won't be remembered
  }
}

examNotesToggle.addEventListener("click", () => setExamNotesOpen(examNotesList.hidden === true));

// --- hiding lines: side conversations, someone's name ---

let lastHidden = -1;

function hiddenRow(first: number) {
  let last = first;
  while (segments[last + 1]?.hidden) last++;
  const count = last - first + 1;

  const item = document.createElement("li");
  item.className = "hidden-lines";
  const row = document.createElement("button");
  row.type = "button";
  const label = document.createElement("span");
  label.textContent = count === 1 ? "1 line hidden" : `${count} lines hidden`;
  const show = document.createElement("span");
  show.className = "hidden-show";
  show.textContent = "Show";
  row.append(label, show);
  row.addEventListener("click", () => setHidden(first, last, false));
  item.append(row);
  return item;
}

// the ••• on a line: hide it, or add it to the exam notes when Lecture missed it
function openLineMenu(anchor: HTMLElement, index: number) {
  closeFixPopup();
  fixPopupWord = anchor;
  fixPopupScroll = transcriptList.scrollTop;
  const buttons = document.createElement("div");
  buttons.className = "line-menu";
  const item = (label: string, action: () => void) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "text-button";
    button.textContent = label;
    button.addEventListener("click", action);
    buttons.append(button);
  };
  item("Hide", () => {
    closeFixPopup();
    hideFrom(index, false);
  });
  if (lastHidden >= 0 && Math.abs(lastHidden - index) > 1) {
    item("Hide back to the last line you hid", () => {
      closeFixPopup();
      hideFrom(index, true);
    });
  }
  if (lectureCourse) {
    item("Add to exam notes", () => chooseExamFor(anchor, index));
  }
  fixPopup.replaceChildren(buttons);
  fixPopup.hidden = false;
  placePopup(anchor.getBoundingClientRect());
  (buttons.firstElementChild as HTMLButtonElement).focus();
}

// which exam a line you're adding is about
function chooseExamFor(anchor: HTMLElement, index: number) {
  if (!lectureCourse) return;
  const label = document.createElement("p");
  label.className = "fix-heard";
  label.textContent = "Which exam is this about?";
  const buttons = document.createElement("div");
  buttons.className = "line-menu";
  const path = currentPath;
  const names = [...lectureCourse.exams.map((exam) => exam.name), NEXT_EXAM];
  for (const name of names) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "text-button";
    button.textContent = name;
    button.addEventListener("click", async () => {
      try {
        await invoke("choose_exam", { path, atMs: segments[index].start_ms, exam: name, added: true, clear: false });
        closeFixPopup();
        statusText.textContent = `Added to exam notes under ${name}`;
        loadExamNotes(path);
      } catch (error) {
        statusText.textContent = String(error);
      }
    });
    buttons.append(button);
  }
  fixPopup.replaceChildren(label, buttons);
  placePopup(anchor.getBoundingClientRect());
  (buttons.firstElementChild as HTMLButtonElement).focus();
}

function hideFrom(index: number, stretch: boolean) {
  // shift-click hides the whole stretch between this and the last line you hid
  const from = stretch && lastHidden >= 0 ? Math.min(lastHidden, index) : index;
  const to = stretch && lastHidden >= 0 ? Math.max(lastHidden, index) : index;
  lastHidden = index;
  setHidden(from, to, true);
}

async function setHidden(first: number, last: number, hidden: boolean) {
  const scrolled = transcriptList.scrollTop;
  try {
    drawLines(await invoke<Transcript>("hide_lines", { path: currentPath, first, last, hidden }));
    transcriptList.scrollTop = scrolled;
    refreshLectureList();
  } catch (error) {
    statusText.textContent = String(error);
  }
}

// playing straight through skips what you hid
function skipHidden() {
  const segment = segments[currentLine];
  if (!segment?.hidden || player.paused) return;
  const next = segments.findIndex((other, index) => index > currentLine && !other.hidden);
  if (next >= 0) {
    player.currentTime = segments[next].start_ms / 1000;
  } else {
    player.pause();
  }
}

// a line's words, with the ones a course fix changed underlined
// fixed words get the amber underline, unsure ones a gray dotted one
function lineText(segment: Segment, lineIndex: number) {
  const chars = Array.from(segment.text);
  const parts: (Node | string)[] = [];
  let at = 0;

  const marked = [
    ...(segment.fixes ?? []).map((fix, index) => ({ start: fix.start, end: fix.end, fix, index })),
    ...(SHOW_UNSURE ? (segment.unsure ?? []) : []).map((unsure, index) => ({ start: unsure.start, end: unsure.end, unsure, index })),
  ].sort((a, b) => a.start - b.start);

  for (const spot of marked) {
    if (spot.start < at || spot.end > chars.length) continue;
    parts.push(chars.slice(at, spot.start).join(""));
    const word = document.createElement("span");
    word.textContent = chars.slice(spot.start, spot.end).join("");
    if ("fix" in spot && spot.fix) {
      const heard = spot.fix.heard;
      word.className = "fixed-word";
      word.title = `Lecture heard "${heard}"`;
      word.addEventListener("click", (event) => {
        // just this word, not playing the line
        event.stopPropagation();
        openFixPopup(word, lineIndex, spot.index, heard);
      });
    } else if ("unsure" in spot && spot.unsure) {
      const unsure = spot.unsure;
      word.className = "unsure-word";
      word.addEventListener("click", (event) => {
        event.stopPropagation();
        openUnsurePopup(word, lineIndex, spot.index, unsure);
      });
    }
    parts.push(word);
    at = spot.end;
  }
  parts.push(chars.slice(at).join(""));
  return parts;
}

// --- unsure words: what the other model heard, one tap to use it ---
// off for now. tested on a real class (Nov 18) the two speech models make the same mistakes,
// so their second opinions were mostly wrong. the confidence is still saved with each transcript,
// so a better second opinion (the on-device language model) can turn this back on
const SHOW_UNSURE = false;

function openUnsurePopup(word: HTMLElement, line: number, index: number, unsure: Unsure) {
  closeFixPopup();
  fixPopupWord = word;
  fixPopupScroll = transcriptList.scrollTop;
  word.classList.add("active");
  const heard = word.textContent ?? "";

  const label = document.createElement("p");
  label.className = "fix-heard";
  label.textContent = unsure.options?.length ? "Lecture isn't sure. The other model heard" : "Lecture isn't sure it heard this right";

  const buttons = document.createElement("div");
  buttons.className = "fix-buttons";
  for (const option of unsure.options ?? []) {
    const use = document.createElement("button");
    use.type = "button";
    use.className = "text-button option-button";
    use.textContent = option;
    use.addEventListener("click", () =>
      updateLines("change_one_word", { line, near: unsure.start, heard, word: option }),
    );
    buttons.append(use);
  }
  const right = document.createElement("button");
  right.type = "button";
  right.className = "text-button";
  right.textContent = "It's right";
  right.addEventListener("click", () => updateLines("sure_now", { line, word: index }));
  buttons.append(right);

  fixPopup.replaceChildren(label, buttons);
  fixPopup.hidden = false;
  placePopup(word.getBoundingClientRect());
  (buttons.firstElementChild as HTMLButtonElement).focus();
}

async function updateLines(command: string, args: Record<string, unknown>) {
  const scrolled = transcriptList.scrollTop;
  try {
    drawLines(await invoke<Transcript>(command, { path: currentPath, ...args }));
    transcriptList.scrollTop = scrolled;
  } catch (error) {
    statusText.textContent = String(error);
    closeFixPopup();
  }
}

// after a transcript is ready, the other model gives a second opinion on the unsure words.
// it fills in a moment later, the transcript is already usable
async function crossCheck(path: string) {
  progressText.textContent = "Double-checking unsure words";
  try {
    const transcript = await invoke<Transcript>("cross_check", { path });
    if (currentPath === path) {
      const scrolled = transcriptList.scrollTop;
      drawLines(transcript);
      transcriptList.scrollTop = scrolled;
    }
  } catch {
    // the gray underlines are still there, just without suggestions
  } finally {
    if (progressText.textContent === "Double-checking unsure words") progressText.textContent = "";
  }
}

// --- fixing a word from the transcript: double click it, type what it should say ---

type FixedWord = { transcript: Transcript; course: string | null; lectures: number };

const wordLetter = /[\p{L}\p{N}'’-]/u;

function fixWordAt(event: MouseEvent) {
  const line = event.currentTarget as HTMLElement;
  const lineIndex = lineButtons.indexOf(line as HTMLButtonElement);
  // a line with math in it first turns back into the words that were said, then you pick one.
  // a line without math already shows its words, so it goes straight to the fix
  const hasMath = mathLines !== null && mathLines[lineIndex] !== segments[lineIndex]?.text;
  if (hasMath && lineIndex !== spokenLine) {
    showSpoken(lineIndex);
    return;
  }
  const caret = document.caretRangeFromPoint(event.clientX, event.clientY);
  const node = caret?.startContainer;
  if (!caret || !node || node.nodeType !== Node.TEXT_NODE || !node.parentElement?.closest(".line-text")) return;

  // grow out from where you clicked to the whole word
  const text = node.textContent ?? "";
  let start = caret.startOffset;
  let end = caret.startOffset;
  while (start > 0 && wordLetter.test(text[start - 1])) start--;
  while (end < text.length && wordLetter.test(text[end])) end++;
  while (end > start && /['’-]/.test(text[end - 1])) end--;
  while (start < end && /['’-]/.test(text[start])) start++;
  if (start === end) return;

  const word = document.createRange();
  word.setStart(node, start);
  word.setEnd(node, end);
  // where the word starts in the line's text, so "just this one" changes this spot
  const before = document.createRange();
  before.setStart(line.querySelector(".line-text")!, 0);
  before.setEnd(node, start);
  const near = Array.from(before.toString()).length;
  openWordPopup(word.getBoundingClientRect(), text.slice(start, end), lineIndex, near);
}

// the course a lecture belongs to, if it's in one
function courseOfLecture(path: string) {
  const top = placeOfLecture(path)?.split("/")[0];
  return places.find((place) => place.path === top && place.course)?.name ?? null;
}

function openWordPopup(box: DOMRect, heard: string, lineIndex: number, near: number) {
  closeFixPopup();
  stopFollowing();
  fixPopupScroll = transcriptList.scrollTop;

  const form = document.createElement("form");
  form.className = "word-popup";

  const heardInput = document.createElement("input");
  heardInput.type = "text";
  heardInput.value = heard;
  heardInput.spellcheck = false;
  const heardLabel = document.createElement("label");
  heardLabel.append(Object.assign(document.createElement("span"), { textContent: "Lecture wrote" }), heardInput);

  const wordInput = document.createElement("input");
  wordInput.type = "text";
  wordInput.value = heard;
  wordInput.spellcheck = false;
  const wordLabel = document.createElement("label");
  wordLabel.append(Object.assign(document.createElement("span"), { textContent: "Should say" }), wordInput);

  const course = courseOfLecture(currentPath);
  const where = document.createElement("p");
  where.className = "fix-heard";
  where.textContent = course ? `All of them fixes every ${course} lecture` : "All of them fixes this whole lecture";

  const justThis = document.createElement("button");
  justThis.type = "button";
  justThis.className = "text-button";
  justThis.textContent = "Just this one";

  const fix = document.createElement("button");
  fix.type = "submit";
  fix.className = "download-button";
  fix.textContent = "All of them";

  const buttons = document.createElement("div");
  buttons.className = "fix-buttons";
  buttons.append(justThis, fix);
  form.append(heardLabel, wordLabel, where, buttons);

  justThis.addEventListener("click", async () => {
    justThis.disabled = true;
    try {
      const transcript = await invoke<Transcript>("change_one_word", {
        path: currentPath,
        line: lineIndex,
        near,
        heard: heardInput.value,
        word: wordInput.value,
      });
      const scrolled = transcriptList.scrollTop;
      drawLines(transcript);
      transcriptList.scrollTop = scrolled;
    } catch (error) {
      where.textContent = String(error);
      justThis.disabled = false;
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    fix.disabled = true;
    try {
      const result = await invoke<FixedWord>("fix_word", {
        path: currentPath,
        heard: heardInput.value,
        word: wordInput.value,
      });
      const scrolled = transcriptList.scrollTop;
      drawLines(result.transcript);
      transcriptList.scrollTop = scrolled;
      statusText.textContent =
        result.course && result.lectures > 1 ? `Fixed in ${result.lectures} ${result.course} lectures` : "";
      refreshLectureList();
    } catch (error) {
      where.textContent = String(error);
      fix.disabled = false;
    }
  });

  fixPopup.replaceChildren(form);
  fixPopup.hidden = false;
  placePopup(box);
  wordInput.focus();
  wordInput.select();
}

// under the word, kept inside the window
function placePopup(box: DOMRect) {
  const width = fixPopup.offsetWidth;
  const height = fixPopup.offsetHeight;
  const left = Math.min(Math.max(8, box.left + box.width / 2 - width / 2), window.innerWidth - width - 8);
  // flip above the word when there's no room under it
  const top = box.bottom + 8 + height > window.innerHeight - 8 ? box.top - 8 - height : box.bottom + 8;
  fixPopup.style.left = `${left}px`;
  fixPopup.style.top = `${top}px`;
}

// --- the popup for a fixed word: change it back or keep it ---

const fixPopup = document.createElement("div");
fixPopup.className = "fix-popup";
fixPopup.hidden = true;
fixPopup.setAttribute("role", "dialog");
document.body.append(fixPopup);
let fixPopupWord: HTMLElement | undefined;
let fixPopupScroll = 0;

function openFixPopup(word: HTMLElement, line: number, fix: number, heard: string) {
  closeFixPopup();
  fixPopupWord = word;
  fixPopupScroll = transcriptList.scrollTop;
  word.classList.add("active");

  const label = document.createElement("p");
  label.className = "fix-heard";
  label.append("Heard ");
  const heardWord = document.createElement("strong");
  heardWord.textContent = heard;
  label.append(heardWord);

  const changeBack = document.createElement("button");
  changeBack.type = "button";
  changeBack.className = "text-button";
  changeBack.textContent = "Change back";
  changeBack.addEventListener("click", () => updateFix("change_back_fix", line, fix));

  const keep = document.createElement("button");
  keep.type = "button";
  keep.className = "text-button";
  keep.textContent = "Keep";
  keep.addEventListener("click", () => updateFix("keep_fix", line, fix));

  const buttons = document.createElement("div");
  buttons.className = "fix-buttons";
  buttons.append(changeBack, keep);

  // the same fix all over this lecture can be kept in one go
  const fixedWord = word.textContent ?? "";
  const sameFixes = lineButtons.reduce(
    (count, button) =>
      count +
      [...button.querySelectorAll(".fixed-word")].filter(
        (other) => other.textContent?.toLowerCase() === fixedWord.toLowerCase(),
      ).length,
    0,
  );
  if (sameFixes > 1) {
    const keepAll = document.createElement("button");
    keepAll.type = "button";
    keepAll.className = "text-button";
    keepAll.textContent = `Keep all ${sameFixes}`;
    keepAll.addEventListener("click", () => updateAll(fixedWord));
    buttons.append(keepAll);
  }
  fixPopup.replaceChildren(label, buttons);
  fixPopup.hidden = false;
  placePopup(word.getBoundingClientRect());
  changeBack.focus();
}

function closeFixPopup() {
  fixPopup.hidden = true;
  fixPopupWord?.classList.remove("active");
  fixPopupWord = undefined;
}

async function updateAll(word: string) {
  const scrolled = transcriptList.scrollTop;
  try {
    drawLines(await invoke<Transcript>("keep_all_fixes", { path: currentPath, word }));
    transcriptList.scrollTop = scrolled;
  } catch (error) {
    statusText.textContent = String(error);
    closeFixPopup();
  }
}

async function updateFix(command: "change_back_fix" | "keep_fix", line: number, fix: number) {
  const scrolled = transcriptList.scrollTop;
  try {
    const transcript = await invoke<Transcript>(command, { path: currentPath, line, fix });
    drawLines(transcript);
    transcriptList.scrollTop = scrolled;
  } catch (error) {
    statusText.textContent = String(error);
    closeFixPopup();
  }
}

document.addEventListener("click", (event) => {
  // composedPath still has the popup when the button clicked was just swapped out for new ones
  const inside = event.composedPath().includes(fixPopup);
  if (!fixPopup.hidden && !inside && event.target !== fixPopupWord) closeFixPopup();
});
document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  // first the popup closes, then the spoken line goes back to math
  if (!fixPopup.hidden) {
    closeFixPopup();
  } else if (spokenLine >= 0) {
    showSpoken(-1);
  }
});
// a line showing its spoken words goes back to math when you click anywhere else
// (Escape works too, and so does finishing a fix)
document.addEventListener("click", (event) => {
  if (spokenLine < 0 || !fixPopup.hidden) return;
  const path = event.composedPath();
  if (lineButtons[spokenLine] && path.includes(lineButtons[spokenLine])) return;
  showSpoken(-1);
});

// double clicking a word opens the fix popup, so the Mac's own double-click
// word selection would just leave a highlight behind. dragging to select still works
transcriptList.addEventListener("mousedown", (event) => {
  if (event.detail > 1) event.preventDefault();
});

// it sits under the word, so it goes away once the words scroll out from under it
transcriptList.addEventListener("scroll", () => {
  if (Math.abs(transcriptList.scrollTop - fixPopupScroll) > 8) closeFixPopup();
});

function playFrom(index: number) {
  player.currentTime = segments[index].start_ms / 1000;
  player.play();
  followAgain();
}

// --- following along: the transcript keeps the playing line in view until you scroll yourself ---

const backToPlaying = document.querySelector<HTMLButtonElement>("#back-to-playing")!;
let following = true;

function stopFollowing() {
  following = false;
  updateBackToPlaying();
}

// pressing play, skipping or clicking a line brings it back to where you are, once.
// it goes by where the audio is now, not the line that was lit before you clicked
function followAgain() {
  following = true;
  showPlayingLine();
  updateBackToPlaying();
}

function updateBackToPlaying() {
  backToPlaying.hidden = following || player.paused || currentLine < 0;
}

// only things you do count, the smooth scrolling the app does itself doesn't
transcriptList.addEventListener("wheel", stopFollowing, { passive: true });
transcriptList.addEventListener("touchmove", stopFollowing, { passive: true });
transcriptList.addEventListener("pointerdown", (event) => {
  // grabbing the scrollbar
  if (event.target === transcriptList) stopFollowing();
});
transcriptList.addEventListener("keydown", (event) => {
  if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"].includes(event.key)) stopFollowing();
});
backToPlaying.addEventListener("click", () => {
  followAgain();
  lineButtons[currentLine]?.scrollIntoView({ block: "center", behavior: "smooth" });
});
player.addEventListener("play", followAgain);
player.addEventListener("pause", updateBackToPlaying);
player.addEventListener("seeked", followAgain);

// highlight whichever line is playing right now, and keep it in view while following
function showPlayingLine() {
  const nowMs = player.currentTime * 1000;
  const playing = segments.findIndex((segment) => nowMs >= segment.start_ms && nowMs < segment.end_ms);
  if (playing !== currentLine) {
    lineButtons[currentLine]?.classList.remove("current");
    currentLine = playing;
    lineButtons[currentLine]?.classList.add("current");
  }
  if (following) lineButtons[currentLine]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
}

player.addEventListener("timeupdate", () => {
  skipHidden();
  const before = currentLine;
  if (following) {
    // only scroll when the line changes, not on every tick
    const nowMs = player.currentTime * 1000;
    const segment = segments[currentLine];
    if (segment && nowMs >= segment.start_ms && nowMs < segment.end_ms) return updateBackToPlaying();
  }
  showPlayingLine();
  if (before !== currentLine) updateBackToPlaying();
});

// --- the play bar ---

function togglePlaying() {
  if (player.paused) {
    player.play();
  } else {
    player.pause();
  }
}

function updatePlayButton() {
  playButton.classList.toggle("playing", !player.paused);
  playButton.setAttribute("aria-label", player.paused ? "Play" : "Pause");
}

function skipBy(seconds: number) {
  const end = player.duration || 0;
  player.currentTime = Math.min(end, Math.max(0, player.currentTime + seconds));
}

// the ember part of the track is drawn from --progress
function updateScrubFill() {
  const end = Number(scrubber.max);
  const percent = end > 0 ? (Number(scrubber.value) / end) * 100 : 0;
  scrubber.style.setProperty("--progress", `${percent}%`);
}

// little ember ticks on the track where each mark starts, click one to jump there
function drawScrubMarks() {
  scrubMarks.replaceChildren();
  const durationMs = player.duration * 1000;
  if (!durationMs) return;

  for (const startMs of markStarts) {
    const tick = document.createElement("button");
    tick.type = "button";
    tick.className = "scrub-mark";
    tick.style.left = `${(startMs / durationMs) * 100}%`;
    tick.setAttribute("aria-label", `Play your mark at ${formatTime(startMs)}`);
    tick.title = `Your mark, ${formatTime(startMs)}`;
    tick.addEventListener("click", () => {
      player.currentTime = startMs / 1000;
      player.play();
    });
    scrubMarks.append(tick);
  }

}

playButton.addEventListener("click", togglePlaying);
backButton.addEventListener("click", () => skipBy(-15));
forwardButton.addEventListener("click", () => skipBy(15));
player.addEventListener("play", updatePlayButton);
player.addEventListener("pause", updatePlayButton);
player.addEventListener("ended", updatePlayButton);

player.addEventListener("loadedmetadata", () => {
  scrubber.max = String(player.duration);
  totalTimeText.textContent = formatTime(player.duration * 1000);
  updateScrubFill();
  drawScrubMarks();
});

player.addEventListener("timeupdate", () => {
  if (scrubbing) return;
  scrubber.value = String(player.currentTime);
  currentTimeText.textContent = formatTime(player.currentTime * 1000);
  updateScrubFill();
});

// while dragging just show the time, then jump when they let go
scrubber.addEventListener("input", () => {
  scrubbing = true;
  currentTimeText.textContent = formatTime(Number(scrubber.value) * 1000);
  updateScrubFill();
});

scrubber.addEventListener("change", () => {
  player.currentTime = Number(scrubber.value);
  scrubbing = false;
});

speedButton.addEventListener("click", () => {
  speedIndex = (speedIndex + 1) % playbackSpeeds.length;
  const speed = playbackSpeeds[speedIndex];
  player.playbackRate = speed;
  speedButton.textContent = `${speed}×`;
});

// space plays and pauses, unless you're typing or on a button
document.addEventListener("keydown", (event) => {
  if (event.code !== "Space" || playback.hidden) return;
  const target = event.target as HTMLElement;
  if (target.closest("input, textarea, button")) return;
  event.preventDefault();
  togglePlaying();
});

async function openLecture(path: string, jumpToLine?: number) {
  try {
    const transcript = await invoke<Transcript>("open_lecture", { path });
    // opening a lecture from the sidebar closes the model screen (unless there's no model yet)
    if (!setup.hidden && modelStatus?.current) hideSetup();
    statusText.textContent = "";
    if (!coursePage.hidden) closeCourse();
    showTranscript(path, transcript);
    setSaveTo(placeOfLecture(path));

    if (jumpToLine !== undefined) {
      // a search result goes to that line, not where you left off
      resumeSpot = false;
      lineButtons[jumpToLine]?.scrollIntoView({ block: "center" });
      playFrom(jumpToLine);
    }
  } catch (error) {
    statusText.textContent = String(error);
  }
}

// --- sidebar: saved lectures and search ---

function highlightOpenLecture() {
  for (const row of lectureList.querySelectorAll<HTMLElement>(".lecture-item, .lecture-row")) {
    row.classList.toggle("open", row.dataset.path === currentPath);
  }
}

async function refreshLectureList() {
  const [lectures, allPlaces] = await Promise.all([
    invoke<LectureSummary[]>("list_lectures"),
    invoke<Place[]>("list_places"),
  ]);
  places = allPlaces;
  updateRecordInto();
  lectureList.replaceChildren();

  // courses and folders first, then whatever isn't in one (new recordings land there)
  for (const place of places.filter((place) => !place.path.includes("/"))) {
    lectureList.append(buildPlace(place, lectures));
  }
  for (const lecture of lectures.filter((lecture) => !lecture.folder)) {
    lectureList.append(editing ? buildEditableLecture(lecture) : buildLecture(lecture));
  }

  sidebarEmpty.textContent = "Your lectures will show up here after you record one.";
  sidebarEmpty.hidden = lectures.length > 0 || places.length > 0 || !searchResults.hidden;
  highlightOpenLecture();
}

// --- organizing the list: courses, folders, names, moving and deleting ---
// both are real folders in Documents/Lecture, so Finder shows the same thing.
// courses sit at the top and can have folders inside them, one level deep

let editing = false;
let places: Place[] = [];
const closedFolders = new Set<string>(loadClosedFolders());

function loadClosedFolders(): string[] {
  try {
    return JSON.parse(localStorage.getItem("closedFolders") ?? "[]");
  } catch {
    return [];
  }
}

function saveClosedFolders() {
  try {
    localStorage.setItem("closedFolders", JSON.stringify([...closedFolders]));
  } catch {
    // forgetting which folders were closed isn't a problem
  }
}

// "Math 172 › Homework" for menus
function placeLabel(path: string) {
  return path.split("/").join(" › ");
}

// what a lecture is called until you name it.
// TODO: have the on-device language model name it from the transcript once that's in (v0.5)
const UNTITLED = "Untitled lecture";

function lectureName(lecture: LectureSummary) {
  return lecture.title || UNTITLED;
}

function showLectureName(title: string | null | undefined) {
  lectureTitle.textContent = title || UNTITLED;
}

// "Mon, Nov 16, 11:06 AM · 47:38 · 3 marks"
function lectureDetails(lecture: LectureSummary) {
  let details = `${formatDate(lecture.recorded_at)} · ${formatTime(lecture.duration_ms)}`;
  if (lecture.mark_count > 0) {
    details += ` · ${lecture.mark_count} ${lecture.mark_count === 1 ? "mark" : "marks"}`;
  }
  return details;
}

function svgIcon(path: string) {
  return `<svg viewBox="0 0 20 20" aria-hidden="true">${path}</svg>`;
}

const icons = {
  chevron: svgIcon('<path d="M7.5 5 12.5 10l-5 5" />'),
  grip: svgIcon('<circle cx="7.5" cy="5" r="1" /><circle cx="12.5" cy="5" r="1" /><circle cx="7.5" cy="10" r="1" /><circle cx="12.5" cy="10" r="1" /><circle cx="7.5" cy="15" r="1" /><circle cx="12.5" cy="15" r="1" />'),
  pencil: svgIcon('<path d="M12.5 4.5l3 3L8 15H5v-3z" />'),
  folder: svgIcon('<path d="M3 6.5V15h14V7.5H9.5L8 5.5H3z" />'),
  folderOpen: svgIcon('<path d="M3 15V5.5h5l1.5 2H15V9" /><path d="M3 15l2.2-6H18l-2.2 6z" />'),
  course: svgIcon('<path d="M5.5 3.5H15v13H5.5A1.5 1.5 0 0 1 4 15V5a1.5 1.5 0 0 1 1.5-1.5z" /><path d="M4 15a1.5 1.5 0 0 1 1.5-1.5H15" />'),
  courseOpen: svgIcon('<path d="M10 6c-1.5-1.2-3.5-1.5-6-1.5v10c2.5 0 4.5.3 6 1.5 1.5-1.2 3.5-1.5 6-1.5v-10c-2.5 0-4.5.3-6 1.5z" /><path d="M10 6v10" />'),
  words: svgIcon('<path d="M4 5.5h9M4 10h12M4 14.5h7" /><circle cx="15.5" cy="5.5" r="1.5" /><circle cx="13.5" cy="14.5" r="1.5" />'),
  newFolder: svgIcon('<path d="M3 6.5V15h14V7.5H9.5L8 5.5H3z" /><path d="M10 9.5v4M8 11.5h4" />'),
  trash: svgIcon('<path d="M4.5 6h11M8 6V4.5h4V6M6 6l.7 9.5h6.6L14 6" />'),
};

function iconButton(icon: string, label: string) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "icon-button";
  button.innerHTML = icon;
  button.setAttribute("aria-label", label);
  button.title = label;
  return button;
}

function buildLecture(lecture: LectureSummary) {
  const item = document.createElement("li");
  const button = document.createElement("button");
  const name = document.createElement("span");
  const preview = document.createElement("span");
  const details = document.createElement("span");

  button.type = "button";
  button.className = "lecture-item";
  button.dataset.path = lecture.path;
  name.className = "lecture-date";
  preview.className = "lecture-preview";
  details.className = "lecture-details";

  name.textContent = lectureName(lecture);
  preview.textContent = lecture.preview;
  details.textContent = lectureDetails(lecture);

  button.append(name);
  if (lecture.preview) button.append(preview);
  button.append(details);
  button.addEventListener("click", () => openLecture(lecture.path));
  item.append(button);
  return item;
}

// in edit mode clicking a lecture still opens it (so you can see what you're about to delete),
// and it can be renamed, dragged, added to a course or folder, and deleted
function buildEditableLecture(lecture: LectureSummary) {
  const name = lectureName(lecture);
  const item = document.createElement("li");
  item.className = "lecture-row";
  item.dataset.path = lecture.path;
  if (lecture.folder) item.dataset.folder = lecture.folder;

  const grip = document.createElement("span");
  grip.className = "drag-handle";
  grip.innerHTML = icons.grip;

  const text = document.createElement("div");
  text.className = "edit-text";
  const title = document.createElement("span");
  title.className = "row-name";
  title.textContent = name;
  const details = document.createElement("span");
  details.className = "lecture-details";
  details.textContent = lectureDetails(lecture);
  text.append(title, details);

  const rename = iconButton(icons.pencil, `Rename ${name}`);
  rename.addEventListener("click", () => {
    editInPlace(title, lecture.title ?? "", "Name this lecture", async (newTitle) => {
      await invoke("rename_lecture", { path: lecture.path, title: newTitle });
      if (currentPath === lecture.path) showLectureName(newTitle);
    });
  });

  const addTo = document.createElement("select");
  addTo.className = "move-select";
  addTo.setAttribute("aria-label", `Add ${name} to a course or folder`);
  addTo.add(new Option("Add to", "", true, true));
  addTo.options[0].disabled = true;
  if (lecture.folder) addTo.add(new Option("No course or folder", ""));
  for (const place of places) {
    if (place.path !== lecture.folder) addTo.add(new Option(placeLabel(place.path), place.path));
  }
  addTo.addEventListener("change", () => moveLecture(lecture.path, addTo.value || null));

  // a folder icon with the real menu laid invisibly over it, so it opens the normal Mac menu
  const addToButton = document.createElement("span");
  addToButton.className = "icon-button move-button";
  addToButton.title = "Add to a course or folder";
  addToButton.innerHTML = icons.folder;
  addToButton.append(addTo);
  addToButton.hidden = addTo.options.length === 1;

  const remove = iconButton(icons.trash, `Delete ${name}`);
  askFirst(remove, "Delete?", async () => {
    await invoke("delete_lecture", { path: lecture.path });
    if (currentPath === lecture.path) forgetOpenLecture();
    statusText.textContent = "Moved to the Trash. You can put it back from the Trash in Finder";
    await refreshLectureList();
  });

  item.addEventListener("pointerdown", (event) => startPress(event, item));
  item.addEventListener("click", (event) => {
    if ((event.target as HTMLElement).closest("button, select, input")) return;
    openLecture(lecture.path);
  });

  const actions = document.createElement("div");
  actions.className = "row-actions";
  actions.append(rename, addToButton, remove);
  item.append(grip, text, actions);
  return item;
}

// a course or folder, with the folders inside it (if it's at the top) and its lectures
function buildPlace(place: Place, lectures: LectureSummary[]) {
  const inside = places.filter((other) => other.path.startsWith(`${place.path}/`));
  const own = lectures.filter((lecture) => lecture.folder === place.path);
  const total = own.length + lectures.filter((lecture) => lecture.folder?.startsWith(`${place.path}/`)).length;

  const item = document.createElement("li");
  item.className = place.course ? "folder course" : "folder";
  item.dataset.folder = place.path;
  const open = !closedFolders.has(place.path);

  const header = document.createElement("div");
  header.className = "folder-header";

  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "folder-toggle";
  toggle.setAttribute("aria-expanded", String(open));
  const chevron = document.createElement("span");
  chevron.className = "chevron";
  chevron.innerHTML = icons.chevron;
  const kind = document.createElement("span");
  kind.className = "place-icon";
  if (place.course) kind.innerHTML = open ? icons.courseOpen : icons.course;
  else kind.innerHTML = open ? icons.folderOpen : icons.folder;
  const name = document.createElement("span");
  name.className = "folder-name";
  name.textContent = place.name;
  const count = document.createElement("span");
  count.className = "folder-count";
  count.textContent = String(total);
  toggle.append(chevron, kind, name, count);
  toggle.addEventListener("click", () => {
    if (closedFolders.has(place.path)) closedFolders.delete(place.path);
    else closedFolders.add(place.path);
    saveClosedFolders();
    refreshLectureList();
  });
  const actions = document.createElement("div");
  actions.className = "row-actions";
  header.append(toggle, actions);

  if (place.course) {
    const words = iconButton(icons.words, `${place.name} word fixes`);
    words.classList.add("course-button");
    words.addEventListener("click", () => openCourse(place.path));
    actions.append(words);
  }

  const list = document.createElement("ul");
  list.className = "folder-lectures";

  if (editing) {
    // a folder inside, only from the top level (one level deep)
    if (!place.path.includes("/")) {
      const addFolder = iconButton(icons.newFolder, `New folder in ${place.name}`);
      addFolder.addEventListener("click", () => {
        closedFolders.delete(place.path);
        list.hidden = false;
        askForName(list, "Folder name", (newName) =>
          invoke("create_place", { name: newName, parent: place.path, course: false }),
        );
      });
      actions.append(addFolder);
    }

    // a course can go back to being a plain folder (its lectures stay)
    if (place.course) {
      const makeFolder = iconButton(icons.folder, `Make ${place.name} a folder`);
      askFirst(makeFolder, "Make it a folder?", async () => {
        await invoke("make_folder", { folder: place.path });
        if (openCourseName === place.path && !coursePage.hidden) closeCourse();
        await refreshLectureList();
      });
      actions.append(makeFolder);
    }

    // a plain folder at the top can become a course
    if (!place.course && !place.path.includes("/")) {
      const makeCourse = iconButton(icons.course, `Make ${place.name} a course`);
      makeCourse.addEventListener("click", async () => {
        try {
          await invoke("make_course", { folder: place.path });
          await refreshLectureList();
        } catch (error) {
          statusText.textContent = String(error);
        }
      });
      actions.append(makeCourse);
    }

    const rename = iconButton(icons.pencil, `Rename ${place.name}`);
    rename.addEventListener("click", () => {
      editInPlace(name, place.name, place.course ? "Course name" : "Folder name", async (newName) => {
        const renamed = await invoke<string>("rename_place", { path: place.path, newName });
        if (closedFolders.delete(place.path)) closedFolders.add(renamed);
        saveClosedFolders();
        // the open lecture's path has the name in it
        if (currentPath.includes(`/${place.path}/`)) await openLecture(currentPath.replace(`/${place.path}/`, `/${renamed}/`));
      });
    });

    const remove = iconButton(icons.trash, `Remove ${place.name}`);
    askFirst(remove, total > 0 ? "Remove? Lectures stay" : "Remove?", async () => {
      await invoke("delete_place", { path: place.path });
      closedFolders.delete(place.path);
      saveClosedFolders();
      if (currentPath.includes(`/${place.path}/`)) forgetOpenLecture();
      await refreshLectureList();
    });
    actions.append(rename, remove);
  }

  list.hidden = !open || (own.length === 0 && inside.length === 0);
  for (const inner of inside) list.append(buildPlace(inner, lectures));
  for (const lecture of own) list.append(editing ? buildEditableLecture(lecture) : buildLecture(lecture));

  item.append(header, list);
  return item;
}

// a text box at the top of a list for naming something new
function askForName(list: HTMLElement, placeholder: string, create: (name: string) => Promise<unknown>) {
  const item = document.createElement("li");
  const spot = document.createElement("span");
  item.append(spot);
  list.prepend(item);
  editInPlace(spot, "", placeholder, async (name) => {
    if (name) await create(name);
  });
}

// --- dragging lectures ---
// each list is basically an array, picking a lecture up leaves a gap that follows the mouse.
// using plain mouse events since the web view's drag and drop didn't work

const sidebarScroll = document.querySelector<HTMLElement>(".sidebar-scroll")!;
const DRAG_START_DISTANCE = 5;
const SCROLL_EDGE = 56;
const FASTEST_SCROLL = 14; // px per frame, right at the edge
const SLIDE_MS = 180;

type Drag = {
  row: HTMLElement;
  path: string;
  fromFolder: string | null;
  startY: number;
  startX: number;
  offsetX: number;
  offsetY: number;
  pointerId: number;
  lastX: number;
  lastY: number;
  ghost?: HTMLElement;
  // a closed or empty folder that got opened so it could be dropped into
  openedList?: HTMLElement;
};

let drag: Drag | undefined;
let scrollFrame = 0;
// a drag ends with a click on whatever was under the mouse, this swallows it
let swallowNextClick = false;

function startPress(event: PointerEvent, row: HTMLElement) {
  if (!editing || event.button !== 0 || drag) return;
  // the move menu, delete and the rename box keep working as normal
  if ((event.target as HTMLElement).closest("select, input, .icon-button")) return;

  const box = row.getBoundingClientRect();
  drag = {
    row,
    path: row.dataset.path ?? "",
    fromFolder: row.dataset.folder ?? null,
    startX: event.clientX,
    startY: event.clientY,
    offsetX: event.clientX - box.left,
    offsetY: event.clientY - box.top,
    pointerId: event.pointerId,
    lastX: event.clientX,
    lastY: event.clientY,
  };
}

function beginDragging(current: Drag) {
  const box = current.row.getBoundingClientRect();
  const ghost = current.row.cloneNode(true) as HTMLElement;
  ghost.classList.add("drag-ghost");
  ghost.style.width = `${box.width}px`;
  document.body.append(ghost);
  current.ghost = ghost;
  // the row itself stays in the list as the empty slot
  current.row.classList.add("drop-slot");
  document.body.classList.add("dragging-lecture");
}

// the copy follows the mouse but never leaves the list area (under New folder, above the bottom)
function placeGhost(current: Drag) {
  if (!current.ghost) return;
  const area = sidebarScroll.getBoundingClientRect();
  const height = current.ghost.offsetHeight;
  const x = Math.min(Math.max(current.lastX - current.offsetX, area.left), area.right - current.ghost.offsetWidth);
  const y = Math.min(Math.max(current.lastY - current.offsetY, area.top), area.bottom - height);
  current.ghost.style.transform = `translate(${x}px, ${y}px)`;
}

// every row and folder in the list, for sliding them when the slot moves
function slidingItems() {
  return [...lectureList.querySelectorAll<HTMLElement>(".lecture-row, .folder-header")].filter(
    (item) => !item.classList.contains("drop-slot"),
  );
}

// moves the slot and slides the other rows into place
function moveSlot(current: Drag, list: HTMLElement, before: Element | null) {
  if (current.row.parentElement === list && current.row.nextElementSibling === before) return;

  const items = slidingItems();
  const oldTops = new Map(items.map((item) => [item, item.getBoundingClientRect().top]));
  list.insertBefore(current.row, before);

  for (const item of items) {
    const shift = (oldTops.get(item) ?? 0) - item.getBoundingClientRect().top;
    if (Math.abs(shift) < 1) continue;
    item.animate([{ transform: `translateY(${shift}px)` }, { transform: "translateY(0)" }], {
      duration: SLIDE_MS,
      easing: "cubic-bezier(0.23, 1, 0.32, 1)",
    });
  }
}

// the rows in a list the slot can go between (not the slot itself, not folders)
function rowsIn(list: HTMLElement) {
  return [...list.children].filter(
    (child): child is HTMLElement => child.classList.contains("lecture-row") && !child.classList.contains("drop-slot"),
  );
}

// the first row whose middle is below the mouse, the slot goes right before it
function rowAfter(list: HTMLElement, y: number) {
  return rowsIn(list).find((row) => {
    const box = row.getBoundingClientRect();
    return y < box.top + box.height / 2;
  }) ?? null;
}

// works out where the lecture would land and moves the slot there
function updateSlot(current: Drag) {
  const area = sidebarScroll.getBoundingClientRect();
  // the copy can't leave the list, so neither can where it lands
  const x = Math.min(Math.max(current.lastX, area.left + 1), area.right - 1);
  const y = Math.min(Math.max(current.lastY, area.top + 1), area.bottom - 1);

  // ignore the slot itself when looking at what's under the mouse
  current.row.style.pointerEvents = "none";
  const under = document.elementFromPoint(x, y) as HTMLElement | null;
  current.row.style.pointerEvents = "";
  if (!under) return;

  const folder = under.closest<HTMLElement>(".folder");
  if (folder) {
    const list = folder.querySelector<HTMLElement>(":scope > .folder-lectures")!;
    // over a closed or empty folder's name: open it so there's somewhere to drop
    if (list.hidden) {
      if (current.openedList && current.openedList !== list) current.openedList.hidden = true;
      list.hidden = false;
      current.openedList = list;
    }
    const overHeader = under.closest(".folder-header")?.parentElement === folder;
    moveSlot(current, list, overHeader ? (rowsIn(list)[0] ?? null) : rowAfter(list, y));
    return;
  }

  // main list, but skip the little gaps between folders so the slot doesn't jump around
  const looseRow = under.closest(".lecture-row")?.parentElement === lectureList;
  const belowList = under === sidebarScroll;
  if (looseRow || belowList) moveSlot(current, lectureList, rowAfter(lectureList, y));
}

// scrolls the list when you hold near the top or bottom (only after moving toward that edge)
function autoScroll() {
  scrollFrame = 0;
  if (!drag?.ghost) return;

  const box = sidebarScroll.getBoundingClientRect();
  const fromTop = drag.lastY - box.top;
  const fromBottom = box.bottom - drag.lastY;
  const topZone = Math.min(SCROLL_EDGE, drag.startY - box.top - 4);
  const bottomZone = Math.min(SCROLL_EDGE, box.bottom - drag.startY - 4);
  const closeness = (distance: number, zone: number) => Math.min(1, Math.max(0, 1 - distance / zone));
  let speed = 0;
  if (fromTop < topZone) speed = -FASTEST_SCROLL * closeness(fromTop, topZone);
  else if (fromBottom < bottomZone) speed = FASTEST_SCROLL * closeness(fromBottom, bottomZone);

  const bottom = sidebarScroll.scrollHeight - sidebarScroll.clientHeight;
  const next = Math.min(bottom, Math.max(0, sidebarScroll.scrollTop + speed));
  if (speed === 0 || Math.abs(next - sidebarScroll.scrollTop) < 0.5) return;

  sidebarScroll.scrollTop = next;
  // the list moved under the mouse, so where it would land may have changed
  updateSlot(drag);
  scrollFrame = requestAnimationFrame(autoScroll);
}

// the recordings' file names in a list, top to bottom, with the slot where it is
function filesIn(list: HTMLElement) {
  return [...list.children]
    .filter((child): child is HTMLElement => child.classList.contains("lecture-row"))
    .map((row) => (row.dataset.path ?? "").split("/").pop() ?? "");
}

async function endDrag(drop: boolean) {
  const current = drag;
  drag = undefined;
  cancelAnimationFrame(scrollFrame);
  scrollFrame = 0;
  if (!current?.ghost) return;

  swallowNextClick = true;
  setTimeout(() => (swallowNextClick = false), 0);
  current.ghost.remove();
  document.body.classList.remove("dragging-lecture");

  if (!drop) {
    await refreshLectureList();
    return;
  }

  const list = current.row.parentElement as HTMLElement;
  const toFolder = list.closest<HTMLElement>(".folder")?.dataset.folder ?? null;
  const files = filesIn(list);
  current.row.classList.remove("drop-slot");

  try {
    if (toFolder !== current.fromFolder) {
      const newPath = await invoke<string>("move_lecture", { path: current.path, folder: toFolder });
      if (toFolder) closedFolders.delete(toFolder);
      saveClosedFolders();
      if (currentPath === current.path) await openLecture(newPath);
    }
    await invoke("save_order", { folder: toFolder, files });
  } catch (error) {
    statusText.textContent = String(error);
  }
  await refreshLectureList();
}

document.addEventListener("pointermove", (event) => {
  if (!drag || event.pointerId !== drag.pointerId) return;
  drag.lastX = event.clientX;
  drag.lastY = event.clientY;

  if (!drag.ghost) {
    const moved = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
    if (moved < DRAG_START_DISTANCE) return;
    beginDragging(drag);
  }
  event.preventDefault();
  placeGhost(drag);
  updateSlot(drag);
  if (!scrollFrame) scrollFrame = requestAnimationFrame(autoScroll);
});

document.addEventListener("pointerup", (event) => {
  if (!drag || event.pointerId !== drag.pointerId) return;
  if (drag.ghost) endDrag(true);
  else drag = undefined;
});
document.addEventListener("pointercancel", () => endDrag(false));
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && drag) endDrag(false);
});
window.addEventListener("blur", () => {
  if (drag) endDrag(false);
});
document.addEventListener(
  "click",
  (event) => {
    if (!swallowNextClick) return;
    swallowNextClick = false;
    event.stopPropagation();
    event.preventDefault();
  },
  true,
);

async function moveLecture(path: string, folder: string | null) {
  try {
    const newPath = await invoke<string>("move_lecture", { path, folder });
    if (folder) closedFolders.delete(folder);
    saveClosedFolders();
    if (currentPath === path) await openLecture(newPath);
    await refreshLectureList();
  } catch (error) {
    statusText.textContent = String(error);
  }
}

// swaps some text for a text box. enter or clicking away saves, escape cancels
function editInPlace(shown: HTMLElement, value: string, placeholder: string, save: (value: string) => Promise<void>) {
  const input = document.createElement("input");
  input.className = "inline-input";
  input.value = value;
  input.placeholder = placeholder;
  input.setAttribute("aria-label", placeholder);
  shown.replaceWith(input);
  input.focus();
  input.select();

  let finished = false;
  async function finish(keep: boolean) {
    if (finished) return;
    finished = true;
    try {
      if (keep && input.value.trim() !== value) await save(input.value.trim());
    } catch (error) {
      statusText.textContent = String(error);
    }
    await refreshLectureList();
  }

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") finish(true);
    if (event.key === "Escape") finish(false);
  });
  input.addEventListener("blur", () => finish(true));
}

// asks once before deleting, resets after a few seconds
function askFirst(button: HTMLButtonElement, question: string, action: () => Promise<void>) {
  const normal = button.innerHTML;
  let timeout: number | undefined;

  button.addEventListener("click", async () => {
    if (!button.classList.contains("confirming")) {
      button.classList.add("confirming");
      button.textContent = question;
      timeout = setTimeout(() => {
        button.classList.remove("confirming");
        button.innerHTML = normal;
      }, 4000);
      return;
    }
    clearTimeout(timeout);
    try {
      await action();
    } catch (error) {
      statusText.textContent = String(error);
    }
  });
}

function setEditing(on: boolean) {
  editing = on;
  lectureList.classList.toggle("editing", on);
  editListButton.textContent = on ? "Done" : "Edit";
  editListButton.setAttribute("aria-pressed", String(on));
  editBar.hidden = !on;
  refreshLectureList();
}

editListButton.addEventListener("click", () => setEditing(!editing));

// a new course opens its page right away, so word fixes can go in
newCourseButton.addEventListener("click", () => {
  askForName(lectureList, "Course name", async (name) => {
    const path = await invoke<string>("create_place", { name, parent: null, course: true });
    await refreshLectureList();
    await openCourse(path);
  });
});

newFolderButton.addEventListener("click", () => {
  askForName(lectureList, "Folder name", (name) => invoke("create_place", { name, parent: null, course: false }));
});

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

// the open lecture is gone (deleted), so clear the player and the transcript
function forgetOpenLecture() {
  player.pause();
  player.removeAttribute("src");
  currentPath = "";
  segments = [];
  playback.hidden = true;
}

exportButton.addEventListener("click", async () => {
  try {
    const exportedPath = await invoke<string>("export_lecture", {
      path: currentPath,
      title: `${lectureTitle.textContent}, ${lectureWhen.textContent}`,
      math: showingMath(),
    });
    // just the file name, the full path has your Mac username in it
    const fileName = exportedPath.split("/").pop();
    statusText.textContent = `Exported ${fileName}, it's open in Finder`;
  } catch (error) {
    statusText.textContent = String(error);
  }
});

// whisper reports how far it has gotten while transcribing a saved lecture
let transcribingTotalMs = 0;
let transcribingLabel = "";

listen<number>("transcribe-progress", (event) => {
  if (!transcribingLabel) return;
  const done = Math.min(event.payload, transcribingTotalMs);
  statusText.textContent = `${transcribingLabel}: ${formatTime(done)} of ${formatTime(transcribingTotalMs)}`;
});

// runs the lecture through the speech model again, the old transcript is kept as .before.json
retranscribeButton.addEventListener("click", async () => {
  const path = currentPath;
  retranscribeButton.disabled = true;
  recordButton.disabled = true;
  retranscribeButton.textContent = "Transcribing...";
  statusText.textContent = "Transcribing this lecture again. A 75 minute class takes about 6 to 8 minutes";
  transcribingTotalMs = player.duration > 0 ? player.duration * 1000 : (segments[segments.length - 1]?.end_ms ?? 0);
  transcribingLabel = "Transcribing again";
  const started = Date.now();

  try {
    const transcript = await invoke<Transcript>("retranscribe", { path });
    transcribingLabel = "";
    // only show it if they're still looking at the same lecture
    if (currentPath === path) showTranscript(path, transcript);
    if (SHOW_UNSURE) crossCheck(path);
    const seconds = Math.round((Date.now() - started) / 1000);
    statusText.textContent = `Transcribed again in ${seconds}s. The old version is saved next to it as .before.json`;
    await refreshLectureList();
  } catch (error) {
    transcribingLabel = "";
    statusText.textContent = String(error);
  } finally {
    retranscribeButton.disabled = false;
    recordButton.disabled = false;
    retranscribeButton.textContent = "Transcribe again";
  }
});

// --- recording ---

// only needed if it couldn't transcribe during class (like if the model wasn't installed yet)
async function transcribe(path: string, marks: number[], totalMs: number) {
  statusText.textContent = "Transcribing...";
  transcribingTotalMs = totalMs;
  transcribingLabel = "Transcribing";
  const started = Date.now();

  try {
    const transcript = await invoke<Transcript>("transcribe", { path, marks });
    const seconds = Math.round((Date.now() - started) / 1000);
    statusText.textContent = `Transcribed in ${seconds}s`;
    showTranscript(path, transcript);
    if (SHOW_UNSURE) crossCheck(path);
  } finally {
    transcribingLabel = "";
  }
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
      const recordedMs = Date.now() - startTime;
      statusText.textContent = "Finishing the transcript...";
      const stopped = await invoke<StoppedRecording>("stop_recording");
      setRecording(false);
      progressText.textContent = "";

      if (stopped.transcript) {
        const markNote = stopped.marks.length > 0 ? `, ${stopped.marks.length} marked` : "";
        statusText.textContent = `Transcript ready${markNote}`;
        showTranscript(stopped.path, stopped.transcript);
        if (SHOW_UNSURE) crossCheck(stopped.path);
      } else {
        await transcribe(stopped.path, stopped.marks, recordedMs);
      }
      await refreshLectureList();
    } else {
      // the tick sound needs this, and it has to be made after a click
      audioContext ??= new AudioContext();

      // start recording and the timer
      player.pause();
      await invoke("start_recording", { folder: recordInto.value || null });
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

// --- courses ---
// any folder can be a course. its word list fixes how those words come out in its lectures

type CourseWord = { word: string; heard_as: string[] };
type CourseExam = { name: string; date: string };
type ExamChoice = { file: string; at_ms: number; exam: string | null; added: boolean };
type Course = { subject: string | null; words: CourseWord[]; exams?: CourseExam[]; exam_choices?: ExamChoice[] };

let openCourseName = "";
let openCourseSettings: Course = { subject: null, words: [] };

// which course a new recording goes into, remembered between launches
// "Save to" follows whatever you're looking at: open a lecture in Math 172 (or its course page)
// and new recordings go there too. you can still pick something else before recording
function setSaveTo(place: string | null) {
  if (recordButton.classList.contains("recording")) return;
  recordInto.value = place && places.some((known) => known.path === place) ? place : "";
}

// the course or folder a lecture is in, from its path ("…/Math 172/Homework/lecture-1.wav")
function placeOfLecture(path: string) {
  const folder = path.slice(0, path.lastIndexOf("/"));
  const matches = places.filter((place) => folder.endsWith(`/${place.path}`));
  // the longest match, so a lecture in "Math 172/Homework" isn't put in "Math 172"
  return matches.sort((a, b) => b.path.length - a.path.length)[0]?.path ?? null;
}

function updateRecordInto() {
  const wanted = recordInto.value;
  recordInto.replaceChildren(
    new Option("Lectures", ""),
    ...places.map((place) => new Option(placeLabel(place.path), place.path)),
  );
  recordInto.value = places.some((place) => place.path === wanted) ? wanted : "";
  recordIntoLabel.hidden = places.length === 0;
}


async function openCourse(folder: string) {
  if (recordButton.classList.contains("recording")) return;
  try {
    openCourseSettings = await invoke<Course>("course_settings", { folder });
  } catch (error) {
    statusText.textContent = String(error);
    return;
  }
  openCourseName = folder;
  courseName.textContent = folder;
  courseSubject.value = openCourseSettings.subject ?? "";
  courseStatus.textContent = "";
  setSaveTo(folder);
  drawWords();
  drawExamDates();
  loadCourseExams();

  setup.hidden = true;
  recorder.hidden = true;
  playback.hidden = true;
  coursePage.hidden = false;
  heardInput.focus();
}

function closeCourse() {
  coursePage.hidden = true;
  recorder.hidden = false;
  playback.hidden = segments.length === 0;
  // the open lecture may have just had words fixed
  if (currentPath.includes(`/${openCourseName}/`)) openLecture(currentPath);
}

function drawWords() {
  wordList.replaceChildren(
    ...openCourseSettings.words.map((entry, index) => {
      const item = document.createElement("li");
      item.className = "word-row";
      const text = document.createElement("div");
      text.className = "word-fix";
      const wrote = document.createElement("span");
      wrote.className = "wrote";
      wrote.textContent = entry.heard_as.length > 0 ? entry.heard_as.join(", ") : "capitals";
      if (entry.heard_as.length === 0) wrote.classList.add("plain");
      const arrow = document.createElement("span");
      arrow.className = "arrow";
      arrow.textContent = "→";
      arrow.setAttribute("aria-label", "becomes");
      const word = document.createElement("span");
      word.className = "word";
      word.textContent = entry.word;
      text.append(wrote, arrow, word);

      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "icon-button";
      remove.innerHTML = icons.trash;
      remove.setAttribute("aria-label", `Remove ${entry.word}`);
      remove.addEventListener("click", () => {
        openCourseSettings.words.splice(index, 1);
        saveCourse();
      });

      item.append(text, remove);
      return item;
    }),
  );
}

async function saveCourse() {
  try {
    const changed = await invoke<number>("save_course", { folder: openCourseName, course: openCourseSettings });
    openCourseSettings = await invoke<Course>("course_settings", { folder: openCourseName });
    drawWords();
    drawExamDates();
    loadCourseExams();
    courseStatus.textContent = changed > 0 ? `Fixed ${changed} ${changed === 1 ? "lecture" : "lectures"}` : "";
    await refreshLectureList();
  } catch (error) {
    courseStatus.textContent = String(error);
  }
}

wordForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const word = wordInput.value.trim();
  if (!word) {
    wordInput.focus();
    return;
  }
  const heardAs = heardInput.value.split(",").map((heard) => heard.trim()).filter(Boolean);

  // adding a word that's already there just adds to what it's heard as
  const existing = openCourseSettings.words.find((entry) => entry.word.toLowerCase() === word.toLowerCase());
  if (existing) existing.heard_as.push(...heardAs);
  else openCourseSettings.words.push({ word, heard_as: heardAs });

  wordInput.value = "";
  heardInput.value = "";
  heardInput.focus();
  saveCourse();
});

courseSubject.addEventListener("change", () => {
  openCourseSettings.subject = courseSubject.value || null;
  saveCourse();
});

courseBack.addEventListener("click", closeCourse);

// --- speech model: first launch and switching ---

type ModelChoice = "fast" | "accurate";

type ModelStatus = {
  current: ModelChoice | null;
  fast_downloaded: boolean;
  accurate_downloaded: boolean;
};

const modelLabels: Record<ModelChoice, string> = { fast: "Fast", accurate: "Most accurate" };
let modelStatus: ModelStatus | undefined;
let pickedModel: ModelChoice = "accurate";
let downloading = false;

function formatSize(bytes: number) {
  const megabytes = bytes / (1024 * 1024);
  return megabytes >= 1000 ? `${(megabytes / 1024).toFixed(1)} GB` : `${Math.round(megabytes)} MB`;
}

function isDownloaded(choice: ModelChoice) {
  return choice === "fast" ? modelStatus?.fast_downloaded : modelStatus?.accurate_downloaded;
}

function pickModel(choice: ModelChoice) {
  pickedModel = choice;
  modelChoices.forEach((button) => {
    button.setAttribute("aria-checked", String(button.dataset.choice === choice));
  });

  if (choice === modelStatus?.current) {
    downloadButton.textContent = "In use";
    downloadButton.disabled = true;
  } else {
    downloadButton.textContent = isDownloaded(choice) ? "Use this model" : "Download";
    downloadButton.disabled = false;
  }
}

function showSetup(firstLaunch: boolean) {
  coursePage.hidden = true;
  setup.hidden = false;
  recorder.hidden = true;
  playback.hidden = true;
  setupTitle.textContent = firstLaunch ? "Welcome to Lecture" : "Speech model";
  setupBackButton.hidden = firstLaunch;
  setupError.textContent = "";
  pickModel(modelStatus?.current ?? "accurate");
}

function hideSetup() {
  setup.hidden = true;
  recorder.hidden = false;
  playback.hidden = segments.length === 0;
}

async function refreshModelStatus() {
  modelStatus = await invoke<ModelStatus>("model_status");
  const current = modelStatus.current;
  modelButton.hidden = current === null;
  if (current) modelName.textContent = modelLabels[current];
  return modelStatus;
}

modelChoices.forEach((button) => {
  button.addEventListener("click", () => {
    if (!downloading) pickModel(button.dataset.choice as ModelChoice);
  });
});

listen<[number, number]>("download-progress", (event) => {
  const [done, total] = event.payload;
  if (total > 0) {
    downloadAmount.textContent = `${formatSize(done)} of ${formatSize(total)}`;
    downloadFill.style.width = `${Math.min(100, (done / total) * 100)}%`;
  } else {
    downloadAmount.textContent = formatSize(done);
  }
});

downloadButton.addEventListener("click", async () => {
  setupError.textContent = "";

  // already downloaded, just switch to it
  if (isDownloaded(pickedModel)) {
    await invoke("use_model", { choice: pickedModel });
    await refreshModelStatus();
    hideSetup();
    return;
  }

  downloading = true;
  downloadButton.disabled = true;
  setupBackButton.disabled = true;
  downloadButton.textContent = "Downloading...";
  downloadProgress.hidden = false;
  downloadAmount.textContent = "";
  downloadFill.style.width = "0%";

  try {
    await invoke("download_model", { choice: pickedModel });
    await refreshModelStatus();
    hideSetup();
    statusText.textContent = "All set. Press Record when class starts.";
  } catch (error) {
    setupError.textContent = String(error);
    downloadButton.disabled = false;
    downloadButton.textContent = "Try again";
  } finally {
    downloading = false;
    setupBackButton.disabled = false;
    downloadProgress.hidden = true;
  }
});

setupBackButton.addEventListener("click", hideSetup);

modelButton.addEventListener("click", () => {
  if (recordButton.classList.contains("recording")) return;
  showSetup(false);
});

refreshModelStatus()
  .then((status) => {
    if (status.current === null) showSetup(true);
  })
  .catch((error) => (statusText.textContent = String(error)));

refreshLectureList().catch((error) => (statusText.textContent = String(error)));

// --- the course page's Exams tab: your exam dates, and what was said about each exam ---

const tabWords = document.querySelector<HTMLButtonElement>("#tab-words")!;
const tabExams = document.querySelector<HTMLButtonElement>("#tab-exams")!;
const courseWordsTab = document.querySelector<HTMLElement>("#course-words")!;
const courseExamsTab = document.querySelector<HTMLElement>("#course-exams")!;
const examForm = document.querySelector<HTMLFormElement>("#exam-form")!;
const examNameInput = document.querySelector<HTMLInputElement>("#exam-name")!;
const examDateInput = document.querySelector<HTMLInputElement>("#exam-date")!;
const examDates = document.querySelector<HTMLUListElement>("#exam-dates")!;
const examGroups = document.querySelector<HTMLElement>("#exam-groups")!;

type CourseNote = {
  path: string;
  recorded_at: number;
  line: number;
  start_ms: number;
  said: string;
  not_on_exam: boolean;
  on_exam: boolean;
  date: boolean;
  material: boolean;
  chosen: boolean;
};
type ExamGroup = { name: string; date: string | null; past: boolean; notes: CourseNote[] };

function showCourseTab(exams: boolean) {
  courseWordsTab.hidden = exams;
  courseExamsTab.hidden = !exams;
  tabWords.classList.toggle("on", !exams);
  tabExams.classList.toggle("on", exams);
  tabWords.setAttribute("aria-selected", String(!exams));
  tabExams.setAttribute("aria-selected", String(exams));
}

tabWords.addEventListener("click", () => showCourseTab(false));
tabExams.addEventListener("click", () => showCourseTab(true));

// "2026-12-01" as "Tue, Dec 1"
function formatExamDate(date: string) {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(year, month - 1, day).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
}

function todayString() {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

function drawExamDates() {
  const exams = openCourseSettings.exams ?? [];
  examDates.replaceChildren(
    ...exams.map((exam, index) => {
      const item = document.createElement("li");
      const name = document.createElement("span");
      name.className = "exam-date-name";
      name.textContent = exam.name;
      const when = document.createElement("span");
      when.className = "exam-date-when";
      when.textContent = formatExamDate(exam.date);
      const edit = document.createElement("button");
      edit.type = "button";
      edit.className = "text-button";
      edit.textContent = "Edit";
      edit.addEventListener("click", () => editExam(item, exam));
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "text-button";
      remove.textContent = "Remove";
      remove.addEventListener("click", () => {
        openCourseSettings.exams = exams.filter((_, other) => other !== index);
        saveCourse();
      });
      item.append(name, when, edit, remove);
      return item;
    }),
  );
}

// the professor moved the exam, or you named it wrong. notes you put under it stay with it
function editExam(item: HTMLLIElement, exam: CourseExam) {
  const form = document.createElement("form");
  form.className = "exam-edit";
  const name = document.createElement("input");
  name.type = "text";
  name.value = exam.name;
  name.setAttribute("aria-label", "Exam");
  const date = document.createElement("input");
  date.type = "date";
  date.value = exam.date;
  date.setAttribute("aria-label", "Date");
  const save = document.createElement("button");
  save.type = "submit";
  save.className = "text-button";
  save.textContent = "Save";
  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "text-button";
  cancel.textContent = "Cancel";
  cancel.addEventListener("click", drawExamDates);
  form.append(name, date, save, cancel);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!name.value.trim() || !date.value) {
      courseStatus.textContent = "Give the exam a name and a date";
      return;
    }
    try {
      await invoke("rename_exam", { folder: openCourseName, old: exam.name, name: name.value.trim(), date: date.value });
      courseStatus.textContent = "";
      reloadCourse();
    } catch (error) {
      courseStatus.textContent = String(error);
    }
  });
  item.replaceChildren(form);
  name.focus();
}

examForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const name = examNameInput.value.trim();
  const date = examDateInput.value;
  if (!name || !date) {
    courseStatus.textContent = "Give the exam a name and a date";
    return;
  }
  openCourseSettings.exams = [...(openCourseSettings.exams ?? []), { name, date }];
  examNameInput.value = "";
  examDateInput.value = "";
  saveCourse();
  examNameInput.focus();
});

// after a choice is saved straight to the course file, read it back so saving
// something else on this page doesn't write over it
async function reloadCourse() {
  try {
    openCourseSettings = await invoke<Course>("course_settings", { folder: openCourseName });
    drawExamDates();
    loadCourseExams();
  } catch (error) {
    courseStatus.textContent = String(error);
  }
}

async function loadCourseExams() {
  const folder = openCourseName;
  try {
    const groups = await invoke<ExamGroup[]>("course_exam_notes", {
      folder,
      today: todayString(),
      minutesBehindUtc: new Date().getTimezoneOffset(),
    });
    if (folder === openCourseName) drawExamGroups(groups);
  } catch (error) {
    examGroups.textContent = String(error);
  }
}

function drawExamGroups(groups: ExamGroup[]) {
  if (groups.every((group) => group.notes.length === 0)) {
    const empty = document.createElement("p");
    empty.className = "course-tip";
    empty.textContent = "Nothing about exams yet. When your professor talks about an exam, it shows up here.";
    examGroups.replaceChildren(empty);
    return;
  }
  examGroups.replaceChildren(
    ...groups.map((group) => {
      // ones that already happened start folded closed
      const section = document.createElement("details");
      section.className = "exam-group";
      section.open = !group.past;
      const summary = document.createElement("summary");
      const name = document.createElement("span");
      name.className = "exam-group-name";
      name.textContent = group.name;
      summary.append(name);
      if (group.date) {
        const when = document.createElement("span");
        when.className = "exam-group-when";
        when.textContent = formatExamDate(group.date);
        summary.append(when);
      }
      const count = document.createElement("span");
      count.className = "exam-notes-count";
      count.textContent = String(group.notes.length);
      summary.append(count);

      const list = document.createElement("ol");
      list.className = "exam-notes-list";
      for (const note of group.notes) {
        const item = document.createElement("li");
        const button = document.createElement("button");
        button.type = "button";
        const time = document.createElement("time");
        time.className = "exam-note-when";
        const day = new Date(note.recorded_at * 1000).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
        time.textContent = `${day}, ${formatTime(note.start_ms)}`;
        const said = document.createElement("span");
        said.className = "exam-note-text";
        said.textContent = note.said;
        const tags = examTags(note);
        button.append(time, ...(tags ? [tags] : []), said);
        button.addEventListener("click", () => openLecture(note.path, note.line));
        item.append(button, moveMenu(note.path, note.start_ms, group.name, note.chosen, openCourseSettings.exams ?? [], reloadCourse));
        list.append(item);
      }
      section.append(summary, list);
      return section;
    }),
  );
}

