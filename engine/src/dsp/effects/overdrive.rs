use crate::dsp::util::*;
use crate::dsp::*;

/// Overdrive inspirado no Tube Screamer: só as médias passam pelo clipping e
/// o sinal limpo é somado de volta, o que mantém o grave firme.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "overdrive",
    name: "Scream Drive",
    category: Category::Drive,
    description: "Overdrive com corcova de médios (estilo TS). Perfeito pra empurrar o amp.",
    color: "#22c55e",
    params: &[
        ParamSpec::knob("drive", "Drive", 0.0, 10.0, 4.0, ""),
        ParamSpec::knob("tone", "Tone", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("level", "Level", 0.0, 10.0, 6.0, ""),
    ],
    create: |sr| Dual::boxed(|| Overdrive::new(sr)),
};

struct Overdrive {
    sr: f32,
    pre_hp: OnePole,
    clip_lp: OnePole,
    tone: OnePole,
    os: Oversampler4,
    drive: Smoother,
    level: Smoother,
}

impl Overdrive {
    fn new(sr: f32) -> Self {
        Self {
            sr,
            pre_hp: OnePole::highpass(sr, 720.0),
            clip_lp: OnePole::lowpass(sr, 5500.0),
            tone: OnePole::lowpass(sr, 2000.0),
            os: Oversampler4::new(sr),
            drive: Smoother::new(1.0, 30.0, sr),
            level: Smoother::new(1.0, 30.0, sr),
        }
    }
}

impl MonoEffect for Overdrive {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.drive.set(db_to_lin(knob(value, 6.0, 46.0))),
            1 => self.tone.set_freq(self.sr, knob_exp(value, 600.0, 7000.0)),
            2 => self.level.set(db_to_lin(knob(value, -24.0, 8.0))),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let dry = *x;
            let g = self.drive.next();
            let mids = self.pre_hp.process(dry);
            let clipped = self.os.process(mids * g, |s| diode(s * 1.2) * 0.8);
            let y = self.clip_lp.process(clipped) + dry;
            *x = self.tone.process(y) * self.level.next();
        }
    }

    fn reset(&mut self) {
        self.pre_hp.reset();
        self.clip_lp.reset();
        self.tone.reset();
        self.os.reset();
        self.drive.snap();
        self.level.snap();
    }
}
