import { convertFileSrc, invoke } from "@tauri-apps/api/core";

type Segment = {
  start_ms: number;
  end_ms: number;
  text: string;
};

const recordButton = document.querySelector<HTMLButtonElement>("#record")!;
const buttonLabel = recordButton.querySelector<HTMLElement>(".label")!;
const timer = document.querySelector<HTMLElement>("#timer")!;
const statusText = document.querySelector<HTMLElement>("#status")!;
const playback = document.querySelector<HTMLElement>("#playback")!;
const player = document.querySelector<HTMLAudioElement>("#player")!;
const transcriptList = document.querySelector<HTMLOListElement>("#transcript")!;

let startTime = 0;
let timerInterval: number | undefined;
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
}

// each line is a button, clicking it jumps the audio to when that was said
function showTranscript(audioPath: string, newSegments: Segment[]) {
  segments = newSegments;
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

async function transcribe(path: string) {
  statusText.textContent = "Transcribing...";
  const started = Date.now();

  const newSegments = await invoke<Segment[]>("transcribe", { path });

  const seconds = Math.round((Date.now() - started) / 1000);
  statusText.textContent = `Transcribed in ${seconds}s. Saved next to ${path}`;
  showTranscript(path, newSegments);
}

recordButton.addEventListener("click", async () => {
  const isRecording = recordButton.classList.contains("recording");
  // disable the button so it can't be double clicked while waiting on rust
  recordButton.disabled = true;

  try {
    if (isRecording) {
      // stop recording, then transcribe it
      const savedPath = await invoke<string>("stop_recording");
      clearInterval(timerInterval);
      setRecording(false);
      await transcribe(savedPath);
    } else {
      // start recording and the timer
      player.pause();
      await invoke("start_recording");
      startTime = Date.now();
      timer.textContent = "00:00";
      timerInterval = setInterval(() => {
        timer.textContent = formatTime(Date.now() - startTime);
      }, 250);
      statusText.textContent = "Recording";
      playback.hidden = true;
      setRecording(true);
    }
  } catch (error) {
    statusText.textContent = String(error);
  } finally {
    recordButton.disabled = false;
  }
});
