//! Presets: arquivos JSON que guardam a cadeia por *id* de parâmetro (não por
//! índice), então continuam válidos mesmo se um efeito ganhar knobs novos.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetSlot {
    pub effect: String,
    #[serde(default)]
    pub bypass: bool,
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub chain: Vec<PresetSlot>,
}

fn slot(effect: &str, bypass: bool, params: &[(&str, f32)]) -> PresetSlot {
    PresetSlot {
        effect: effect.into(),
        bypass,
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}

pub fn default_chain() -> Vec<PresetSlot> {
    factory().into_iter().find(|p| p.name == "Crunch Clássico").map(|p| p.chain).unwrap_or_default()
}

pub fn factory() -> Vec<Preset> {
    vec![
        Preset {
            name: "Clean Cristalino".into(),
            chain: vec![
                slot("compressor", false, &[("threshold", -26.0), ("ratio", 4.0), ("makeup", 7.0)]),
                slot("amp", false, &[("channel", 0.0), ("gain", 4.0), ("treble", 6.0), ("master", 6.0)]),
                slot("cabinet", false, &[("model", 0.0), ("highCut", 7000.0)]),
                slot("chorus", false, &[("rate", 0.7), ("depth", 4.0), ("mix", 35.0)]),
                slot("delay", true, &[]),
                slot("reverb", false, &[("size", 70.0), ("mix", 28.0)]),
            ],
        },
        Preset {
            name: "Crunch Clássico".into(),
            chain: vec![
                slot("gate", false, &[("threshold", -62.0)]),
                slot("overdrive", true, &[("drive", 3.0), ("level", 7.0)]),
                slot("amp", false, &[("channel", 1.0), ("gain", 6.0), ("master", 5.5)]),
                slot("cabinet", false, &[("model", 1.0)]),
                slot("delay", true, &[]),
                slot("reverb", false, &[("mix", 15.0)]),
            ],
        },
        Preset {
            name: "ScreamO Lead".into(),
            chain: vec![
                slot("gate", false, &[("threshold", -56.0)]),
                slot("overdrive", false, &[("drive", 1.0), ("tone", 6.0), ("level", 8.0)]),
                slot("amp", false, &[("channel", 2.0), ("gain", 6.5), ("mid", 6.0), ("master", 5.0)]),
                slot("cabinet", false, &[("model", 2.0)]),
                slot("delay", false, &[("time", 420.0), ("feedback", 30.0), ("mix", 22.0), ("mode", 1.0)]),
                slot("reverb", false, &[("mix", 20.0)]),
            ],
        },
        Preset {
            name: "Metalcore Rhythm".into(),
            chain: vec![
                slot("gate", false, &[("threshold", -50.0), ("hold", 15.0), ("release", 40.0)]),
                slot("overdrive", false, &[("drive", 0.0), ("tone", 6.0), ("level", 8.5)]),
                slot("amp", false, &[("channel", 2.0), ("gain", 7.0), ("bass", 6.0), ("mid", 4.0), ("treble", 6.5), ("presence", 6.0)]),
                slot("cabinet", false, &[("model", 2.0), ("lowCut", 90.0)]),
                slot("eq", false, &[("b400", -2.0), ("b1k6", 1.5)]),
                slot("reverb", false, &[("size", 40.0), ("mix", 8.0)]),
            ],
        },
        Preset {
            name: "Fuzz Garage".into(),
            chain: vec![
                slot("fuzz", false, &[("fuzz", 8.0), ("tone", 5.5)]),
                slot("amp", false, &[("channel", 0.0), ("gain", 5.0)]),
                slot("cabinet", false, &[("model", 1.0)]),
                slot("tremolo", true, &[]),
                slot("reverb", false, &[("mix", 20.0)]),
            ],
        },
        Preset {
            name: "Shoegaze Wall".into(),
            chain: vec![
                slot("fuzz", false, &[("fuzz", 7.0), ("tone", 4.0)]),
                slot("amp", false, &[("channel", 1.0), ("gain", 4.0)]),
                slot("cabinet", false, &[("model", 1.0)]),
                slot("chorus", false, &[("rate", 0.4), ("depth", 6.0), ("mix", 40.0)]),
                slot("delay", false, &[("time", 550.0), ("feedback", 55.0), ("mix", 40.0), ("mode", 1.0)]),
                slot("reverb", false, &[("size", 92.0), ("mix", 45.0), ("predelay", 40.0)]),
            ],
        },
    ]
}

/// Aceita só nomes seguros para arquivo (sem `..`, barras etc.).
pub fn sanitize_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 48 {
        bail!("nome de preset deve ter entre 1 e 48 caracteres");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')')) {
        bail!("use apenas letras, números, espaço, - _ ( )");
    }
    Ok(name.to_string())
}

pub struct PresetStore {
    dir: PathBuf,
}

impl PresetStore {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.dir.join(format!("{}.json", sanitize_name(name)?)))
    }

    pub fn list_user(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension()? != "json" {
                    return None;
                }
                Some(p.file_stem()?.to_string_lossy().to_string())
            })
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    pub fn load(&self, name: &str) -> Result<Preset> {
        if let Some(p) = factory().into_iter().find(|p| p.name == name) {
            return Ok(p);
        }
        read_json(&self.path(name)?)
    }

    pub fn save(&self, preset: &Preset) -> Result<()> {
        if factory().iter().any(|p| p.name == preset.name) {
            bail!("esse nome é de um preset de fábrica, escolha outro");
        }
        write_json(&self.path(&preset.name)?, preset)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        Ok(std::fs::remove_file(self.path(name)?)?)
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
