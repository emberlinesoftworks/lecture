// shrinks long silences before whisper sees the audio, then maps
// whisper's times back to the real recording.
//
// why: whisper spends the same time on a minute of silence as a minute of
// talking, and it likes to "hear" things like "Thank you." in silence.
// a voice detector finds where the talking is, every silence longer than
// 2 seconds gets cut down to 1 second, and nothing else changes

use crate::transcriber::WHISPER_SAMPLE_RATE;

// silences longer than this get shortened
const LONGEST_GAP_MS: usize = 2000;
// how much of a long silence is left in (half after the talking, half before the next),
// enough that whisper still hears a pause and ends the sentence
const KEPT_GAP_MS: usize = 1000;

// one stretch of audio that was kept, and where it came from
#[derive(Debug, Clone, PartialEq)]
pub struct KeptPiece {
    pub squeezed_start: usize,
    pub original_start: usize,
    pub length: usize,
}

// speech is a list of (start, end) sample positions where someone is talking.
// returns the shortened audio and the pieces needed to map times back
pub fn squeeze_silence(audio: &[f32], speech: &[(usize, usize)]) -> (Vec<f32>, Vec<KeptPiece>) {
    let longest_gap = ms_to_samples(LONGEST_GAP_MS);
    let half_kept_gap = ms_to_samples(KEPT_GAP_MS) / 2;

    // work out which parts of the original audio to keep
    let mut keep: Vec<(usize, usize)> = Vec::new();
    let mut previous_end = 0;

    for (i, &(start, end)) in speech.iter().enumerate() {
        let start = start.min(audio.len());
        let end = end.min(audio.len());
        if end <= start {
            continue;
        }

        // the silence before this speech (the very start counts too)
        let gap_start = if i == 0 { 0 } else { previous_end };
        let gap = start.saturating_sub(gap_start);

        if gap > longest_gap {
            // keep a little after the last speech and a little before this one
            if i > 0 {
                keep.push((gap_start, gap_start + half_kept_gap));
            }
            keep.push((start - half_kept_gap, end));
        } else {
            keep.push((gap_start, end));
        }
        previous_end = end;
    }

    // and a little bit of whatever is left at the end
    if !keep.is_empty() {
        let tail_end = (previous_end + half_kept_gap).min(audio.len());
        keep.push((previous_end, tail_end));
    }

    // join pieces that touch so the list stays short
    let mut joined: Vec<(usize, usize)> = Vec::new();
    for (start, end) in keep {
        match joined.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => joined.push((start, end)),
        }
    }

    let mut squeezed = Vec::new();
    let mut pieces = Vec::new();
    for (start, end) in joined {
        if end <= start {
            continue;
        }
        pieces.push(KeptPiece { squeezed_start: squeezed.len(), original_start: start, length: end - start });
        squeezed.extend_from_slice(&audio[start..end]);
    }

    (squeezed, pieces)
}

// whisper gives times in the shortened audio, this turns one back into a time in the recording
pub fn original_ms(pieces: &[KeptPiece], squeezed_ms: i64) -> i64 {
    let position = (squeezed_ms.max(0) as usize) * WHISPER_SAMPLE_RATE as usize / 1000;

    // the last piece that starts at or before this spot
    let piece = pieces.iter().rev().find(|piece| piece.squeezed_start <= position);
    let Some(piece) = piece else { return squeezed_ms };

    // past the end of a piece means it's in the cut, so stick to the end of the piece
    let into_piece = (position - piece.squeezed_start).min(piece.length);
    ((piece.original_start + into_piece) * 1000 / WHISPER_SAMPLE_RATE as usize) as i64
}

fn ms_to_samples(ms: usize) -> usize {
    ms * WHISPER_SAMPLE_RATE as usize / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: usize = WHISPER_SAMPLE_RATE as usize;

    #[test]
    fn short_pauses_are_left_alone() {
        let audio = vec![0.1; 10 * SECOND];
        let speech = [(SECOND, 3 * SECOND), (4 * SECOND, 6 * SECOND)];
        let (squeezed, pieces) = squeeze_silence(&audio, &speech);

        // the 1 second pause in the middle stays, only the end is trimmed
        assert_eq!(pieces.len(), 1);
        assert_eq!(squeezed.len(), 6 * SECOND + SECOND / 2);
        assert_eq!(original_ms(&pieces, 4500), 4500);
    }

    #[test]
    fn long_silence_is_cut_down_and_times_map_back() {
        let audio = vec![0.1; 30 * SECOND];
        // talking 0-2s, then nothing until 20s, then talking until 25s
        let speech = [(0, 2 * SECOND), (20 * SECOND, 25 * SECOND)];
        let (squeezed, pieces) = squeeze_silence(&audio, &speech);

        // 2s + 1s of gap + 5s + half a second at the end
        assert_eq!(squeezed.len(), 8 * SECOND + SECOND / 2);
        // 1s into the speech is still 1s
        assert_eq!(original_ms(&pieces, 1000), 1000);
        // the second stretch of talking starts at 3s in the squeezed audio, which is 20s really
        assert_eq!(original_ms(&pieces, 3000), 20_000);
        assert_eq!(original_ms(&pieces, 5000), 22_000);
    }

    #[test]
    fn silence_at_the_start_is_cut_down() {
        let audio = vec![0.1; 20 * SECOND];
        let speech = [(15 * SECOND, 18 * SECOND)];
        let (squeezed, pieces) = squeeze_silence(&audio, &speech);

        assert_eq!(squeezed.len(), SECOND / 2 + 3 * SECOND + SECOND / 2);
        assert_eq!(original_ms(&pieces, 500), 15_000);
    }

    #[test]
    fn no_speech_means_nothing_to_transcribe() {
        let audio = vec![0.0; 5 * SECOND];
        let (squeezed, pieces) = squeeze_silence(&audio, &[]);
        assert!(squeezed.is_empty());
        assert!(pieces.is_empty());
    }
}
