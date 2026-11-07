//! A cat's meow when an agent finishes a turn: a voiced source whose pitch glides up and down,
//! shaped by two resonances so it sounds like a vowel. Claude's is a long, drawn-out "мяяу";
//! Codex's is a short, brighter "мяу!". Synthesized once into an in-memory WAV, so nothing ships
//! as an asset.

use std::{f32::consts::TAU, sync::OnceLock};

const RATE: u32 = 22_050;

/// One meow: a pitch contour through `pitch` (Hz at the start, the peak, and the end), a total
/// length, and the two vowel resonances.
struct Meow {
    seconds: f32,
    pitch: [f32; 3],
    formants: [f32; 2],
    /// Gain of the second formant against the first.
    brightness: f32,
}

/// RBJ band-pass biquad, direct form I.
struct Band {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Band {
    fn new(frequency: f32, q: f32) -> Self {
        let w0 = TAU * frequency / RATE as f32;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b: [alpha / a0, 0.0, -alpha / a0],
            a: [-2.0 * w0.cos() / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn step(&mut self, x: f32) -> f32 {
        let y =
            self.b[0] * x + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

fn render(meow: &Meow) -> Vec<f32> {
    let total = (RATE as f32 * meow.seconds) as usize;
    let mut first = Band::new(meow.formants[0], 7.0);
    let mut second = Band::new(meow.formants[1], 9.0);
    let mut phase = 0.0_f32;
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let t = i as f32 / total as f32;
        // Pitch glides through the three points: up to the peak, then down to the end.
        let contour = if t < 0.4 {
            meow.pitch[0] + (meow.pitch[1] - meow.pitch[0]) * ease(t / 0.4)
        } else {
            meow.pitch[1] + (meow.pitch[2] - meow.pitch[1]) * ease((t - 0.4) / 0.6)
        };
        // A slow, shallow vibrato, the way a real voice wobbles.
        let vibrato = 1.0 + 0.012 * (TAU * 5.5 * i as f32 / RATE as f32).sin();
        phase += TAU * contour * vibrato / RATE as f32;
        if phase > TAU {
            phase -= TAU;
        }
        // A glottal-like source: harmonics falling off with frequency, which sounds like a voice.
        let mut source = 0.0_f32;
        for n in 1..=24 {
            let harmonic = n as f32;
            if harmonic * contour * vibrato > 6000.0 {
                break;
            }
            source += (harmonic * phase).sin() / harmonic;
        }
        // Resonances move with the mouth: the vowel opens as the pitch peaks.
        let open = 1.0 + 0.15 * (1.0 - (t - 0.4).abs() * 2.0).max(0.0);
        let a = first.step(source * open);
        let b = second.step(source) * meow.brightness;
        let t_sec = i as f32 / RATE as f32;
        let attack = (t_sec / 0.06).min(1.0);
        let release = ((meow.seconds - t_sec) / 0.14).clamp(0.0, 1.0);
        out.push((a + b) * attack * release);
    }
    let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs())).max(1e-6);
    out.iter().map(|s| s / peak * 0.6).collect()
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
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

/// The meow for agent 0 (Claude) or 1 (Codex); kept alive for the asynchronous player.
fn chime(agent: usize) -> &'static [u8] {
    static MEOWS: [OnceLock<Vec<u8>>; 2] = [OnceLock::new(), OnceLock::new()];
    MEOWS[agent.min(1)].get_or_init(|| {
        let meow = if agent == 0 {
            // A long, sleepy "мяяяу": low start, a rising question, a soft fall.
            Meow {
                seconds: 1.05,
                pitch: [520.0, 780.0, 430.0],
                formants: [820.0, 1900.0],
                brightness: 0.45,
            }
        } else {
            // A short, bright "мяу!": quick and high, like a happy hello.
            Meow {
                seconds: 0.52,
                pitch: [760.0, 1010.0, 640.0],
                formants: [1000.0, 2400.0],
                brightness: 0.6,
            }
        };
        wav(&render(&meow))
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
    fn meows_are_valid_short_wavs_that_differ() {
        let (claude, codex) = (chime(0), chime(1));
        for wav in [claude, codex] {
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            let length = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
            assert_eq!(length + 44, wav.len());
            assert!(length / 2 <= RATE as usize * 3 / 2); // under a second and a half
            // The meow is audible: not silence, and not clipped to a square wave.
            let samples: Vec<i16> = wav[44..]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            assert!((4000..=i16::MAX as u16).contains(&peak), "peak {peak}");
        }
        assert_ne!(claude, codex);
    }
}
