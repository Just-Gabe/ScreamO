//! Núcleo de DSP do ScreamO.
//!
//! Todo efeito é um módulo independente em `dsp/effects/` que expõe um
//! [`EffectDescriptor`] estático. Para adicionar um efeito novo basta criar o
//! arquivo e registrar o descritor em [`effects::REGISTRY`]. Veja
//! `docs/ADDING_EFFECTS.md`.

pub mod convolver;
pub mod effects;
pub mod nam;
pub mod util;
pub mod wav;

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
    /// Liga/desliga (0.0 / 1.0).
    Toggle,
    /// Botões que disparam ações (valor = índice + 1; 0 = nada). Não é
    /// guardado no estado nem em presets.
    Actions { options: &'static [&'static str] },
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

    pub const fn actions(id: &'static str, name: &'static str, options: &'static [&'static str]) -> Self {
        Self { id, name, min: 0.0, max: options.len() as f32, default: 0.0, unit: "", kind: ParamKind::Actions { options } }
    }

    pub fn is_action(&self) -> bool {
        matches!(self.kind, ParamKind::Actions { .. })
    }

    pub const fn toggle(id: &'static str, name: &'static str, default: bool) -> Self {
        Self { id, name, min: 0.0, max: 1.0, default: if default { 1.0 } else { 0.0 }, unit: "", kind: ParamKind::Toggle }
    }

    pub fn clamp(&self, v: f32) -> f32 {
        let v = if v.is_finite() { v } else { self.default };
        let v = v.clamp(self.min, self.max);
        match self.kind {
            ParamKind::Choice { .. } | ParamKind::Toggle | ParamKind::Actions { .. } => v.round(),
            ParamKind::Knob { .. } => v,
        }
    }
}

/// Tipo de arquivo que um efeito sabe carregar (IR de caixa, captura NAM...).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    Ir,
    Nam,
}

impl FileKind {
    pub const ALL: [FileKind; 2] = [FileKind::Ir, FileKind::Nam];

    pub fn extension(self) -> &'static str {
        match self {
            FileKind::Ir => "wav",
            FileKind::Nam => "nam",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let ext = name.rsplit('.').next()?.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.extension() == ext)
    }
}

/// Efeitos que dependem de um arquivo do usuário.
#[derive(Serialize)]
pub struct FileSupport {
    pub kind: FileKind,
    /// Cria a instância a partir do conteúdo do arquivo. Fora da thread de áudio.
    #[serde(skip)]
    pub create: fn(sample_rate: f32, data: &[u8]) -> anyhow::Result<Box<dyn Effect>>,
    /// Valida o arquivo antes de salvar na biblioteca.
    #[serde(skip)]
    pub validate: fn(data: &[u8]) -> anyhow::Result<()>,
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
    /// Sem arquivo carregado o efeito deve funcionar como passagem direta.
    #[serde(skip)]
    pub create: fn(sample_rate: f32) -> Box<dyn Effect>,
    pub file: Option<FileSupport>,
    /// Parâmetro que pode seguir o BPM global.
    pub tempo: Option<TempoSync>,
}

/// Subdivisões de tempo do parâmetro "Sync" (em semínimas).
pub const SYNC_OPTIONS: &[&str] = &["Off", "1/4", "1/8", "1/8 .", "1/8 T", "1/16"];
const SYNC_BEATS: [f32; 6] = [0.0, 1.0, 0.5, 0.75, 1.0 / 3.0, 0.25];

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TempoTarget {
    /// O alvo é um tempo em ms (delay).
    TimeMs,
    /// O alvo é uma frequência em Hz (tremolo, chorus).
    RateHz,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoSync {
    /// Índice do parâmetro Choice com `SYNC_OPTIONS`.
    pub sync_param: usize,
    /// Índice do parâmetro controlado (tempo ou rate).
    pub target_param: usize,
    pub target: TempoTarget,
}

impl TempoSync {
    /// Valor do alvo para o BPM e a subdivisão escolhida (None = sync desligado).
    pub fn value(&self, bpm: f32, sync_index: f32) -> Option<f32> {
        let beats = *SYNC_BEATS.get(sync_index.round() as usize)?;
        if beats == 0.0 {
            return None;
        }
        let ms = 60_000.0 / bpm * beats;
        Some(match self.target {
            TempoTarget::TimeMs => ms,
            TempoTarget::RateHz => 1000.0 / ms,
        })
    }
}

impl EffectDescriptor {
    pub fn defaults(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }

    pub fn param_index(&self, id: &str) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }

    /// Cria a instância já com os parâmetros aplicados. Se o arquivo não
    /// puder ser carregado, devolve a versão sem arquivo e o erro.
    pub fn instantiate_with(&self, sample_rate: f32, values: &[f32], file: Option<&[u8]>) -> (Box<dyn Effect>, Option<String>) {
        let mut error = None;
        let mut fx = match (&self.file, file) {
            (Some(fs), Some(data)) => match (fs.create)(sample_rate, data) {
                Ok(fx) => fx,
                Err(e) => {
                    error = Some(format!("{e:#}"));
                    (self.create)(sample_rate)
                }
            },
            _ => (self.create)(sample_rate),
        };
        for (i, spec) in self.params.iter().enumerate() {
            let v = values.get(i).copied().unwrap_or(spec.default);
            fx.set_param(i, spec.clamp(v));
        }
        fx.reset();
        (fx, error)
    }

    pub fn instantiate(&self, sample_rate: f32, values: &[f32]) -> Box<dyn Effect> {
        self.instantiate_with(sample_rate, values, None).0
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

#[cfg(test)]
mod tempo_tests {
    use super::*;

    #[test]
    fn subdivisions_match_musical_values() {
        let delay = TempoSync { sync_param: 0, target_param: 0, target: TempoTarget::TimeMs };
        assert_eq!(delay.value(120.0, 0.0), None); // Off
        assert_eq!(delay.value(120.0, 1.0), Some(500.0)); // 1/4
        assert_eq!(delay.value(120.0, 3.0), Some(375.0)); // 1/8 pontuada
        let trem = TempoSync { sync_param: 0, target_param: 0, target: TempoTarget::RateHz };
        assert_eq!(trem.value(120.0, 2.0), Some(4.0)); // 1/8 a 120 BPM = 4 Hz
    }
}
