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
};

const recordButton = document.querySelector<HTMLButtonElement>("#record")!;
const buttonLabel = recordButton.querySelector<HTMLElement>(".label")!;
const markButton = document.querySelector<HTMLButtonElement>("#mark")!;
const timer = document.querySelector<HTMLElement>("#timer")!;
const statusText = document.querySelector<HTMLElement>("#status")!;
const playback = document.querySelector<HTMLElement>("#playback")!;
const player = document.querySelector<HTMLAudioElement>("#player")!;
const transcriptList = document.querySelector<HTMLOListElement>("#transcript")!;

let startTime = 0;
let timerInterval: number | undefined;
let markCount = 0;
let audioContext: AudioContext | undefined;
let segments: Segment[] = [];
let lineButtons: HTMLButtonElement[] = [];
let currentLine = -1;

// turns milliseconds into 03:25 (or 1:03:25 once it passes an hour)
function formatTime(milliseconds: number) {
  const totalSeconds = Math.floor(milliseconds / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = String(Math.floor(totalSeconds / 60) % 60).padStart(2, "0");
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${minutes}:${seconds}` : `${minutes}:${seconds}`;
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

// each line is a button, clicking it jumps the audio to when that was said
function showTranscript(audioPath: string, transcript: Transcript) {
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
      lineButtons[index].classList.add("in-mark");
    }
    lineButtons[range.first_line].classList.add("marked");
    lineButtons[range.first_line].title = `You marked this at ${formatTime(range.mark_ms)}`;
  }

  player.src = convertFileSrc(audioPath);
  playback.hidden = segments.length === 0;
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

async function transcribe(path: string, marks: number[]) {
  statusText.textContent = "Transcribing...";
  const started = Date.now();

  const transcript = await invoke<Transcript>("transcribe", { path, marks });

  const seconds = Math.round((Date.now() - started) / 1000);
  const markNote = marks.length > 0 ? `, ${marks.length} marked` : "";
  statusText.textContent = `Transcribed in ${seconds}s${markNote}. Saved next to ${path}`;
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
      // stop recording, then transcribe it
      const stopped = await invoke<StoppedRecording>("stop_recording");
      clearInterval(timerInterval);
      setRecording(false);
      await transcribe(stopped.path, stopped.marks);
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
      playback.hidden = true;
      setRecording(true);
    }
  } catch (error) {
    statusText.textContent = String(error);
  } finally {
    recordButton.disabled = false;
  }
});
