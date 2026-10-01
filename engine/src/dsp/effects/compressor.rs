use crate::dsp::util::*;
use crate::dsp::*;

pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "compressor",
    name: "Compressor",
    category: Category::Dynamics,
    description: "Iguala a dinâmica e aumenta o sustain. Ótimo para clean e funk.",
    color: "#0ea5e9",
    params: &[
        ParamSpec::knob("threshold", "Threshold", -60.0, 0.0, -24.0, "dB"),
        ParamSpec::knob("ratio", "Ratio", 1.0, 20.0, 4.0, ":1"),
        ParamSpec::log_knob("attack", "Attack", 0.1, 100.0, 8.0, "ms"),
        ParamSpec::log_knob("release", "Release", 10.0, 1000.0, 150.0, "ms"),
        ParamSpec::knob("makeup", "Makeup", 0.0, 24.0, 8.0, "dB"),
        ParamSpec::knob("blend", "Blend", 0.0, 100.0, 100.0, "%"),
    ],
    create: |sr| Dual::boxed(|| Compressor::new(sr)),
    file: None,
    tempo: None,
};

struct Compressor {
    sr: f32,
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: Smoother,
    blend: Smoother,
    env_db: f32,
}

const KNEE_DB: f32 = 6.0;

impl Compressor {
    fn new(sr: f32) -> Self {
        Self {
            sr,
            threshold_db: -24.0,
            ratio: 4.0,
            attack: 0.0,
            release: 0.0,
            makeup: Smoother::new(1.0, 20.0, sr),
            blend: Smoother::new(1.0, 20.0, sr),
            env_db: 0.0,
        }
    }

    /// Curva de ganho estático com soft knee (em dB de redução, <= 0).
    #[inline]
    fn gain_reduction(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        let slope = 1.0 / self.ratio - 1.0;
        if over <= -KNEE_DB / 2.0 {
            0.0
        } else if over >= KNEE_DB / 2.0 {
            slope * over
        } else {
            let k = over + KNEE_DB / 2.0;
            slope * k * k / (2.0 * KNEE_DB)
        }
    }
}

impl MonoEffect for Compressor {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.threshold_db = value,
            1 => self.ratio = value.max(1.0),
            2 => self.attack = time_coef(value, self.sr),
            3 => self.release = time_coef(value, self.sr),
            4 => self.makeup.set(db_to_lin(value)),
            5 => self.blend.set(value / 100.0),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let target = self.gain_reduction(lin_to_db(x.abs()));
            // Detector em dB no domínio da redução (ataque quando reduz mais).
            let coef = if target < self.env_db { self.attack } else { self.release };
            self.env_db = target + (self.env_db - target) * coef;
            let wet = *x * db_to_lin(self.env_db) * self.makeup.next();
            let b = self.blend.next();
            *x = wet * b + *x * (1.0 - b);
        }
    }

    fn reset(&mut self) {
        self.env_db = 0.0;
        self.makeup.snap();
        self.blend.snap();
    }
}
