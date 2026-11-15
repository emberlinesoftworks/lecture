import { invoke } from "@tauri-apps/api/core";

const recordBtn = document.querySelector<HTMLButtonElement>("#record")!;
const label = recordBtn.querySelector<HTMLElement>(".label")!;
const timer = document.querySelector<HTMLElement>("#timer")!;
const status = document.querySelector<HTMLElement>("#status")!;

let startedAt = 0;
let interval: number | undefined;

function formatTime(ms: number) {
  const secs = Math.floor(ms / 1000);
  const h = Math.floor(secs / 3600);
  const m = String(Math.floor(secs / 60) % 60).padStart(2, "0");
  const s = String(secs % 60).padStart(2, "0");
  return h ? `${h}:${m}:${s}` : `${m}:${s}`;
}

function setRecording(on: boolean) {
  recordBtn.classList.toggle("recording", on);
  recordBtn.setAttribute("aria-pressed", String(on));
  label.textContent = on ? "Stop" : "Record";
}

recordBtn.addEventListener("click", async () => {
  const recording = recordBtn.classList.contains("recording");
  recordBtn.disabled = true;

  try {
    if (recording) {
      const path = await invoke<string>("stop_recording");
      clearInterval(interval);
      status.textContent = `Saved to ${path}`;
      setRecording(false);
    } else {
      await invoke("start_recording");
      startedAt = Date.now();
      timer.textContent = "00:00";
      interval = setInterval(() => (timer.textContent = formatTime(Date.now() - startedAt)), 250);
      status.textContent = "Recording";
      setRecording(true);
    }
  } catch (err) {
    status.textContent = String(err);
  } finally {
    recordBtn.disabled = false;
  }
});
