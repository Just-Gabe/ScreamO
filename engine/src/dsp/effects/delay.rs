use crate::dsp::util::*;
use crate::dsp::*;

const MAX_SECONDS: f32 = 2.0;

/// Delay estéreo com repetições que escurecem (estilo analógico) e modo
/// ping-pong. Mudar o tempo com o knob gera aquele "pitch warp" de fita.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "delay",
    name: "Delay",
    category: Category::Time,
    description: "Delay estéreo com repetições que escurecem e modo ping-pong.",
    color: "#eab308",
    params: &[
        ParamSpec::log_knob("time", "Time", 20.0, 2000.0, 380.0, "ms"),
        ParamSpec::knob("feedback", "Feedback", 0.0, 95.0, 35.0, "%"),
        ParamSpec::knob("mix", "Mix", 0.0, 100.0, 30.0, "%"),
        ParamSpec::log_knob("tone", "Tone", 800.0, 12000.0, 4500.0, "Hz"),
        ParamSpec::choice("mode", "Mode", &["Stereo", "Ping-Pong"], 0),
    ],
    create: |sr| Box::new(Delay::new(sr)),
    file: None,
};

struct Delay {
    sr: f32,
    dl: DelayLine,
    dr: DelayLine,
    time: Smoother,
    feedback: Smoother,
    mix: Smoother,
    lp_l: OnePole,
    lp_r: OnePole,
    hp_l: OnePole,
    hp_r: OnePole,
    ping_pong: bool,
}

impl Delay {
    fn new(sr: f32) -> Self {
        let max = (MAX_SECONDS * sr) as usize + 8;
        Self {
            sr,
            dl: DelayLine::new(max),
            dr: DelayLine::new(max),
            time: Smoother::new(0.38 * sr, 120.0, sr),
            feedback: Smoother::new(0.35, 30.0, sr),
            mix: Smoother::new(0.3, 30.0, sr),
            lp_l: OnePole::lowpass(sr, 4500.0),
            lp_r: OnePole::lowpass(sr, 4500.0),
            hp_l: OnePole::highpass(sr, 90.0),
            hp_r: OnePole::highpass(sr, 90.0),
            ping_pong: false,
        }
    }
}

impl Effect for Delay {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.time.set(value * 0.001 * self.sr),
            1 => self.feedback.set(value / 100.0),
            2 => self.mix.set(value / 100.0),
            3 => {
                self.lp_l.set_freq(self.sr, value);
                self.lp_r.set_freq(self.sr, value);
            }
            4 => self.ping_pong = value >= 0.5,
            _ => {}
        }
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], _stereo: bool) -> bool {
        for (xl, xr) in l.iter_mut().zip(r.iter_mut()) {
            let t = self.time.next();
            let fb = self.feedback.next();
            let mix = self.mix.next();
            let wl = self.dl.read(t);
            let wr = self.dr.read(t);
            let fl = self.hp_l.process(self.lp_l.process(wl)) * fb;
            let fr = self.hp_r.process(self.lp_r.process(wr)) * fb;
            if self.ping_pong {
                // Entrada mono vai só para a esquerda e as repetições cruzam.
                self.dl.push((*xl + *xr) * 0.5 + fr);
                self.dr.push(fl);
            } else {
                self.dl.push(*xl + fl);
                self.dr.push(*xr + fr);
            }
            *xl += wl * mix;
            *xr += wr * mix;
        }
        true
    }

    fn reset(&mut self) {
        self.dl.reset();
        self.dr.reset();
        self.time.snap();
        self.feedback.snap();
        self.mix.snap();
        for f in [&mut self.lp_l, &mut self.lp_r, &mut self.hp_l, &mut self.hp_r] {
            f.reset();
        }
    }
}
