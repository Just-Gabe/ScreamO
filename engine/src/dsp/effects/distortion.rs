use crate::dsp::util::*;
use crate::dsp::*;

/// Distorção de alto ganho com EQ ativo de médios (pensada pra metal/screamo).
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "distortion",
    name: "Buzzsaw",
    category: Category::Drive,
    description: "Distorção de alto ganho com médio paramétrico. Riff pesado e apertado.",
    color: "#f97316",
    params: &[
        ParamSpec::knob("gain", "Gain", 0.0, 10.0, 6.0, ""),
        ParamSpec::knob("low", "Low", -12.0, 12.0, 2.0, "dB"),
        ParamSpec::knob("mid", "Mid", -15.0, 15.0, -4.0, "dB"),
        ParamSpec::log_knob("midFreq", "Mid Freq", 200.0, 4000.0, 800.0, "Hz"),
        ParamSpec::knob("high", "High", -12.0, 12.0, 3.0, "dB"),
        ParamSpec::knob("level", "Level", 0.0, 10.0, 5.0, ""),
    ],
    create: |sr| Dual::boxed(|| Distortion::new(sr)),
    file: None,
    tempo: None,
};

struct Distortion {
    sr: f32,
    tight: Biquad,
    pre_mid: Biquad,
    inter_hp: OnePole,
    inter_lp: OnePole,
    os: Oversampler4,
    low: Biquad,
    mid: Biquad,
    high: Biquad,
    mid_db: f32,
    mid_freq: f32,
    gain: Smoother,
    level: Smoother,
}

impl Distortion {
    fn new(sr: f32) -> Self {
        let os_sr = sr * Oversampler4::FACTOR as f32;
        Self {
            sr,
            tight: Biquad::new(FilterKind::HighPass, sr, 140.0, 0.7, 0.0),
            pre_mid: Biquad::new(FilterKind::Peak, sr, 1200.0, 0.8, 6.0),
            inter_hp: OnePole::highpass(os_sr, 90.0),
            inter_lp: OnePole::lowpass(os_sr, 9000.0),
            os: Oversampler4::new(sr),
            low: Biquad::new(FilterKind::LowShelf, sr, 110.0, 0.7, 0.0),
            mid: Biquad::new(FilterKind::Peak, sr, 800.0, 0.9, 0.0),
            high: Biquad::new(FilterKind::HighShelf, sr, 3500.0, 0.7, 0.0),
            mid_db: -4.0,
            mid_freq: 800.0,
            gain: Smoother::new(1.0, 30.0, sr),
            level: Smoother::new(1.0, 30.0, sr),
        }
    }
}

impl MonoEffect for Distortion {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.gain.set(db_to_lin(knob(value, 14.0, 58.0))),
            1 => self.low.set(FilterKind::LowShelf, self.sr, 110.0, 0.7, value),
            2 | 3 => {
                if index == 2 { self.mid_db = value } else { self.mid_freq = value }
                self.mid.set(FilterKind::Peak, self.sr, self.mid_freq, 0.9, self.mid_db);
            }
            4 => self.high.set(FilterKind::HighShelf, self.sr, 3500.0, 0.7, value),
            5 => self.level.set(db_to_lin(knob(value, -34.0, 0.0))),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let (hp, lp) = (&mut self.inter_hp, &mut self.inter_lp);
        for x in buf.iter_mut() {
            let g = self.gain.next();
            let pre = self.pre_mid.process(self.tight.process(*x)) * g;
            let y = self.os.process(pre, |s| {
                let s1 = tube(s, 0.25);
                let s2 = lp.process(hp.process(s1)) * 4.0;
                (s2 * 1.5).tanh()
            });
            let y = self.high.process(self.mid.process(self.low.process(y)));
            *x = y * self.level.next();
        }
    }

    fn reset(&mut self) {
        for b in [&mut self.tight, &mut self.pre_mid, &mut self.low, &mut self.mid, &mut self.high] {
            b.reset();
        }
        self.inter_hp.reset();
        self.inter_lp.reset();
        self.os.reset();
        self.gain.snap();
        self.level.snap();
    }
}
