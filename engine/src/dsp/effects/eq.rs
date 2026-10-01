use crate::dsp::util::*;
use crate::dsp::*;

const BANDS: [f32; 7] = [100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0];

/// EQ gráfico de 7 bandas no estilo GE-7.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "eq",
    name: "Graphic EQ",
    category: Category::Eq,
    description: "EQ gráfico de 7 bandas. Depois do amp esculpe o timbre; antes, empurra o drive.",
    color: "#14b8a6",
    params: &[
        ParamSpec::knob("b100", "100", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("b200", "200", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("b400", "400", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("b800", "800", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("b1k6", "1.6k", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("b3k2", "3.2k", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("b6k4", "6.4k", -15.0, 15.0, 0.0, "dB"),
        ParamSpec::knob("level", "Level", -15.0, 15.0, 0.0, "dB"),
    ],
    create: |sr| Dual::boxed(|| Eq::new(sr)),
    file: None,
    tempo: None,
};

struct Eq {
    sr: f32,
    bands: [Biquad; 7],
    level: Smoother,
}

impl Eq {
    fn new(sr: f32) -> Self {
        Self {
            sr,
            bands: BANDS.map(|f| Biquad::new(FilterKind::Peak, sr, f, 1.4, 0.0)),
            level: Smoother::new(1.0, 20.0, sr),
        }
    }
}

impl MonoEffect for Eq {
    fn set_param(&mut self, index: usize, value: f32) {
        if index < BANDS.len() {
            self.bands[index].set(FilterKind::Peak, self.sr, BANDS[index], 1.4, value);
        } else if index == BANDS.len() {
            self.level.set(db_to_lin(value));
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let mut y = *x;
            for b in &mut self.bands {
                y = b.process(y);
            }
            *x = y * self.level.next();
        }
    }

    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Biquad::reset);
        self.level.snap();
    }
}
