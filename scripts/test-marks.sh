#!/bin/sh
# makes a test recording of the mitochondria script with real pauses using the
# Mac's built-in voice, then checks that the marks land where they should
set -e

project="$(cd "$(dirname "$0")/.." && pwd)"
audio="$HOME/Documents/Lecture/test-marks.wav"
model="$HOME/Library/Application Support/com.emberlinesoftworks.lecture/models/ggml-large-v3-turbo.bin"

echo "Making the test recording..."
mkdir -p "$HOME/Documents/Lecture"
say -o /tmp/lecture-test.aiff "Today we're going to talk about the mitochondria. \
The mitochondria is found in almost every cell in your body. [[slnc 1500]] \
Here's the important part. The mitochondria makes ATP through a process called cellular respiration. \
Cellular respiration uses glucose and oxygen, and it produces carbon dioxide and water as waste. [[slnc 1500]] \
Now let's switch topics and talk about the cell membrane. \
The cell membrane is made of a phospholipid bilayer. [[slnc 1500]] \
Each phospholipid has a hydrophilic head and two hydrophobic tails. \
The heads face the water and the tails face each other. [[slnc 1500]] \
That's all for today. Make sure you review these before the quiz."
afconvert -f WAVE -d LEI16@48000 -c 1 /tmp/lecture-test.aiff "$audio"

echo "Building..."
cd "$project/src-tauri"
cargo build --quiet --example check_marks

echo "Transcribing and checking marks..."
./target/debug/examples/check_marks "$model" "$audio" 2>/dev/null
