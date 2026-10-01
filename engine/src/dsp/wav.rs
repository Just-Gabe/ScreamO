//! Leitor de WAV mínimo (PCM 16/24/32 bits e float 32/64) para IRs.

use anyhow::{Result, bail};

pub struct Wav {
    pub sample_rate: u32,
    /// Primeiro canal, normalizado em -1..1.
    pub samples: Vec<f32>,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn parse(data: &[u8]) -> Result<Wav> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        bail!("não é um arquivo WAV");
    }
    let mut fmt = None;
    let mut body = None;
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32_at(data, pos + 4) as usize;
        let start = pos + 8;
        let end = start.saturating_add(size).min(data.len());
        match id {
            b"fmt " if end - start >= 16 => {
                let mut tag = u16_at(data, start);
                if tag == 0xFFFE && end - start >= 40 {
                    // WAVE_FORMAT_EXTENSIBLE: o formato real está no SubFormat.
                    tag = u16_at(data, start + 24);
                }
                fmt = Some((tag, u16_at(data, start + 2), u32_at(data, start + 4), u16_at(data, start + 14)));
            }
            b"data" => body = Some(&data[start..end]),
            _ => {}
        }
        pos = start + size + (size & 1);
    }
    let (tag, channels, sample_rate, bits) = fmt.ok_or_else(|| anyhow::anyhow!("WAV sem bloco fmt"))?;
    let body = body.ok_or_else(|| anyhow::anyhow!("WAV sem dados"))?;
    if channels == 0 || sample_rate == 0 {
        bail!("WAV inválido");
    }
    let bytes = (bits as usize).div_ceil(8);
    let frame = bytes * channels as usize;
    let read: fn(&[u8]) -> f32 = match (tag, bits) {
        (1, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
        (1, 24) => |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        (3, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (3, 64) => |b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32,
        _ => bail!("formato WAV não suportado (tag {tag}, {bits} bits)"),
    };
    let samples: Vec<f32> = body.chunks_exact(frame).map(|f| read(&f[..bytes])).collect();
    if samples.is_empty() {
        bail!("WAV vazio");
    }
    if samples.iter().any(|x| !x.is_finite()) {
        bail!("WAV contém valores inválidos");
    }
    Ok(Wav { sample_rate, samples })
}

/// Reamostra com interpolação Hermite (suficiente para IRs curtas).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = ((input.len() as f64) / ratio).ceil() as usize;
    let at = |i: isize| -> f32 { if i < 0 || i as usize >= input.len() { 0.0 } else { input[i as usize] } };
    (0..len)
        .map(|n| {
            let pos = n as f64 * ratio;
            let i = pos.floor() as isize;
            let t = (pos - i as f64) as f32;
            let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            let c1 = 0.5 * (x1 - xm1);
            let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
            let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
            ((c3 * t + c2) * t + c1) * t + x0
        })
        .collect()
}

#[cfg(test)]
pub fn encode_f32(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let mut v = Vec::new();
    let data_len = (samples.len() * 4) as u32;
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_float_wav() {
        let wav = parse(&encode_f32(44100, &[0.5, -0.25, 0.0])).unwrap();
        assert_eq!(wav.sample_rate, 44100);
        assert_eq!(wav.samples, vec![0.5, -0.25, 0.0]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"not a wav file at all").is_err());
    }
}
