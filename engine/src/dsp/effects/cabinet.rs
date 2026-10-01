use crate::dsp::util::*;
use crate::dsp::*;

/// Simulação de caixa por filtros (ressonância de graves, "honk" dos médios,
/// pico de presença do cone e corte agudo íngreme). Um carregador de IR
/// (.wav) está no roadmap; este módulo funciona sem arquivos externos.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "cabinet",
    name: "Cabinet",
    category: Category::Cabinet,
    description: "Caixa de guitarra microfonada. Sem ela o amp soa como abelha no fone!",
    color: "#a16207",
    params: &[
        ParamSpec::choice("model", "Model", &["1x12 Open", "2x12 Combo", "4x12 Closed"], 1),
        ParamSpec::log_knob("lowCut", "Low Cut", 40.0, 250.0, 75.0, "Hz"),
        ParamSpec::log_knob("highCut", "High Cut", 2500.0, 12000.0, 5500.0, "Hz"),
        ParamSpec::knob("mic", "Mic Pos", 0.0, 10.0, 5.0, ""),
    ],
    create: |sr| Dual::boxed(|| Cabinet::new(sr)),
    file: None,
};

#[derive(Clone, Copy)]
struct Model {
    thump_hz: f32,
    thump_db: f32,
    thump_q: f32,
    honk_hz: f32,
    honk_db: f32,
    cone_hz: f32,
    cone_db: f32,
}

const MODELS: [Model; 3] = [
    Model { thump_hz: 120.0, thump_db: 2.0, thump_q: 1.1, honk_hz: 900.0, honk_db: 1.5, cone_hz: 1900.0, cone_db: 3.0 },
    Model { thump_hz: 105.0, thump_db: 3.5, thump_q: 1.3, honk_hz: 450.0, honk_db: -3.0, cone_hz: 2300.0, cone_db: 4.0 },
    Model { thump_hz: 90.0, thump_db: 5.0, thump_q: 1.5, honk_hz: 500.0, honk_db: -4.5, cone_hz: 2700.0, cone_db: 5.0 },
];

struct Cabinet {
    sr: f32,
    model: Model,
    low_cut: f32,
    high_cut: f32,
    mic: f32,
    hp: Biquad,
    thump: Biquad,
    honk: Biquad,
    cone: Biquad,
    mic_shelf: Biquad,
    lp1: Biquad,
    lp2: Biquad,
}

impl Cabinet {
    fn new(sr: f32) -> Self {
        let mut c = Self {
            sr,
            model: MODELS[1],
            low_cut: 75.0,
            high_cut: 5500.0,
            mic: 5.0,
            hp: Biquad::default(),
            thump: Biquad::default(),
            honk: Biquad::default(),
            cone: Biquad::default(),
            mic_shelf: Biquad::default(),
            lp1: Biquad::default(),
            lp2: Biquad::default(),
        };
        c.update();
        c
    }

    fn update(&mut self) {
        let (sr, m) = (self.sr, self.model);
        self.hp.set(FilterKind::HighPass, sr, self.low_cut, 0.8, 0.0);
        self.thump.set(FilterKind::Peak, sr, m.thump_hz, m.thump_q, m.thump_db);
        self.honk.set(FilterKind::Peak, sr, m.honk_hz, 0.9, m.honk_db);
        // Mic no centro do cone = mais brilho; na borda = mais escuro.
        let mic_db = (self.mic - 5.0) * 1.2;
        self.cone.set(FilterKind::Peak, sr, m.cone_hz, 1.2, m.cone_db + mic_db * 0.5);
        self.mic_shelf.set(FilterKind::HighShelf, sr, 3500.0, 0.7, mic_db);
        // Butterworth de 4ª ordem: o cone de um falante de guitarra cai rápido.
        self.lp1.set(FilterKind::LowPass, sr, self.high_cut, 0.541, 0.0);
        self.lp2.set(FilterKind::LowPass, sr, self.high_cut, 1.307, 0.0);
    }
}

impl MonoEffect for Cabinet {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.model = MODELS[(value as usize).min(2)],
            1 => self.low_cut = value,
            2 => self.high_cut = value,
            3 => self.mic = value,
            _ => return,
        }
        self.update();
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            let mut y = self.hp.process(*x);
            y = self.thump.process(y);
            y = self.honk.process(y);
            y = self.cone.process(y);
            y = self.mic_shelf.process(y);
            y = self.lp2.process(self.lp1.process(y));
            *x = y;
        }
    }

    fn reset(&mut self) {
        for b in [&mut self.hp, &mut self.thump, &mut self.honk, &mut self.cone, &mut self.mic_shelf, &mut self.lp1, &mut self.lp2] {
            b.reset();
        }
    }
}
