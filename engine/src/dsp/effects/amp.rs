use crate::dsp::util::*;
use crate::dsp::*;

/// Simulação de amplificador valvulado em estágios:
/// entrada → pré (1 ou 2 estágios de válvula, oversampled) → tone stack →
/// power amp (saturação suave + presence) → master.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "amp",
    name: "ScreamO Amp",
    category: Category::Amp,
    description: "Amp valvulado de 3 canais: Clean, Crunch e Lead. Use com um Cabinet depois.",
    color: "#ef4444",
    params: &[
        ParamSpec::choice("channel", "Channel", &["Clean", "Crunch", "Lead"], 1),
        ParamSpec::knob("gain", "Gain", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("bass", "Bass", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("mid", "Middle", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("treble", "Treble", 0.0, 10.0, 5.5, ""),
        ParamSpec::knob("presence", "Presence", 0.0, 10.0, 5.0, ""),
        ParamSpec::knob("master", "Master", 0.0, 10.0, 5.0, ""),
    ],
    create: |sr| Dual::boxed(|| Amp::new(sr)),
    file: None,
    tempo: None,
};

#[derive(Clone, Copy)]
struct Voicing {
    tight_hz: f32,
    bright_db: f32,
    gain_lo_db: f32,
    gain_hi_db: f32,
    two_stages: bool,
    mid_scoop_db: f32,
    trim_db: f32,
}

const VOICINGS: [Voicing; 3] = [
    // Clean: headroom alto, quase linear.
    Voicing { tight_hz: 50.0, bright_db: 2.0, gain_lo_db: -6.0, gain_hi_db: 18.0, two_stages: false, mid_scoop_db: 0.0, trim_db: 0.0 },
    // Crunch: rock clássico.
    Voicing { tight_hz: 80.0, bright_db: 3.0, gain_lo_db: 8.0, gain_hi_db: 34.0, two_stages: true, mid_scoop_db: -1.0, trim_db: -2.0 },
    // Lead: alto ganho, grave apertado para palhetadas rápidas.
    Voicing { tight_hz: 130.0, bright_db: 4.0, gain_lo_db: 22.0, gain_hi_db: 52.0, two_stages: true, mid_scoop_db: -3.0, trim_db: -4.0 },
];

struct Amp {
    sr: f32,
    voicing: Voicing,
    gain_knob: f32,
    tight: Biquad,
    bright: Biquad,
    os: Oversampler4,
    coupling: OnePole,
    miller: OnePole,
    bass: Biquad,
    mid: Biquad,
    treble: Biquad,
    presence: Biquad,
    dc: OnePole,
    tone: [f32; 3],
    gain: Smoother,
    master: Smoother,
}

impl Amp {
    fn new(sr: f32) -> Self {
        let os_sr = sr * Oversampler4::FACTOR as f32;
        let mut amp = Self {
            sr,
            voicing: VOICINGS[1],
            gain_knob: 5.0,
            tight: Biquad::default(),
            bright: Biquad::default(),
            os: Oversampler4::new(sr),
            coupling: OnePole::highpass(os_sr, 25.0),
            miller: OnePole::lowpass(os_sr, 7500.0),
            bass: Biquad::default(),
            mid: Biquad::default(),
            treble: Biquad::default(),
            presence: Biquad::default(),
            dc: OnePole::highpass(sr, 10.0),
            tone: [5.0, 5.0, 5.5],
            gain: Smoother::new(1.0, 40.0, sr),
            master: Smoother::new(0.3, 40.0, sr),
        };
        amp.update_voicing();
        amp.update_tone();
        amp.set_param(5, 5.0);
        amp
    }

    fn update_voicing(&mut self) {
        let v = self.voicing;
        self.tight.set(FilterKind::HighPass, self.sr, v.tight_hz, 0.7, 0.0);
        self.bright.set(FilterKind::HighShelf, self.sr, 1500.0, 0.7, v.bright_db);
        self.gain.set(db_to_lin(knob(self.gain_knob, v.gain_lo_db, v.gain_hi_db)));
        self.update_tone();
    }

    fn update_tone(&mut self) {
        let [b, m, t] = self.tone;
        self.bass.set(FilterKind::LowShelf, self.sr, 120.0, 0.7, (b - 5.0) * 2.4);
        let mid_db = (m - 5.0) * 2.0 + self.voicing.mid_scoop_db;
        self.mid.set(FilterKind::Peak, self.sr, 650.0, 0.8, mid_db);
        self.treble.set(FilterKind::HighShelf, self.sr, 2800.0, 0.7, (t - 5.0) * 2.4);
    }
}

impl MonoEffect for Amp {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => {
                self.voicing = VOICINGS[(value as usize).min(2)];
                self.update_voicing();
            }
            1 => {
                self.gain_knob = value;
                let v = self.voicing;
                self.gain.set(db_to_lin(knob(value, v.gain_lo_db, v.gain_hi_db)));
            }
            2..=4 => {
                self.tone[index - 2] = value;
                self.update_tone();
            }
            5 => self.presence.set(FilterKind::HighShelf, self.sr, 4500.0, 0.7, (value - 5.0) * 2.0),
            6 => self.master.set(db_to_lin(knob(value, -30.0, 6.0))),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let two = self.voicing.two_stages;
        let trim = db_to_lin(self.voicing.trim_db);
        let (coupling, miller) = (&mut self.coupling, &mut self.miller);
        for x in buf.iter_mut() {
            let g = self.gain.next();
            let pre = self.bright.process(self.tight.process(*x)) * g;
            let y = self.os.process(pre, |s| {
                let mut y = tube(s, 0.18);
                if two {
                    y = miller.process(coupling.process(y));
                    y = tube(y * 3.0, -0.12);
                }
                y
            });
            let y = self.treble.process(self.mid.process(self.bass.process(y)));
            // Power amp: saturação suave e simétrica (push-pull).
            let y = (y * 1.3).tanh() / 1.3;
            let y = self.presence.process(self.dc.process(y));
            *x = y * trim * self.master.next();
        }
    }

    fn reset(&mut self) {
        for b in [&mut self.tight, &mut self.bright, &mut self.bass, &mut self.mid, &mut self.treble, &mut self.presence] {
            b.reset();
        }
        self.os.reset();
        self.coupling.reset();
        self.miller.reset();
        self.dc.reset();
        self.gain.snap();
        self.master.snap();
    }
}
