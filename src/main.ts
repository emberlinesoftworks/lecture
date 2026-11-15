import { invoke } from "@tauri-apps/api/core";

type Segment = {
  start_ms: number;
  end_ms: number;
  text: string;
};

const recordButton = document.querySelector<HTMLButtonElement>("#record")!;
const buttonLabel = recordButton.querySelector<HTMLElement>(".label")!;
const timer = document.querySelector<HTMLElement>("#timer")!;
const statusText = document.querySelector<HTMLElement>("#status")!;
const transcriptList = document.querySelector<HTMLOListElement>("#transcript")!;

let startTime = 0;
let timerInterval: number | undefined;

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

function showTranscript(segments: Segment[]) {
  transcriptList.replaceChildren();

  for (const segment of segments) {
    const line = document.createElement("li");
    const time = document.createElement("time");
    const text = document.createElement("span");

    time.textContent = formatTime(segment.start_ms);
    text.textContent = segment.text;

    line.append(time, text);
    transcriptList.append(line);
  }
  transcriptList.hidden = segments.length === 0;
}

async function transcribe(path: string) {
  statusText.textContent = "Transcribing...";
  const started = Date.now();

  const segments = await invoke<Segment[]>("transcribe", { path });

  const seconds = Math.round((Date.now() - started) / 1000);
  statusText.textContent = `Transcribed in ${seconds}s. Saved next to ${path}`;
  showTranscript(segments);
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
      await invoke("start_recording");
      startTime = Date.now();
      timer.textContent = "00:00";
      timerInterval = setInterval(() => {
        timer.textContent = formatTime(Date.now() - startTime);
      }, 250);
      statusText.textContent = "Recording";
      transcriptList.hidden = true;
      setRecording(true);
    }
  } catch (error) {
    statusText.textContent = String(error);
  } finally {
    recordButton.disabled = false;
  }
});
