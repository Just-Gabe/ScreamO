use anyhow::Result;

use crate::dsp::nam::{self, Model};
use crate::dsp::util::*;
use crate::dsp::*;

/// Loudness alvo da normalização (mesmo valor do plugin oficial).
const TARGET_LOUDNESS_DB: f64 = -18.0;

/// Captura de amp/pedal real por rede neural (Neural Amp Modeler).
/// Milhares de capturas gratuitas em tonehunt.org. Sem arquivo, deixa passar.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "nam",
    name: "NAM Capture",
    category: Category::Amp,
    description: "Carregue uma captura .nam (Neural Amp Modeler) de um amp ou pedal real.",
    color: "#10b981",
    params: &[
        ParamSpec::knob("input", "Input", -20.0, 20.0, 0.0, "dB"),
        ParamSpec::knob("output", "Output", -30.0, 12.0, 0.0, "dB"),
        ParamSpec::toggle("normalize", "Normalizar", true),
    ],
    create: |sr| Box::new(NamAmp::new(sr, None)),
    file: Some(FileSupport { kind: FileKind::Nam, create: create_with_model, validate }),
    tempo: None,
};

fn validate(data: &[u8]) -> Result<()> {
    nam::load(data).map(|_| ())
}

fn create_with_model(sr: f32, data: &[u8]) -> Result<Box<dyn Effect>> {
    let (model, info) = nam::load(data)?;
    Ok(Box::new(NamAmp::new(sr, Some((model, info)))))
}

struct NamAmp {
    model: Option<Model>,
    loudness_gain: f32,
    normalize: bool,
    input: Smoother,
    output: Smoother,
    dc: OnePole,
    buf: Vec<f32>,
}

impl NamAmp {
    fn new(sr: f32, model: Option<(Model, nam::ModelInfo)>) -> Self {
        let (model, loudness_gain) = match model {
            Some((m, info)) => {
                let g = info.loudness.map(|l| db_to_lin((TARGET_LOUDNESS_DB - l) as f32)).unwrap_or(1.0);
                (Some(m), g)
            }
            None => (None, 1.0),
        };
        Self {
            model,
            loudness_gain,
            normalize: true,
            input: Smoother::new(1.0, 20.0, sr),
            output: Smoother::new(1.0, 20.0, sr),
            dc: OnePole::highpass(sr, 10.0),
            buf: vec![0.0; MAX_BLOCK],
        }
    }
}

impl Effect for NamAmp {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.input.set(db_to_lin(value)),
            1 => self.output.set(db_to_lin(value)),
            2 => self.normalize = value >= 0.5,
            _ => {}
        }
    }

    /// Amp é mono: entrada estéreo é somada antes do modelo (rodar duas
    /// redes neurais dobraria o custo de CPU).
    fn process(&mut self, l: &mut [f32], r: &mut [f32], stereo: bool) -> bool {
        let Some(model) = &mut self.model else { return stereo };
        let n = l.len();
        for i in 0..n {
            let x = if stereo { 0.5 * (l[i] + r[i]) } else { l[i] };
            self.buf[i] = x * self.input.next();
        }
        model.process(&self.buf[..n], &mut l[..n]);
        let norm = if self.normalize { self.loudness_gain } else { 1.0 };
        for i in 0..n {
            let y = self.dc.process(l[i]) * norm * self.output.next();
            l[i] = if y.is_finite() { y } else { 0.0 };
        }
        r.copy_from_slice(l);
        false
    }

    /// Não aquece o modelo aqui: reset roda na thread de áudio e o prewarm
    /// (feito ao carregar) custaria milhares de amostras de uma vez.
    fn reset(&mut self) {
        self.dc.reset();
        self.input.snap();
        self.output.snap();
    }
}
