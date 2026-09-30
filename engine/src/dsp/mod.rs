//! Núcleo de DSP do ScreamO.
//!
//! Todo efeito é um módulo independente em `dsp/effects/` que expõe um
//! [`EffectDescriptor`] estático. Para adicionar um efeito novo basta criar o
//! arquivo e registrar o descritor em [`effects::REGISTRY`]. Veja
//! `docs/ADDING_EFFECTS.md`.

pub mod effects;
pub mod util;

use serde::Serialize;

/// Tamanho máximo de bloco processado de uma vez pela cadeia.
pub const MAX_BLOCK: usize = 512;

/// Interface que o motor usa para rodar qualquer efeito.
///
/// Regras de tempo real: `process` e `set_param` rodam na thread de áudio e
/// **não podem** alocar memória, travar mutex ou fazer I/O.
pub trait Effect: Send {
    /// Recebe o valor já na unidade do parâmetro (dB, ms, Hz, 0..10...).
    fn set_param(&mut self, index: usize, value: f32);

    /// Processa um bloco estéreo in-place. `stereo` indica se `r` carrega um
    /// sinal independente (senão é cópia de `l`). Retorna se a saída é estéreo.
    fn process(&mut self, l: &mut [f32], r: &mut [f32], stereo: bool) -> bool;

    /// Zera o estado interno (buffers de delay, envelopes...).
    fn reset(&mut self) {}
}

/// Forma simplificada para efeitos mono (drive, amp, cab, gate...).
/// Envolva com [`Dual`] para ganhar suporte estéreo automaticamente.
pub trait MonoEffect: Send {
    fn set_param(&mut self, index: usize, value: f32);
    fn process(&mut self, buf: &mut [f32]);
    fn reset(&mut self) {}
}

/// Adapta um [`MonoEffect`] para a cadeia estéreo mantendo duas instâncias:
/// enquanto o sinal é mono só uma trabalha; depois de um efeito estéreo
/// (chorus, delay, reverb) cada lado tem seu próprio estado.
pub struct Dual<T: MonoEffect> {
    a: T,
    b: T,
}

impl<T: MonoEffect> Dual<T> {
    pub fn new(make: impl Fn() -> T) -> Self {
        Self { a: make(), b: make() }
    }
}

impl<T: MonoEffect + 'static> Dual<T> {
    pub fn boxed(make: impl Fn() -> T) -> Box<dyn Effect> {
        Box::new(Self::new(make))
    }
}

impl<T: MonoEffect> Effect for Dual<T> {
    fn set_param(&mut self, index: usize, value: f32) {
        self.a.set_param(index, value);
        self.b.set_param(index, value);
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], stereo: bool) -> bool {
        self.a.process(l);
        if stereo {
            self.b.process(r);
        } else {
            r.copy_from_slice(l);
        }
        stereo
    }

    fn reset(&mut self) {
        self.a.reset();
        self.b.reset();
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Dynamics,
    Drive,
    Amp,
    Cabinet,
    Eq,
    Modulation,
    Time,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ParamKind {
    /// Knob contínuo. `log` indica mapeamento logarítmico na UI (Hz, ms).
    Knob { log: bool },
    /// Seletor discreto; o valor é o índice da opção.
    Choice { options: &'static [&'static str] },
    /// Liga/desliga (0.0 / 1.0). A UI já sabe desenhar; disponível para novos efeitos.
    #[allow(dead_code)]
    Toggle,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParamSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    #[serde(flatten)]
    pub kind: ParamKind,
}

