use crate::dsp::util::*;
use crate::dsp::*;

pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "gate",
    name: "Noise Gate",
    category: Category::Dynamics,
    description: "Corta o chiado entre as notas. Essencial com ganho alto.",
    color: "#64748b",
    params: &[
        ParamSpec::knob("threshold", "Threshold", -90.0, -10.0, -58.0, "dB"),
        ParamSpec::log_knob("hold", "Hold", 1.0, 300.0, 30.0, "ms"),
        ParamSpec::log_knob("release", "Release", 5.0, 800.0, 90.0, "ms"),
    ],
    create: |sr| Dual::boxed(|| Gate::new(sr)),
};

struct Gate {
    sr: f32,
    threshold: f32,
    hold_samples: u32,
    release_coef: f32,
    attack_coef: f32,
    env: f32,
    env_release: f32,
    gain: f32,
    hold_counter: u32,
    open: bool,
}

impl Gate {
    fn new(sr: f32) -> Self {
        Self {
            sr,
            threshold: db_to_lin(-58.0),
            hold_samples: 0,
            release_coef: 0.0,
            attack_coef: time_coef(0.5, sr),
            env: 0.0,
            env_release: time_coef(20.0, sr),
            gain: 0.0,
            hold_counter: 0,
            open: false,
        }
    }
}

impl MonoEffect for Gate {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.threshold = db_to_lin(value),
            1 => self.hold_samples = (value * 0.001 * self.sr) as u32,
            2 => self.release_coef = time_coef(value, self.sr),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        // Histerese de 6 dB: abre no threshold, fecha na metade dele.
        let close = self.threshold * 0.5;
        for x in buf.iter_mut() {
            let a = x.abs();
            self.env = if a > self.env { a } else { self.env * self.env_release };
            if self.env > self.threshold {
                self.open = true;
                self.hold_counter = self.hold_samples;
            } else if self.env < close {
                if self.hold_counter > 0 {
                    self.hold_counter -= 1;
                } else {
                    self.open = false;
                }
            }
            let (target, coef) = if self.open { (1.0, self.attack_coef) } else { (0.0, self.release_coef) };
            self.gain = target + (self.gain - target) * coef;
            *x *= self.gain;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 0.0;
        self.open = false;
    }
}
