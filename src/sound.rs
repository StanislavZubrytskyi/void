//! A short melodic chime when an agent finishes a turn: a warm marimba arpeggio for Claude,
//! a bright glassy bell motif for Codex, both with a touch of room reverb. Synthesized once
//! into an in-memory WAV, so nothing ships as an asset.

use std::{f32::consts::TAU, sync::OnceLock};

const RATE: u32 = 22_050;
const LENGTH: f32 = 1.25;

/// One instrument voice: FM synthesis, the modulator at `ratio` times the note, its
/// brightness fading faster than the note itself.
struct Voice {
    ratio: f32,
    brightness: f32,
    decay: f32,
}

/// (start in seconds, frequency, loudness)
type Note = (f32, f32, f32);

fn render(voice: &Voice, notes: &[Note]) -> Vec<f32> {
    let total = (RATE as f32 * LENGTH) as usize;
    let mut dry = vec![0.0_f32; total];
    for &(start, frequency, gain) in notes {
        let first = (start * RATE as f32) as usize;
        for (i, sample) in dry[first.min(total)..].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let envelope = (t / 0.004).min(1.0) * (-t * voice.decay).exp();
            let index = voice.brightness * (-t * 7.0).exp();
            let modulator = (TAU * frequency * voice.ratio * t).sin();
            let body = (TAU * frequency * t + index * modulator).sin();
            // A soft octave below gives the note some weight.
            let low = 0.18 * (TAU * frequency * 0.5 * t).sin() * (-t * 3.0).exp();
            *sample += gain * envelope * (body + low);
        }
    }
    // A small room: four damped feedback combs, mixed in under the dry sound.
    let mut wet = vec![0.0_f32; total];
    for delay in [0.0297_f32, 0.0371, 0.0411, 0.0437] {
        let delay = (delay * RATE as f32) as usize;
        let mut line = vec![0.0_f32; total];
        let mut low = 0.0_f32;
        for i in 0..total {
            let back = if i >= delay { line[i - delay] } else { 0.0 };
            low = 0.6 * low + 0.4 * back;
            line[i] = dry[i] + 0.7 * low;
            wet[i] += line[i] * 0.25;
        }
    }
    let mixed: Vec<f32> = dry.iter().zip(&wet).map(|(d, w)| d + 0.28 * w).collect();
    let peak = mixed.iter().fold(0.0_f32, |m, s| m.max(s.abs())).max(1e-6);
    // Fade the tail so the sound never clicks off.
    let fade = (RATE as f32 * 0.15) as usize;
    mixed
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let tail = ((total - i) as f32 / fade as f32).min(1.0);
            s / peak * 0.55 * tail
        })
        .collect()
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect();
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

/// The chime for agent 0 (Claude) or 1 (Codex); kept alive for the asynchronous player.
fn chime(agent: usize) -> &'static [u8] {
    static CHIMES: [OnceLock<Vec<u8>>; 2] = [OnceLock::new(), OnceLock::new()];
    CHIMES[agent.min(1)].get_or_init(|| {
        if agent == 0 {
            // Warm marimba: C major rising to the octave, the last note held.
            let marimba = Voice {
                ratio: 2.0,
                brightness: 1.6,
                decay: 5.5,
            };
            wav(&render(
                &marimba,
                &[
                    (0.00, 523.25, 0.55),
                    (0.07, 659.25, 0.55),
                    (0.14, 783.99, 0.6),
                    (0.24, 1046.5, 0.8),
                ],
            ))
        } else {
            // Glassy bell: an open fifth and its octave, bright and short.
            let bell = Voice {
                ratio: 3.5,
                brightness: 2.2,
                decay: 4.2,
            };
            wav(&render(
                &bell,
                &[
                    (0.00, 659.25, 0.55),
                    (0.09, 987.77, 0.6),
                    (0.20, 1318.5, 0.75),
                ],
            ))
        }
    })
}

pub fn play(agent: usize) {
    let wav = chime(agent);
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        PlaySoundW(
            wav.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
    // shortcut: no sound outside Windows until the macOS and Linux builds ship.
    #[cfg(not(windows))]
    let _ = wav;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chimes_are_valid_short_wavs_that_differ() {
        let (claude, codex) = (chime(0), chime(1));
        assert_eq!(&claude[..4], b"RIFF");
        assert_eq!(&claude[8..16], b"WAVEfmt ");
        let length = u32::from_le_bytes(claude[40..44].try_into().unwrap()) as usize;
        assert_eq!(length + 44, claude.len());
        assert!(length / 2 < RATE as usize * 3 / 2); // under a second and a half
        assert_ne!(claude, codex);
    }
}
