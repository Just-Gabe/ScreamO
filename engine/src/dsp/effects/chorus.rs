use std::f32::consts::TAU;

use crate::dsp::util::*;
use crate::dsp::*;

/// Chorus estéreo: LFOs defasados em 90° entre L e R abrem o som no fone.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "chorus",
    name: "Chorus",
    category: Category::Modulation,
    description: "Chorus estéreo, do shimmer anos 80 ao vibrato.",
    color: "#3b82f6",
    params: &[
        ParamSpec::log_knob("rate", "Rate", 0.05, 8.0, 0.8, "Hz"),
        ParamSpec::knob("depth", "Depth", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("mix", "Mix", 0.0, 100.0, 50.0, "%"),
        ParamSpec::knob("width", "Width", 0.0, 100.0, 100.0, "%"),
    ],
    create: |sr| Box::new(Chorus::new(sr)),
    file: None,
    tempo: None,
};

struct Chorus {
    sr: f32,
    dl: DelayLine,
    dr: DelayLine,
    phase: f32,
    rate: f32,
    depth: Smoother,
    mix: Smoother,
    width: f32,
}

impl Chorus {
    fn new(sr: f32) -> Self {
        let max = (0.03 * sr) as usize;
        Self {
            sr,
            dl: DelayLine::new(max),
            dr: DelayLine::new(max),
            phase: 0.0,
            rate: 0.8,
            depth: Smoother::new(0.5, 50.0, sr),
            mix: Smoother::new(0.5, 30.0, sr),
            width: 1.0,
        }
    }
}

impl Effect for Chorus {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.rate = value,
            1 => self.depth.set(value / 10.0),
            2 => self.mix.set(value / 100.0),
            3 => self.width = value / 100.0,
            _ => {}
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], _stereo: bool) -> bool {
        let base = 0.007 * self.sr;
        let inc = self.rate / self.sr;
        let offset = 0.25 * self.width;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let depth = self.depth.next() * 0.0035 * self.sr;
            let mix = self.mix.next();
            self.phase = (self.phase + inc).fract();
            let ml = (self.phase * TAU).sin();
            let mr = ((self.phase + offset) * TAU).sin();
            self.dl.push(*xl);
            self.dr.push(*xr);
            let wl = self.dl.read(base + depth * ml);
            let wr = self.dr.read(base + depth * mr);
            *xl = *xl * (1.0 - 0.5 * mix) + wl * mix;
            *xr = *xr * (1.0 - 0.5 * mix) + wr * mix;
        }
        true
    }

    fn reset(&mut self) {
        self.dl.reset();
        self.dr.reset();
        self.depth.snap();
        self.mix.snap();
    }
}
