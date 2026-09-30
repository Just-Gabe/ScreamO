//! Afinador cromático usando o algoritmo YIN (de Cheveigné & Kawahara).
//! Roda numa thread própria para nunca pesar na thread de áudio.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rtrb::Consumer;
use serde::Serialize;

use super::processor::Meters;

const WINDOW: usize = 2048;
const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunerReading {
    pub active: bool,
    pub frequency: f32,
    pub note: String,
    pub octave: i32,
    pub cents: f32,
}

pub struct Tuner {
    pub reading: Arc<Mutex<TunerReading>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Tuner {
    pub fn start(mut rx: Consumer<f32>, sample_rate: f32, meters: Arc<Meters>) -> Self {
        let reading = Arc::new(Mutex::new(TunerReading::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (reading, stop) = (reading.clone(), stop.clone());
            std::thread::Builder::new()
                .name("screamo-tuner".into())
                .spawn(move || {
                    let mut history = vec![0f32; WINDOW * 2];
                    let mut diff = vec![0f32; WINDOW];
                    let mut smoothed = 0f32;
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(40));
                        if !meters.tuner_enabled.load(Ordering::Relaxed) {
                            // Descarta o que chegou e espera.
                            let n = rx.slots();
                            if let Ok(c) = rx.read_chunk(n) {
                                c.commit_all();
                            }
                            continue;
                        }
                        let n = rx.slots().min(history.len());
                        if n == 0 {
                            continue;
                        }
                        history.rotate_left(n);
                        if let Ok(chunk) = rx.read_chunk(n) {
                            let (a, b) = chunk.as_slices();
                            let start = history.len() - n;
                            history[start..start + a.len()].copy_from_slice(a);
                            history[start + a.len()..].copy_from_slice(b);
                            chunk.commit_all();
                        }
                        let result = detect(&history, sample_rate, &mut diff);
                        let mut r = reading.lock();
                        match result {
                            Some(f) => {
                                // Suaviza leitura para o ponteiro não tremer.
                                smoothed = if smoothed > 0.0 && (f / smoothed - 1.0).abs() < 0.03 {
                                    smoothed * 0.6 + f * 0.4
                                } else {
                                    f
                                };
                                *r = describe(smoothed);
                            }
                            None => r.active = false,
                        }
                    }
                })
                .expect("tuner thread")
        };
        Self { reading, stop, thread: Some(thread) }
    }
}

impl Drop for Tuner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn describe(freq: f32) -> TunerReading {
    let midi = 69.0 + 12.0 * (freq / 440.0).log2();
    let nearest = midi.round();
    let idx = (nearest as i32).rem_euclid(12) as usize;
    TunerReading {
        active: true,
        frequency: freq,
        note: NOTES[idx].to_string(),
        octave: nearest as i32 / 12 - 1,
        cents: (midi - nearest) * 100.0,
    }
}

/// YIN: retorna a frequência fundamental ou `None` se o sinal for fraco ou
/// sem periodicidade clara.
fn detect(buf: &[f32], sr: f32, diff: &mut [f32]) -> Option<f32> {
    let frame = &buf[buf.len() - WINDOW * 2..];
    let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
    if rms < 0.002 {
        return None;
    }
    // Faixa: ~30 Hz (baixo 5 cordas/drop) até ~1400 Hz.
    let tau_min = (sr / 1400.0) as usize;
    let tau_max = ((sr / 30.0) as usize).min(WINDOW - 1);

    diff[0] = 1.0;
    let mut running = 0.0;
    for tau in 1..=tau_max {
        let mut d = 0.0;
        for i in 0..WINDOW {
            let delta = frame[i] - frame[i + tau];
            d += delta * delta;
        }
        running += d;
        // Diferença normalizada cumulativa.
        diff[tau] = if running > 0.0 { d * tau as f32 / running } else { 1.0 };
    }

    const THRESHOLD: f32 = 0.12;
    let mut tau = tau_min;
    while tau < tau_max {
        if diff[tau] < THRESHOLD {
            while tau + 1 < tau_max && diff[tau + 1] < diff[tau] {
                tau += 1;
            }
            // Interpolação parabólica para precisão sub-amostra.
            let (a, b, c) = (diff[tau - 1], diff[tau], diff[tau + 1]);
            let denom = a - 2.0 * b + c;
            let shift = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
            return Some(sr / (tau as f32 + shift));
        }
        tau += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_low_e_string() {
        let sr = 48000.0;
        let f = 82.41;
        let buf: Vec<f32> = (0..WINDOW * 2)
            .map(|i| {
                let t = i as f32 / sr;
                let w = 2.0 * std::f32::consts::PI * f * t;
                0.5 * w.sin() + 0.3 * (2.0 * w).sin() + 0.1 * (3.0 * w).sin()
            })
            .collect();
        let mut diff = vec![0.0; WINDOW];
        let got = detect(&buf, sr, &mut diff).unwrap();
        assert!((got - f).abs() < 0.5, "got {got}");
        let r = describe(got);
        assert_eq!(r.note, "E");
        assert_eq!(r.octave, 2);
    }
}
