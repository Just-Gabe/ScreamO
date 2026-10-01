use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use crate::dsp::util::*;
use crate::dsp::*;

const MAX_SECONDS: f32 = 60.0;
/// Repetições antigas perdem um pouco a cada overdub (evita estourar).
const OVERDUB_DECAY: f32 = 0.97;

/// Estado exposto para a UI (um looper por vez na cadeia).
pub static STATE: AtomicU8 = AtomicU8::new(0);
pub static POSITION: AtomicU32 = AtomicU32::new(0);
pub static LENGTH: AtomicU32 = AtomicU32::new(0);
pub static SAMPLE_RATE: AtomicU32 = AtomicU32::new(48000);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum LoopState {
    Empty = 0,
    Recording = 1,
    Playing = 2,
    Overdubbing = 3,
    Stopped = 4,
}

impl LoopState {
    pub fn label(v: u8) -> &'static str {
        match v {
            1 => "recording",
            2 => "playing",
            3 => "overdubbing",
            4 => "stopped",
            _ => "empty",
        }
    }
}

pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "looper",
    name: "Looper",
    category: Category::Time,
    description: "Grave uma base e toque por cima (até 60 s). Coloque no fim da cadeia.",
    color: "#f43f5e",
    params: &[
        ParamSpec::actions("transport", "Transport", &["Rec/Dub", "Play", "Stop", "Clear"]),
        ParamSpec::knob("level", "Loop Level", 0.0, 100.0, 100.0, "%"),
    ],
    create: |sr| Box::new(Looper::new(sr)),
    file: None,
    tempo: None,
};

const ACT_REC: u8 = 1;
const ACT_PLAY: u8 = 2;
const ACT_STOP: u8 = 3;
const ACT_CLEAR: u8 = 4;

struct Looper {
    l: Vec<f32>,
    r: Vec<f32>,
    len: usize,
    pos: usize,
    state: LoopState,
    level: Smoother,
}

impl Looper {
    fn new(sr: f32) -> Self {
        let max = (MAX_SECONDS * sr) as usize;
        SAMPLE_RATE.store(sr as u32, Ordering::Relaxed);
        let looper = Self {
            l: vec![0.0; max],
            r: vec![0.0; max],
            len: 0,
            pos: 0,
            state: LoopState::Empty,
            level: Smoother::new(1.0, 20.0, sr),
        };
        looper.publish();
        looper
    }

    fn publish(&self) {
        STATE.store(self.state as u8, Ordering::Relaxed);
        POSITION.store(self.pos as u32, Ordering::Relaxed);
        LENGTH.store(self.len as u32, Ordering::Relaxed);
    }

    fn trigger(&mut self, action: u8) {
        use LoopState::*;
        self.state = match (self.state, action) {
            (_, ACT_CLEAR) => {
                self.len = 0;
                self.pos = 0;
                Empty
            }
            (Empty, ACT_REC) => {
                self.len = 0;
                self.pos = 0;
                Recording
            }
            // Fechar o loop: o que foi gravado vira a base.
            (Recording, ACT_REC | ACT_PLAY) if self.len > 0 => {
                self.pos = 0;
                Playing
            }
            (Recording, ACT_STOP) if self.len > 0 => {
                self.pos = 0;
                Stopped
            }
            (Recording, ACT_STOP) => Empty,
            (Playing, ACT_REC) => Overdubbing,
            (Overdubbing, ACT_REC | ACT_PLAY) => Playing,
            (Playing | Overdubbing, ACT_STOP) => {
                self.pos = 0;
                Stopped
            }
            (Stopped, ACT_PLAY) => Playing,
            (Stopped, ACT_REC) => Overdubbing,
            (s, _) => s,
        };
        self.publish();
    }
}

impl Effect for Looper {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => {
                let action = value.round() as u8;
                if action > 0 {
                    self.trigger(action);
                }
            }
            1 => self.level.set(value / 100.0),
            _ => {}
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], stereo: bool) -> bool {
        use LoopState::*;
        let max = self.l.len();
        for i in 0..l.len() {
            let (inl, inr) = (l[i], r[i]);
            let level = self.level.next();
            match self.state {
                Recording => {
                    self.l[self.len] = inl;
                    self.r[self.len] = inr;
                    self.len += 1;
                    if self.len >= max {
                        self.pos = 0;
                        self.state = Playing;
                    }
                }
                Playing | Overdubbing if self.len > 0 => {
                    let p = self.pos;
                    l[i] = inl + self.l[p] * level;
                    r[i] = inr + self.r[p] * level;
                    if self.state == Overdubbing {
                        self.l[p] = self.l[p] * OVERDUB_DECAY + inl;
                        self.r[p] = self.r[p] * OVERDUB_DECAY + inr;
                    }
                    self.pos = if p + 1 >= self.len { 0 } else { p + 1 };
                }
                _ => {}
            }
        }
        self.publish();
        stereo || matches!(self.state, Playing | Overdubbing)
    }

    fn reset(&mut self) {
        self.level.snap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_play_back() {
        let mut lp = Looper::new(1000.0);
        lp.set_param(1, 100.0);
        lp.reset();
        lp.set_param(0, ACT_REC as f32);
        let mut l: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let mut r = l.clone();
        lp.process(&mut l, &mut r, false);
        lp.set_param(0, ACT_PLAY as f32);
        assert_eq!(lp.state, LoopState::Playing);
        assert_eq!(lp.len, 100);
        // Com entrada em silêncio, a saída é o loop gravado.
        let mut l2 = vec![0.0; 100];
        let mut r2 = vec![0.0; 100];
        lp.process(&mut l2, &mut r2, false);
        assert!((l2[50] - 0.5).abs() < 1e-3, "{}", l2[50]);
        lp.set_param(0, ACT_CLEAR as f32);
        assert_eq!(lp.state, LoopState::Empty);
    }
}
