//! Biblioteca de arquivos do usuário (IRs .wav e capturas .nam) em
//! `%APPDATA%\ScreamO\files`.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde::Serialize;

use crate::dsp::{self, FileKind};

/// Limite de upload (capturas NAM grandes têm poucos MB; IRs, KB).
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Default)]
pub struct FileList {
    pub ir: Vec<String>,
    pub nam: Vec<String>,
}

#[derive(Clone)]
pub struct FileLibrary {
    dir: PathBuf,
}

/// Nome de arquivo seguro: sem caminhos, com extensão conhecida.
pub fn sanitize(name: &str) -> Result<(String, FileKind)> {
    let name = name.trim();
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let kind = FileKind::from_name(base).ok_or_else(|| anyhow!("use arquivos .wav (IR) ou .nam (captura NAM)"))?;
    let stem = &base[..base.len() - kind.extension().len() - 1];
    let clean: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')' | '.') { c } else { '_' })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_string();
    if clean.is_empty() || clean.chars().count() > 80 {
        bail!("nome de arquivo inválido");
    }
    Ok((format!("{clean}.{}", kind.extension()), kind))
}

impl FileLibrary {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn list(&self) -> FileList {
        let mut list = FileList::default();
        for entry in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            match FileKind::from_name(&name) {
                Some(FileKind::Ir) => list.ir.push(name),
                Some(FileKind::Nam) => list.nam.push(name),
                None => {}
            }
        }
        list.ir.sort_by_key(|n| n.to_lowercase());
        list.nam.sort_by_key(|n| n.to_lowercase());
        list
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let (name, _) = sanitize(name)?;
        std::fs::read(self.dir.join(&name)).map_err(|_| anyhow!("arquivo '{name}' não está mais na biblioteca"))
    }

    /// Valida (o arquivo precisa carregar de verdade) e salva. Retorna o nome final.
    pub fn save(&self, name: &str, data: &[u8]) -> Result<String> {
        if data.len() > MAX_FILE_BYTES {
            bail!("arquivo grande demais (máx. {} MB)", MAX_FILE_BYTES / 1024 / 1024);
        }
        let (name, kind) = sanitize(name)?;
        let support = dsp::effects::REGISTRY
            .iter()
            .filter_map(|d| d.file.as_ref())
            .find(|f| f.kind == kind)
            .ok_or_else(|| anyhow!("nenhum efeito usa esse tipo de arquivo"))?;
        (support.validate)(data)?;
        std::fs::write(self.dir.join(&name), data)?;
        Ok(name)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let (name, _) = sanitize(name)?;
        Ok(std::fs::remove_file(self.dir.join(name))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_path_traversal() {
        assert_eq!(sanitize("..\\..\\Windows\\evil.wav").unwrap().0, "evil.wav");
        assert_eq!(sanitize("../x/../../a b.nam").unwrap().0, "a b.nam");
        assert!(sanitize("evil.exe").is_err());
        assert!(sanitize("...wav").is_err());
        assert_eq!(sanitize("Mesa 4x12 <V30>.wav").unwrap().0, "Mesa 4x12 _V30_.wav");
    }
}
