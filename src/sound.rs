//! Cat sounds for a finished turn: a few soft meows, then a small bell that settles the moment.
//! Claude hears the calmer set, Codex the brighter kitten. Synthesized once into in-memory WAVs,
//! so nothing ships as an asset.

use std::{f32::consts::TAU, sync::OnceLock};

const RATE: u32 = 22_050;

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

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// One meow: a voice whose pitch rises and then falls like "мяу", shaped by two vowel
/// resonances, with a breath of noise and a gentle low-pass so nothing sits in the harsh top end.
fn meow(seconds: f32, pitch: [f32; 3], formants: [f32; 2], gain: f32, seed: &mut u32) -> Vec<f32> {
    let total = (RATE as f32 * seconds) as usize;
    let mut first = Band::new(formants[0], 4.0);
    let mut second = Band::new(formants[1], 6.0);
    let mut low = 0.0_f32;
    let smoothing = 1.0 - (-TAU * 3000.0 / RATE as f32).exp();
    let mut phase = 0.0_f32;
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let t = i as f32 / total as f32;
        let sec = i as f32 / RATE as f32;
        let f = if t < 0.35 {
            pitch[0] + (pitch[1] - pitch[0]) * ease(t / 0.35)
        } else {
            pitch[1] + (pitch[2] - pitch[1]) * ease((t - 0.35) / 0.65)
        } * (1.0 + 0.01 * (TAU * 6.0 * sec).sin());
        phase += TAU * f / RATE as f32;
        let mut source = 0.0_f32;
        for n in 1..=7 {
            source += (n as f32 * phase).sin() / (n as f32).powf(1.3);
        }
        // A small linear congruential generator: the breath is the same on every start.
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = ((*seed >> 8) as f32 / (1_u32 << 24) as f32) * 2.0 - 1.0;
        let voiced = first.step(source) * 0.7 + second.step(source) * 0.35 + noise * 0.03;
        low += smoothing * (voiced - low);
        let envelope = (sec / 0.05).min(1.0) * ((seconds - sec) / 0.12).clamp(0.0, 1.0);
        out.push(low * envelope);
    }
    normalize(&mut out, gain);
    out
}

/// A soft, round bell: a sine with a quick attack and a long, warm tail. Each note is
/// (start in seconds, frequency, level).
fn bell(seconds: f32, notes: &[(f32, f32, f32)], gain: f32) -> Vec<f32> {
    let total = (RATE as f32 * seconds) as usize;
    let mut out = vec![0.0_f32; total];
    for &(start, frequency, level) in notes {
        let first = (start * RATE as f32) as usize;
        for (i, sample) in out[first.min(total)..].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let envelope = (t / 0.01).min(1.0) * (-t * 3.2).exp();
            *sample += level
                * envelope
                * ((TAU * frequency * t).sin() + 0.12 * (TAU * 2.0 * frequency * t).sin());
        }
    }
    normalize(&mut out, gain);
    out
}

fn normalize(samples: &mut [f32], gain: f32) {
    let peak = samples
        .iter()
        .fold(0.0_f32, |m, s| m.max(s.abs()))
        .max(1e-6);
    for sample in samples.iter_mut() {
        *sample = *sample / peak * gain;
    }
}

/// Parts played one after another, each followed by its own silence.
fn sequence(parts: &[(Vec<f32>, f32)]) -> Vec<f32> {
    let mut out = Vec::new();
    for (part, gap) in parts {
        out.extend_from_slice(part);
        out.extend(std::iter::repeat_n(0.0, (RATE as f32 * gap) as usize));
    }
    out
}

/// `b` starts `offset` seconds into `a`; the result is long enough for both.
fn mix(a: &[f32], b: &[f32], offset: f32) -> Vec<f32> {
    let shift = (RATE as f32 * offset) as usize;
    let len = a.len().max(b.len() + shift);
    (0..len)
        .map(|i| {
            let first = a.get(i).copied().unwrap_or(0.0);
            let second = if i >= shift {
                b.get(i - shift).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            first + second
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

/// Claude's: three soft meows, the last one longer, and a bell that settles the moment.
fn calm() -> Vec<f32> {
    let mut seed = 7_u32;
    let meows = sequence(&[
        (
            meow(0.3, [440.0, 640.0, 380.0], [820.0, 1900.0], 0.16, &mut seed),
            0.07,
        ),
        (
            meow(0.3, [460.0, 680.0, 400.0], [820.0, 1900.0], 0.16, &mut seed),
            0.07,
        ),
        (
            meow(
                0.55,
                [470.0, 720.0, 360.0],
                [780.0, 1800.0],
                0.18,
                &mut seed,
            ),
            0.0,
        ),
    ]);
    let landing = bell(0.9, &[(0.0, 1046.5, 0.45), (0.14, 1318.5, 0.35)], 0.16);
    mix(&meows, &landing, 0.62)
}

/// Codex's: the same idea, a little higher and quicker, like a kitten.
fn kitten() -> Vec<f32> {
    let mut seed = 7_u32;
    let meows = sequence(&[
        (
            meow(
                0.22,
                [620.0, 860.0, 520.0],
                [950.0, 2200.0],
                0.15,
                &mut seed,
            ),
            0.05,
        ),
        (
            meow(
                0.22,
                [640.0, 880.0, 540.0],
                [950.0, 2200.0],
                0.15,
                &mut seed,
            ),
            0.05,
        ),
        (
            meow(0.4, [660.0, 920.0, 480.0], [900.0, 2100.0], 0.17, &mut seed),
            0.0,
        ),
    ]);
    let landing = bell(0.9, &[(0.0, 1046.5, 0.45), (0.14, 1318.5, 0.35)], 0.16);
    mix(&meows, &landing, 0.45)
}

/// The sound for agent 0 (Claude) or 1 (Codex); kept alive for the asynchronous player.
fn chime(agent: usize) -> &'static [u8] {
    static SOUNDS: [OnceLock<Vec<u8>>; 2] = [OnceLock::new(), OnceLock::new()];
    SOUNDS[agent.min(1)].get_or_init(|| wav(&if agent == 0 { calm() } else { kitten() }))
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
    fn sounds_are_valid_short_wavs_that_differ() {
        let (claude, codex) = (chime(0), chime(1));
        for wav in [claude, codex] {
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            let length = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
            assert_eq!(length + 44, wav.len());
            assert!(length / 2 <= RATE as usize * 2); // under two seconds
            // Audible, and not clipped: a soft level.
            let samples: Vec<i16> = wav[44..]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            assert!((2000..=i16::MAX as u16).contains(&peak), "peak {peak}");
        }
        assert_ne!(claude, codex);
    }
}