impl ParamSpec {
    pub const fn knob(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> Self {
        Self { id, name, min, max, default, unit, kind: ParamKind::Knob { log: false } }
    }

    pub const fn log_knob(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> Self {
        Self { id, name, min, max, default, unit, kind: ParamKind::Knob { log: true } }
    }

    pub const fn choice(id: &'static str, name: &'static str, options: &'static [&'static str], default: usize) -> Self {
        Self {
            id,
            name,
            min: 0.0,
            max: (options.len() - 1) as f32,
            default: default as f32,
            unit: "",
            kind: ParamKind::Choice { options },
        }
    }

    pub fn clamp(&self, v: f32) -> f32 {
        let v = if v.is_finite() { v } else { self.default };
        let v = v.clamp(self.min, self.max);
        match self.kind {
            ParamKind::Choice { .. } | ParamKind::Toggle => v.round(),
            ParamKind::Knob { .. } => v,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectDescriptor {
    /// Identificador estável (usado em presets). Nunca mude depois de publicado.
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub description: &'static str,
    /// Cor de destaque do pedal na UI (hex).
    pub color: &'static str,
    pub params: &'static [ParamSpec],
    /// Cria uma instância para a taxa de amostragem dada. Chamado fora da
    /// thread de áudio, então pode alocar à vontade.
    #[serde(skip)]
    pub create: fn(sample_rate: f32) -> Box<dyn Effect>,
}

impl EffectDescriptor {
    pub fn defaults(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }

    pub fn param_index(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }

    /// Cria a instância já com os parâmetros aplicados.
    pub fn instantiate(&self, sample_rate: f32, values: &[f32]) -> Box<dyn Effect> {
        let mut fx = (self.create)(sample_rate);
        for (i, spec) in self.params.iter().enumerate() {
            let v = values.get(i).copied().unwrap_or(spec.default);
            fx.set_param(i, spec.clamp(v));
        }
        fx.reset();
        fx
    }
}

pub fn find(id: &str) -> Option<&'static EffectDescriptor> {
    effects::REGISTRY.iter().copied().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sinal parecido com guitarra: harmônicos decaindo + ruído.
    fn guitar_like(sr: f32, n: usize) -> Vec<f32> {
        let mut seed = 12345u32;
        (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                let env = (-t * 2.0).exp();
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
                let w = 2.0 * std::f32::consts::PI * 110.0 * t;
                env * (0.3 * w.sin() + 0.15 * (2.0 * w).sin() + 0.05 * (3.0 * w).sin()) + noise * 0.002
            })
            .collect()
    }

    fn run(desc: &EffectDescriptor, sr: f32, values: &[f32]) -> (f32, f32) {
        let mut fx = desc.instantiate(sr, values);
        let input = guitar_like(sr, sr as usize);
        let (mut peak, mut energy) = (0f32, 0f32);
        let mut stereo = false;
        for block in input.chunks(MAX_BLOCK) {
            let mut l = block.to_vec();
            let mut r = block.to_vec();
            stereo = fx.process(&mut l, &mut r, stereo);
            for &x in l.iter().chain(r.iter()) {
                assert!(x.is_finite(), "{} gerou NaN/inf", desc.id);
                peak = peak.max(x.abs());
                energy += x * x;
            }
        }
        (peak, energy)
    }

    #[test]
    fn every_effect_is_stable_and_audible() {
        for desc in effects::REGISTRY {
            for sr in [44100.0, 48000.0, 96000.0] {
                let (peak, energy) = run(desc, sr, &desc.defaults());
                assert!(peak < 8.0, "{} @ {sr}: pico {peak} alto demais", desc.id);
                if desc.id != "gate" {
                    assert!(energy > 1e-3, "{} @ {sr}: saída muda (energia {energy})", desc.id);
                }
            }
        }
    }

    #[test]
    fn extreme_params_stay_finite() {
        for desc in effects::REGISTRY {
            let max: Vec<f32> = desc.params.iter().map(|p| p.max).collect();
            let min: Vec<f32> = desc.params.iter().map(|p| p.min).collect();
            run(desc, 48000.0, &max);
            run(desc, 48000.0, &min);
        }
    }

    #[test]
    fn registry_ids_are_unique_and_defaults_valid() {
        let mut ids: Vec<_> = effects::REGISTRY.iter().map(|d| d.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), effects::REGISTRY.len());
        for d in effects::REGISTRY {
            for p in d.params {
                assert!(p.min <= p.default && p.default <= p.max, "{}.{}", d.id, p.id);
            }
        }
    }
}

#[cfg(test)]
mod level_report {
    use super::*;

    #[test]
    #[ignore]
    fn print_levels() {
        let sr = 48000.0;
        let n = sr as usize;
        let input: Vec<f32> = (0..n).map(|i| 0.2 * (2.0 * std::f32::consts::PI * 196.0 * i as f32 / sr).sin()).collect();
        let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        for d in effects::REGISTRY {
            let mut fx = d.instantiate(sr, &d.defaults());
            let mut out = Vec::new();
            for b in input.chunks(MAX_BLOCK) {
                let (mut l, mut r) = (b.to_vec(), b.to_vec());
                fx.process(&mut l, &mut r, false);
                out.extend(l);
            }
            println!("{:12} {:+6.1} dB  peak {:.2}", d.id, util::lin_to_db(rms(&out[n / 2..]) / rms(&input)), out.iter().fold(0f32, |a, x| a.max(x.abs())));
        }
    }
}
