use std::f32::consts::TAU;

use crate::dsp::util::*;
use crate::dsp::*;

/// Tremolo: o exemplo mais simples de efeito — bom ponto de partida para
/// quem quer criar um módulo novo.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "tremolo",
    name: "Tremolo",
    category: Category::Modulation,
    description: "Variação de volume pulsante, de suave (senoidal) a picotado (quadrado).",
    color: "#ec4899",
    params: &[
        ParamSpec::log_knob("rate", "Rate", 0.5, 15.0, 5.0, "Hz"),
        ParamSpec::knob("depth", "Depth", 0.0, 100.0, 60.0, "%"),
        ParamSpec::choice("shape", "Shape", &["Sine", "Square"], 0),
    ],
    create: |sr| Dual::boxed(|| Tremolo::new(sr)),
};

struct Tremolo {
    sr: f32,
    phase: f32,
    rate: f32,
    depth: Smoother,
    square: bool,
    // Suaviza a onda quadrada para não gerar cliques.
    shape_lp: OnePole,
}

impl Tremolo {
    fn new(sr: f32) -> Self {
        Self {
            sr,
            phase: 0.0,
            rate: 5.0,
            depth: Smoother::new(0.6, 30.0, sr),
            square: false,
            shape_lp: OnePole::lowpass(sr, 120.0),
        }
    }
}

impl MonoEffect for Tremolo {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.rate = value,
            1 => self.depth.set(value / 100.0),
            2 => self.square = value >= 0.5,
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let inc = self.rate / self.sr;
        for x in buf.iter_mut() {
            self.phase = (self.phase + inc).fract();
            let s = (self.phase * TAU).sin();
            let lfo = if self.square { self.shape_lp.process(s.signum()) } else { s };
            let depth = self.depth.next();
            *x *= 1.0 - depth * (0.5 - 0.5 * lfo);
        }
    }

    fn reset(&mut self) {
        self.shape_lp.reset();
        self.depth.snap();
    }
}
