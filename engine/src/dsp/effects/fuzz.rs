use crate::dsp::util::*;
use crate::dsp::*;

/// Fuzz com bias ajustável (quanto menor o bias, mais "sputter"/gated fica) e
/// tone estilo Big Muff (mistura de LPF e HPF criando scoop nos médios).
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "fuzz",
    name: "Fuzz Face-Off",
    category: Category::Drive,
    description: "Fuzz sujo com bias ajustável. Do garage ao stoner.",
    color: "#a855f7",
    params: &[
        ParamSpec::knob("fuzz", "Fuzz", 0.0, 10.0, 7.0, ""),
        ParamSpec::knob("tone", "Tone", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("bias", "Bias", 0.0, 10.0, 6.0, ""),
        ParamSpec::knob("level", "Level", 0.0, 10.0, 5.0, ""),
    ],
    create: |sr| Dual::boxed(|| Fuzz::new(sr)),
    file: None,
};

struct Fuzz {
    input_hp: OnePole,
    dc: OnePole,
    lp: OnePole,
    hp: OnePole,
    os: Oversampler4,
    gain: Smoother,
    bias: Smoother,
    tone: Smoother,
    level: Smoother,
}

impl Fuzz {
    fn new(sr: f32) -> Self {
        Self {
            input_hp: OnePole::highpass(sr, 80.0),
            dc: OnePole::highpass(sr, 20.0),
            lp: OnePole::lowpass(sr, 700.0),
            hp: OnePole::highpass(sr, 1100.0),
            os: Oversampler4::new(sr),
            gain: Smoother::new(1.0, 30.0, sr),
            bias: Smoother::new(0.3, 30.0, sr),
            tone: Smoother::new(0.5, 30.0, sr),
            level: Smoother::new(1.0, 30.0, sr),
        }
    }
}

impl MonoEffect for Fuzz {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.gain.set(db_to_lin(knob(value, 20.0, 60.0))),
            1 => self.tone.set(value / 10.0),
            2 => self.bias.set(knob(value, 0.9, 0.05)),
            3 => self.level.set(db_to_lin(knob(value, -30.0, 0.0))),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let g = self.gain.next();
            let bias = self.bias.next();
            let s = self.input_hp.process(*x) * g;
            let y = self.os.process(s, |v| {
                // Transistor saturando: lado negativo clipa bem antes do positivo.
                let v = v + bias;
                let y = if v > 0.0 { v.tanh() } else { (v * 2.5).tanh() * 0.6 };
                y - bias.tanh()
            });
            let y = self.dc.process(y);
            let t = self.tone.next();
            let y = self.lp.process(y) * (1.0 - t) + self.hp.process(y) * t;
            *x = y * self.level.next() * 2.0;
        }
    }

    fn reset(&mut self) {
        self.input_hp.reset();
        self.dc.reset();
        self.lp.reset();
        self.hp.reset();
        self.os.reset();
        for s in [&mut self.gain, &mut self.bias, &mut self.tone, &mut self.level] {
            s.snap();
        }
    }
}
