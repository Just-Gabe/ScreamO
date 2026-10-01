use crate::dsp::util::*;
use crate::dsp::*;

/// Reverb estéreo baseado no Freeverb (Jezar): 8 filtros comb com
/// amortecimento + 4 all-pass por canal, com pré-delay.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "reverb",
    name: "Reverb",
    category: Category::Time,
    description: "Reverb de sala/hall estéreo com pré-delay.",
    color: "#8b5cf6",
    params: &[
        ParamSpec::knob("size", "Size", 0.0, 100.0, 60.0, "%"),
        ParamSpec::knob("damping", "Damping", 0.0, 100.0, 45.0, "%"),
        ParamSpec::knob("mix", "Mix", 0.0, 100.0, 22.0, "%"),
        ParamSpec::knob("predelay", "Pre-delay", 0.0, 120.0, 15.0, "ms"),
        ParamSpec::knob("width", "Width", 0.0, 100.0, 100.0, "%"),
    ],
    create: |sr| Box::new(Reverb::new(sr)),
    file: None,
};

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const FIXED_GAIN: f32 = 0.015;

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx = if self.idx + 1 >= self.buf.len() { 0 } else { self.idx + 1 };
        out
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.store = 0.0;
    }
}

struct AllPass {
    buf: Vec<f32>,
    idx: usize,
}

impl AllPass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx = if self.idx + 1 >= self.buf.len() { 0 } else { self.idx + 1 };
        b - x
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
    }
}

struct Reverb {
    sr: f32,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<AllPass>; 2],
    pre: DelayLine,
    predelay: Smoother,
    feedback: f32,
    damp: f32,
    mix: Smoother,
    width: f32,
}

impl Reverb {
    fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let len = |n: usize, spread: usize| ((n + spread) as f32 * scale) as usize;
        let combs = [0, STEREO_SPREAD].map(|s| COMB_TUNING.iter().map(|&n| Comb::new(len(n, s))).collect());
        let allpasses = [0, STEREO_SPREAD].map(|s| ALLPASS_TUNING.iter().map(|&n| AllPass::new(len(n, s))).collect());
        Self {
            sr,
            combs,
            allpasses,
            pre: DelayLine::new((0.13 * sr) as usize),
            predelay: Smoother::new(0.015 * sr, 100.0, sr),
            feedback: 0.84,
            damp: 0.2,
            mix: Smoother::new(0.22, 30.0, sr),
            width: 1.0,
        }
    }
}

impl Effect for Reverb {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.feedback = 0.7 + 0.28 * value / 100.0,
            1 => self.damp = 0.4 * value / 100.0,
            2 => self.mix.set(value / 100.0),
            3 => self.predelay.set((value * 0.001 * self.sr).max(1.0)),
            4 => self.width = value / 100.0,
            _ => {}
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], _stereo: bool) -> bool {
        let wet1 = 0.5 * (1.0 + self.width);
        let wet2 = 0.5 * (1.0 - self.width);
        let [cl, cr] = &mut self.combs;
        let [al, ar] = &mut self.allpasses;
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            self.pre.push((*xl + *xr) * 0.5);
            let input = self.pre.read(self.predelay.next()) * FIXED_GAIN;
            let mut ol = 0.0;
            let mut or = 0.0;
            for c in cl.iter_mut() {
                ol += c.process(input, self.feedback, self.damp);
            }
            for c in cr.iter_mut() {
                or += c.process(input, self.feedback, self.damp);
            }
            for a in al.iter_mut() {
                ol = a.process(ol);
            }
            for a in ar.iter_mut() {
                or = a.process(or);
            }
            let mix = self.mix.next() * 3.0;
            *xl += (ol * wet1 + or * wet2) * mix;
            *xr += (or * wet1 + ol * wet2) * mix;
        }
        true
    }

    fn reset(&mut self) {
        self.combs.iter_mut().flatten().for_each(Comb::reset);
        self.allpasses.iter_mut().flatten().for_each(AllPass::reset);
        self.pre.reset();
        self.predelay.snap();
        self.mix.snap();
    }
}
