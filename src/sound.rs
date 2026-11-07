//! A mystical bell for a finished turn, the same for Claude and Codex. Inharmonic partials (as
//! in a real bronze bell), a slow shimmer from a detuned pair, and a soft echo for space.
//! Synthesized once into an in-memory WAV, so nothing ships as an asset.

use std::{f32::consts::TAU, sync::OnceLock};

const RATE: u32 = 22_050;

/// One bell strike: partials as (frequency ratio, level, decay per second). The ratios are not
/// whole numbers, which is what makes a bell sound like a bell and not a flute.
const PARTIALS: [(f32, f32, f32); 4] = [
    (1.0, 1.0, 1.5),
    (2.0, 0.45, 2.2),
    (2.76, 0.32, 2.9),
    (5.4, 0.16, 4.0),
];

fn strike(frequency: f32, level: f32, seconds: f32) -> Vec<f32> {
    let total = (RATE as f32 * seconds) as usize;
    (0..total)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.004).min(1.0);
            let body: f32 = PARTIALS
                .iter()
                .map(|&(ratio, amount, decay)| {
                    amount * (-t * decay).exp() * (TAU * frequency * ratio * t).sin()
                })
                .sum();
            // A detuned copy of the fundamental makes the slow shimmer that a bell has.
            let shimmer = 0.35 * (-t * 1.1).exp() * (TAU * (frequency + 2.6) * t).sin();
            level * attack * (body + shimmer)
        })
        .collect()
}

fn mix_into(out: &mut [f32], part: &[f32], start: f32) {
    let first = ((start * RATE as f32) as usize).min(out.len());
    for (sample, value) in out[first..].iter_mut().zip(part) {
        *sample += value;
    }
}

/// A feedback echo: two delays, each returning a quieter copy of what it heard.
fn echo(dry: &[f32]) -> Vec<f32> {
    let mut wet = dry.to_vec();
    for (delay_seconds, feedback) in [(0.061_f32, 0.42_f32), (0.089, 0.36)] {
        let delay = (delay_seconds * RATE as f32) as usize;
        for i in delay..wet.len() {
            wet[i] += feedback * wet[i - delay];
        }
    }
    dry.iter()
        .zip(&wet)
        .map(|(d, w)| d + 0.5 * (w - d))
        .collect()
}

/// The mystical bell: three strikes rising through a open chord, then the echo settles.
fn sound() -> Vec<f32> {
    let mut out = vec![0.0_f32; (RATE as f32 * 2.6) as usize];
    mix_into(&mut out, &strike(587.33, 0.5, 2.6), 0.0); // D5
    mix_into(&mut out, &strike(880.0, 0.36, 2.4), 0.2); // A5
    mix_into(&mut out, &strike(1174.66, 0.26, 2.2), 0.42); // D6
    let mut out = echo(&out);
    // Fade the very end so the echo never cuts off.
    let fade = (RATE as f32 * 0.4) as usize;
    let len = out.len();
    for (i, sample) in out.iter_mut().enumerate() {
        let remaining = len - i;
        if remaining < fade {
            *sample *= remaining as f32 / fade as f32;
        }
    }
    let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs())).max(1e-6);
    out.iter().map(|s| s / peak * 0.3).collect()
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

/// The bell, built once and kept alive for the asynchronous player.
fn chime() -> &'static [u8] {
    static BELL: OnceLock<Vec<u8>> = OnceLock::new();
    BELL.get_or_init(|| wav(&sound()))
}

pub fn play() {
    let wav = chime();
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
    fn bell_is_a_valid_wav_of_a_few_seconds_at_a_soft_level() {
        let wav = chime();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let length = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(length + 44, wav.len());
        assert!(length / 2 <= RATE as usize * 3); // under three seconds
        let peak = wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!((2000..=i16::MAX as u16).contains(&peak), "peak {peak}");
    }
}
