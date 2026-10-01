use std::sync::Arc;

use anyhow::{Result, bail};

use crate::dsp::convolver::{Convolver, IrSpectra};
use crate::dsp::util::*;
use crate::dsp::*;

/// IR máxima aceita (500 ms já cobre caixas e salas pequenas).
const MAX_IR_SECONDS: f32 = 0.5;

/// Caixa por resposta a impulso (.wav), a forma mais realista de simular
/// caixa + microfone. Sem arquivo carregado, deixa o som passar.
pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "irCab",
    name: "IR Cabinet",
    category: Category::Cabinet,
    description: "Carregue uma IR (.wav) de caixa real. Use no lugar do Cabinet.",
    color: "#d97706",
    params: &[
        ParamSpec::knob("mix", "Mix", 0.0, 100.0, 100.0, "%"),
        ParamSpec::log_knob("lowCut", "Low Cut", 20.0, 300.0, 40.0, "Hz"),
        ParamSpec::log_knob("highCut", "High Cut", 2000.0, 20000.0, 16000.0, "Hz"),
        ParamSpec::knob("level", "Level", -24.0, 12.0, 0.0, "dB"),
    ],
    create: |sr| Dual::boxed(|| IrCab::new(sr, None)),
    file: Some(FileSupport { kind: FileKind::Ir, create: create_with_ir, validate }),
};

fn validate(data: &[u8]) -> Result<()> {
    let wav = crate::dsp::wav::parse(data)?;
    if wav.samples.iter().all(|x| *x == 0.0) {
        bail!("IR silenciosa");
    }
    Ok(())
}

/// Prepara a IR: reamostra para a taxa do dispositivo, corta silêncio do fim
/// e normaliza a energia para o volume não mudar entre IRs diferentes.
pub fn prepare_ir(data: &[u8], sample_rate: f32) -> Result<Vec<f32>> {
    let wav = crate::dsp::wav::parse(data)?;
    let mut ir = crate::dsp::wav::resample(&wav.samples, wav.sample_rate, sample_rate as u32);
    ir.truncate((MAX_IR_SECONDS * sample_rate) as usize);
    let peak = ir.iter().fold(0f32, |a, x| a.max(x.abs()));
    if peak <= 0.0 {
        bail!("IR silenciosa");
    }
    let last = ir.iter().rposition(|x| x.abs() > peak * 1e-4).unwrap_or(0);
    ir.truncate(last + 1);
    let energy: f32 = ir.iter().map(|x| x * x).sum();
    let norm = 1.0 / energy.sqrt();
    ir.iter_mut().for_each(|x| *x *= norm);
    Ok(ir)
}

fn create_with_ir(sr: f32, data: &[u8]) -> Result<Box<dyn Effect>> {
    let spectra = IrSpectra::new(&prepare_ir(data, sr)?);
    Ok(Dual::boxed(|| IrCab::new(sr, Some(spectra.clone()))))
}

struct IrCab {
    sr: f32,
    conv: Option<Convolver>,
    hp: Biquad,
    lp: Biquad,
    mix: Smoother,
    level: Smoother,
}

impl IrCab {
    fn new(sr: f32, ir: Option<Arc<IrSpectra>>) -> Self {
        Self {
            sr,
            conv: ir.map(Convolver::new),
            hp: Biquad::new(FilterKind::HighPass, sr, 40.0, 0.7, 0.0),
            lp: Biquad::new(FilterKind::LowPass, sr, 16000.0, 0.7, 0.0),
            mix: Smoother::new(1.0, 20.0, sr),
            level: Smoother::new(1.0, 20.0, sr),
        }
    }
}

impl MonoEffect for IrCab {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.mix.set(value / 100.0),
            1 => self.hp.set(FilterKind::HighPass, self.sr, value, 0.7, 0.0),
            2 => self.lp.set(FilterKind::LowPass, self.sr, value, 0.7, 0.0),
            3 => self.level.set(db_to_lin(value)),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let Some(conv) = &mut self.conv else { return };
        for x in buf.iter_mut() {
            let dry = *x;
            let wet = conv.process_sample(dry);
            let m = self.mix.next();
            let y = wet * m + dry * (1.0 - m);
            *x = self.lp.process(self.hp.process(y)) * self.level.next();
        }
    }

    fn reset(&mut self) {
        if let Some(c) = &mut self.conv {
            c.reset();
        }
        self.hp.reset();
        self.lp.reset();
        self.mix.snap();
        self.level.snap();
    }
}
