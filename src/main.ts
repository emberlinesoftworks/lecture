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
const retranscribeButton = document.querySelector<HTMLButtonElement>("#retranscribe")!;
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

  // a recording with nothing in it would otherwise just leave a blank screen
  if (segments.length === 0) {
    statusText.textContent =
      "Nothing was heard in this recording, so there's no transcript. If someone was talking, check your microphone in System Settings → Sound";
  }

  lectureTitle.textContent = formatDate(recordedAtFromPath(audioPath));
  player.src = convertFileSrc(audioPath);
  scrubber.value = "0";
  updateScrubFill();
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
    showTranscript(path, transcript);

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
      } else {
        await transcribe(stopped.path, stopped.marks, recordedMs);
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
