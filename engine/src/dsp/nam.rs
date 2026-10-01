//! Inferência de modelos Neural Amp Modeler (`.nam`).
//!
//! Porte das arquiteturas WaveNet "A1" (standard/lite/feather/nano, com ou
//! sem gating) e LSTM do NeuralAmpModelerCore, mantendo a mesma ordem de
//! pesos. Recursos das versões mais novas (FiLM, condition DSP, grupos,
//! head1x1, post-stack head) são recusados com mensagem clara.
//!
//! Como o plugin oficial (que chama `enable_fast_tanh()`), `Tanh` usa a
//! aproximação rápida do core, e o LSTM usa sigmoid/tanh rápidos.
//!
//! Tudo é alocado na criação; `process` não aloca.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use super::MAX_BLOCK;

/// Metadados úteis do arquivo.
#[derive(Debug, Clone, Default)]
pub struct ModelInfo {
    pub sample_rate: Option<f64>,
    /// Loudness da saída em dB (para normalizar o volume entre capturas).
    pub loudness: Option<f64>,
}

pub enum Model {
    WaveNet(Box<WaveNet>),
    Lstm(Box<Lstm>),
}

impl Model {
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for (i, o) in input.chunks(MAX_BLOCK).zip(output.chunks_mut(MAX_BLOCK)) {
            match self {
                Model::WaveNet(w) => w.process_block(i, o),
                Model::Lstm(l) => l.process_block(i, o),
            }
        }
    }

    /// Aquece o estado interno com silêncio (como o plugin oficial faz).
    pub fn prewarm(&mut self) {
        let samples = match self {
            Model::WaveNet(w) => w.receptive_field() + 1,
            Model::Lstm(_) => 24000,
        };
        let zeros = vec![0f32; MAX_BLOCK];
        let mut out = vec![0f32; MAX_BLOCK];
        let mut left = samples;
        while left > 0 {
            let n = left.min(MAX_BLOCK);
            self.process(&zeros[..n], &mut out[..n]);
            left -= n;
        }
    }
}

pub fn parse_info(json: &Value) -> ModelInfo {
    let meta = &json["metadata"];
    ModelInfo {
        sample_rate: json["sample_rate"].as_f64(),
        loudness: meta["loudness"].as_f64(),
    }
}

pub fn load(data: &[u8]) -> Result<(Model, ModelInfo)> {
    let json: Value = serde_json::from_slice(data).context("arquivo .nam não é JSON válido")?;
    let info = parse_info(&json);
    let weights: Vec<f32> = json["weights"]
        .as_array()
        .ok_or_else(|| anyhow!(".nam sem pesos"))?
        .iter()
        .map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| anyhow!("peso inválido")))
        .collect::<Result<_>>()?;
    if weights.iter().any(|w| !w.is_finite()) {
        bail!("pesos inválidos (NaN/inf)");
    }
    let config = &json["config"];
    let arch = json["architecture"].as_str().unwrap_or("");
    let mut model = match arch {
        "WaveNet" => Model::WaveNet(Box::new(WaveNet::new(config, &weights)?)),
        "LSTM" => Model::Lstm(Box::new(Lstm::new(config, &weights)?)),
        other => bail!("arquitetura NAM '{other}' ainda não suportada (suportadas: WaveNet, LSTM)"),
    };
    model.prewarm();
    Ok((model, info))
}

// ---------------------------------------------------------------------------
// Utilidades

struct Weights<'a> {
    data: &'a [f32],
    pos: usize,
}

impl<'a> Weights<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [f32]> {
        let end = self.pos + n;
        if end > self.data.len() {
            bail!("o modelo tem menos pesos do que a configuração exige");
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn next(&mut self) -> Result<f32> {
        Ok(self.take(1)?[0])
    }
}

#[derive(Clone, Copy, Debug)]
enum Activation {
    Tanh,
    FastTanh,
    HardTanh,
    Relu,
    LeakyRelu(f32),
    Sigmoid,
    Silu,
    Softsign,
    HardSwish,
}

impl Activation {
    fn from_json(v: &Value) -> Result<Self> {
        let (name, obj) = match v {
            Value::String(s) => (s.as_str(), None),
            Value::Object(o) => (o.get("type").and_then(Value::as_str).unwrap_or(""), Some(o)),
            _ => bail!("ativação inválida"),
        };
        Ok(match name {
            "Tanh" => Self::Tanh,
            "Fasttanh" => Self::FastTanh,
            "Hardtanh" => Self::HardTanh,
            "ReLU" => Self::Relu,
            "LeakyReLU" | "PReLU" => Self::LeakyRelu(
                obj.and_then(|o| o.get("negative_slope")).and_then(Value::as_f64).unwrap_or(0.01) as f32,
            ),
            "Sigmoid" => Self::Sigmoid,
            "SiLU" => Self::Silu,
            "Softsign" => Self::Softsign,
            "Hardswish" => Self::HardSwish,
            other => bail!("ativação '{other}' não suportada"),
        })
    }

    #[inline(always)]
    fn apply(self, x: f32) -> f32 {
        match self {
            Self::Tanh | Self::FastTanh => fast_tanh(x),
            Self::HardTanh => x.clamp(-1.0, 1.0),
            Self::Relu => x.max(0.0),
            Self::LeakyRelu(s) => if x > 0.0 { x } else { s * x },
            Self::Sigmoid => sigmoid(x),
            Self::Silu => x * sigmoid(x),
            Self::Softsign => x / (1.0 + x.abs()),
            Self::HardSwish => x * (x + 3.0).clamp(0.0, 6.0) / 6.0,
        }
    }
}

#[inline]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[inline]
fn fast_sigmoid(x: f32) -> f32 {
    0.5 * (fast_tanh(x * 0.5) + 1.0)
}

/// Mesma aproximação do NeuralAmpModelerCore.
#[inline(always)]
fn fast_tanh(x: f32) -> f32 {
    let ax = x.abs();
    let x2 = x * x;
    (x * (2.455_507_5 + 2.455_507_5 * ax + (0.893_229_85 + 0.821_226_67 * ax) * x2))
        / (2.445_066_3 + (2.445_066_3 + x2) * (x + 0.814_642_7 * x * ax).abs())
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Quatro acumuladores ajudam o compilador a vetorizar.
    let mut acc = [0f32; 4];
    let chunks = a.len() / 4 * 4;
    for i in (0..chunks).step_by(4) {
        acc[0] += a[i] * b[i];
        acc[1] += a[i + 1] * b[i + 1];
        acc[2] += a[i + 2] * b[i + 2];
        acc[3] += a[i + 3] * b[i + 3];
    }
    let mut s = acc[0] + acc[1] + acc[2] + acc[3];
    for i in chunks..a.len() {
        s += a[i] * b[i];
    }
    s
}

/// Convolução 1x1 (camada densa por amostra). Layout frame-major [t][c].
struct Dense {
    cin: usize,
    cout: usize,
    w: Vec<f32>, // [i][o]: cada entrada soma um vetor de saída (axpy, vetoriza bem)
    b: Vec<f32>,
}

/// y += a * x (vetores curtos; o compilador gera SIMD).
#[inline(always)]
fn axpy(y: &mut [f32], a: f32, x: &[f32]) {
    for (yi, xi) in y.iter_mut().zip(x) {
        *yi += a * xi;
    }
}

impl Dense {
    fn new(cin: usize, cout: usize, bias: bool, w: &mut Weights) -> Result<Self> {
        // No arquivo a ordem é [o][i]; transpõe para [i][o].
        let raw = w.take(cin * cout)?;
        let mut weights = vec![0f32; cin * cout];
        for o in 0..cout {
            for i in 0..cin {
                weights[i * cout + o] = raw[o * cin + i];
            }
        }
        let b = if bias { w.take(cout)?.to_vec() } else { vec![0.0; cout] };
        Ok(Self { cin, cout, w: weights, b })
    }

    #[inline(always)]
    fn process(&self, input: &[f32], out: &mut [f32], n: usize) {
        for t in 0..n {
            let x = &input[t * self.cin..(t + 1) * self.cin];
            let y = &mut out[t * self.cout..(t + 1) * self.cout];
            y.copy_from_slice(&self.b);
            for (i, &xi) in x.iter().enumerate() {
                axpy(y, xi, &self.w[i * self.cout..(i + 1) * self.cout]);
            }
        }
    }
}

/// Convolução causal dilatada com histórico circular compactado.
struct Conv {
    cin: usize,
    cout: usize,
    k: usize,
    d: usize,
    w: Vec<f32>, // [k][i][o]
    b: Vec<f32>,
    hist: Vec<f32>,
    cap: usize,
    pos: usize,
}

impl Conv {
    fn new(cin: usize, cout: usize, k: usize, d: usize, bias: bool, w: &mut Weights) -> Result<Self> {
        // Ordem do NAM: para cada saída, cada entrada, cada tap.
        let raw = w.take(cout * cin * k)?;
        let mut weights = vec![0f32; k * cout * cin];
        let mut idx = 0;
        for o in 0..cout {
            for i in 0..cin {
                for kk in 0..k {
                    weights[(kk * cin + i) * cout + o] = raw[idx];
                    idx += 1;
                }
            }
        }
        let b = if bias { w.take(cout)?.to_vec() } else { vec![0.0; cout] };
        let lookback = d * (k - 1);
        let cap = lookback + 8 * MAX_BLOCK;
        Ok(Self { cin, cout, k, d, w: weights, b, hist: vec![0.0; cap * cin], cap, pos: lookback })
    }

    fn lookback(&self) -> usize {
        self.d * (self.k - 1)
    }

    #[inline(always)]
    fn process(&mut self, input: &[f32], out: &mut [f32], n: usize) {
        let lb = self.lookback();
        if self.pos + n > self.cap {
            let keep = lb * self.cin;
            let from = (self.pos - lb) * self.cin;
            self.hist.copy_within(from..from + keep, 0);
            self.pos = lb;
        }
        let ci = self.cin;
        self.hist[self.pos * ci..(self.pos + n) * ci].copy_from_slice(&input[..n * ci]);
        for t in 0..n {
            let tt = self.pos + t;
            let y = &mut out[t * self.cout..(t + 1) * self.cout];
            y.copy_from_slice(&self.b);
            let co = self.cout;
            for kk in 0..self.k {
                let src = (tt - self.d * (self.k - 1 - kk)) * ci;
                let x = &self.hist[src..src + ci];
                let wk = &self.w[kk * ci * co..(kk + 1) * ci * co];
                for (i, &xi) in x.iter().enumerate() {
                    axpy(y, xi, &wk[i * co..(i + 1) * co]);
                }
            }
        }
        self.pos += n;
    }
}

// ---------------------------------------------------------------------------
// WaveNet

struct Layer {
    conv: Conv,
    mixin: Vec<f32>, // [cond][z]
    cond: usize,
    act: Activation,
    gate: Option<Activation>,
    bottleneck: usize,
    channels: usize,
    l1x1: Option<Dense>,
    z: Vec<f32>,
    a: Vec<f32>,
    r: Vec<f32>,
}

impl Layer {
    /// `input`: n×C, `cond`: n×cond, soma o ativado em `head` (n×B) e escreve n×C em `next`.
    #[inline(always)]
    fn process(&mut self, input: &[f32], cond: &[f32], head: &mut [f32], next: &mut [f32], n: usize) {
        let zc = self.conv.cout;
        self.conv.process(input, &mut self.z, n);
        let b = self.bottleneck;
        for t in 0..n {
            let z = &mut self.z[t * zc..(t + 1) * zc];
            let c = &cond[t * self.cond..(t + 1) * self.cond];
            for (ci, &cv) in c.iter().enumerate() {
                axpy(z, cv, &self.mixin[ci * zc..(ci + 1) * zc]);
            }
            let a = &mut self.a[t * b..(t + 1) * b];
            match self.gate {
                None => {
                    for (ao, zo) in a.iter_mut().zip(z.iter()) {
                        *ao = self.act.apply(*zo);
                    }
                }
                Some(g) => {
                    for o in 0..b {
                        a[o] = self.act.apply(z[o]) * g.apply(z[o + b]);
                    }
                }
            }
            for (h, ao) in head[t * b..(t + 1) * b].iter_mut().zip(a.iter()) {
                *h += *ao;
            }
        }
        let ch = self.channels;
        match &self.l1x1 {
            Some(dense) => {
                dense.process(&self.a, &mut self.r, n);
                for i in 0..n * ch {
                    next[i] = input[i] + self.r[i];
                }
            }
            None => next[..n * ch].copy_from_slice(&input[..n * ch]),
        }
    }
}

struct LayerArray {
    rechannel: Dense,
    layers: Vec<Layer>,
    head: Conv,
    bottleneck: usize,
    channels: usize,
    head_size: usize,
    head_in: Vec<f32>,
    head_out: Vec<f32>,
    buf_a: Vec<f32>,
    buf_b: Vec<f32>,
    /// Saída da última camada (entrada do próximo array).
    out: Vec<f32>,
}

pub struct WaveNet {
    arrays: Vec<LayerArray>,
    head_scale: f32,
    cond: Vec<f32>,
}

fn reject_if_present(cfg: &Value, keys: &[&str]) -> Result<()> {
    for k in keys {
        match &cfg[*k] {
            Value::Null => {}
            Value::Object(o) if o.get("active").and_then(Value::as_bool) == Some(false) => {}
            Value::Bool(false) => {}
            _ => bail!("recurso NAM '{k}' ainda não suportado"),
        }
    }
    Ok(())
}

impl WaveNet {
    fn new(config: &Value, weights: &[f32]) -> Result<Self> {
        reject_if_present(config, &["condition_dsp", "head"])?;
        let layers_cfg = config["layers"].as_array().ok_or_else(|| anyhow!("WaveNet sem 'layers'"))?;
        let mut w = Weights { data: weights, pos: 0 };
        let mut arrays = Vec::new();
        let get = |c: &Value, k: &str| -> Result<usize> {
            c[k].as_u64().map(|v| v as usize).ok_or_else(|| anyhow!("WaveNet: campo '{k}' ausente"))
        };
        for lc in layers_cfg {
            reject_if_present(
                lc,
                &[
                    "head1x1",
                    "conv_pre_film",
                    "conv_post_film",
                    "input_mixin_pre_film",
                    "input_mixin_post_film",
                    "activation_pre_film",
                    "activation_post_film",
                    "layer1x1_post_film",
                    "head1x1_post_film",
                ],
            )?;
            if lc["groups_input"].as_u64().unwrap_or(1) != 1 || lc["groups_input_mixin"].as_u64().unwrap_or(1) != 1 {
                bail!("convoluções em grupo ainda não suportadas");
            }
            let input_size = get(lc, "input_size")?;
            let cond = get(lc, "condition_size")?;
            let channels = get(lc, "channels")?;
            let bottleneck = lc["bottleneck"].as_u64().map(|v| v as usize).unwrap_or(channels);
            let (head_size, head_k, head_bias, head_d) = if lc["head"].is_object() {
                let h = &lc["head"];
                (
                    get(h, "out_channels")?,
                    get(h, "kernel_size")?,
                    h["bias"].as_bool().unwrap_or(false),
                    h["head_dilation"].as_u64().unwrap_or(1) as usize,
                )
            } else {
                (get(lc, "head_size")?, 1, lc["head_bias"].as_bool().unwrap_or(false), 1)
            };
            let dilations: Vec<usize> = lc["dilations"]
                .as_array()
                .ok_or_else(|| anyhow!("WaveNet sem 'dilations'"))?
                .iter()
                .map(|v| v.as_u64().map(|d| d as usize).ok_or_else(|| anyhow!("dilation inválida")))
                .collect::<Result<_>>()?;
            let n_layers = dilations.len();
            let kernels: Vec<usize> = match (&lc["kernel_size"], &lc["kernel_sizes"]) {
                (Value::Number(k), _) => vec![k.as_u64().unwrap_or(3) as usize; n_layers],
                (_, Value::Array(a)) => a.iter().map(|v| v.as_u64().unwrap_or(3) as usize).collect(),
                _ => bail!("WaveNet sem kernel_size"),
            };
            let per_layer = |v: &Value| -> Result<Vec<Activation>> {
                match v {
                    Value::Array(a) => a.iter().map(Activation::from_json).collect(),
                    other => Ok(vec![Activation::from_json(other)?; n_layers]),
                }
            };
            let acts = per_layer(&lc["activation"])?;
            let gated = match lc["gating_mode"].as_str() {
                Some("gated") => true,
                Some("none") | None => lc["gated"].as_bool().unwrap_or(false),
                Some(other) => bail!("gating '{other}' ainda não suportado"),
            };
            let secondary = match &lc["secondary_activation"] {
                Value::Null => Activation::Sigmoid,
                v => Activation::from_json(v)?,
            };
            let l1x1_active = lc["layer1x1"]["active"].as_bool().unwrap_or(true);
            if !l1x1_active && bottleneck != channels {
                bail!("layer1x1 inativo exige bottleneck = channels");
            }
            if kernels.len() != n_layers || acts.len() != n_layers {
                bail!("tamanhos de kernel/ativação não batem com as dilations");
            }

            let rechannel = Dense::new(input_size, channels, false, &mut w)?;
            let zc = if gated { 2 * bottleneck } else { bottleneck };
            let mut layers = Vec::with_capacity(n_layers);
            for i in 0..n_layers {
                let conv = Conv::new(channels, zc, kernels[i], dilations[i], true, &mut w)?;
                let raw = w.take(zc * cond)?;
                let mut mixin = vec![0f32; zc * cond];
                for o in 0..zc {
                    for c in 0..cond {
                        mixin[c * zc + o] = raw[o * cond + c];
                    }
                }
                let l1x1 = if l1x1_active { Some(Dense::new(bottleneck, channels, true, &mut w)?) } else { None };
                layers.push(Layer {
                    conv,
                    mixin,
                    cond,
                    act: acts[i],
                    gate: gated.then_some(secondary),
                    bottleneck,
                    channels,
                    l1x1,
                    z: vec![0.0; MAX_BLOCK * zc],
                    a: vec![0.0; MAX_BLOCK * bottleneck],
                    r: vec![0.0; MAX_BLOCK * channels],
                });
            }
            let head = Conv::new(bottleneck, head_size, head_k, head_d, head_bias, &mut w)?;
            arrays.push(LayerArray {
                rechannel,
                layers,
                head,
                bottleneck,
                channels,
                head_size,
                head_in: vec![0.0; MAX_BLOCK * bottleneck],
                head_out: vec![0.0; MAX_BLOCK * head_size],
                buf_a: vec![0.0; MAX_BLOCK * channels],
                buf_b: vec![0.0; MAX_BLOCK * channels],
                out: vec![0.0; MAX_BLOCK * channels],
            });
        }
        for i in 1..arrays.len() {
            if arrays[i - 1].head_size != arrays[i].bottleneck {
                bail!("head_size do array {} não bate com o próximo", i - 1);
            }
        }
        if arrays.last().map(|a| a.head_size) != Some(1) {
            bail!("só modelos com 1 canal de saída são suportados");
        }
        let head_scale = w.next()?;
        if w.pos != weights.len() {
            bail!("o modelo tem {} pesos a mais do que a configuração usa", weights.len() - w.pos);
        }
        let cond = layers_cfg[0]["condition_size"].as_u64().unwrap_or(1) as usize;
        if cond != 1 {
            bail!("condition_size diferente de 1 não suportado");
        }
        Ok(Self { arrays, head_scale, cond: vec![0.0; MAX_BLOCK] })
    }

    pub fn receptive_field(&self) -> usize {
        self.arrays
            .iter()
            .map(|a| a.layers.iter().map(|l| l.conv.lookback()).sum::<usize>() + a.head.lookback())
            .sum()
    }

    #[inline(always)]
    fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        let n = input.len();
        self.cond[..n].copy_from_slice(input);
        let cond = &self.cond[..n];
        for i in 0..self.arrays.len() {
            let (prev, rest) = self.arrays.split_at_mut(i);
            let arr = &mut rest[0];
            // Entrada do array: sinal (1º) ou saída de camadas do anterior.
            match prev.last() {
                None => {
                    arr.rechannel.process(cond, &mut arr.buf_a, n);
                    arr.head_in[..n * arr.bottleneck].fill(0.0);
                }
                Some(p) => {
                    arr.rechannel.process(&p.out, &mut arr.buf_a, n);
                    arr.head_in[..n * arr.bottleneck].copy_from_slice(&p.head_out[..n * p.head_size]);
                }
            }
            let ch = arr.channels;
            let n_layers = arr.layers.len();
            for (li, layer) in arr.layers.iter_mut().enumerate() {
                let (src, dst) = if li % 2 == 0 { (&arr.buf_a, &mut arr.buf_b) } else { (&arr.buf_b, &mut arr.buf_a) };
                layer.process(src, cond, &mut arr.head_in, dst, n);
            }
            let last = if n_layers % 2 == 1 { &arr.buf_b } else { &arr.buf_a };
            arr.out[..n * ch].copy_from_slice(&last[..n * ch]);
            arr.head.process(&arr.head_in, &mut arr.head_out, n);
        }
        let last = self.arrays.last().expect("ao menos um array");
        for (o, h) in output.iter_mut().zip(&last.head_out[..n]) {
            *o = h * self.head_scale;
        }
    }
}

// ---------------------------------------------------------------------------
// LSTM

struct LstmCell {
    input: usize,
    hidden: usize,
    w: Vec<f32>, // [4H][I+H]
    b: Vec<f32>,
    xh: Vec<f32>,
    c: Vec<f32>,
    ifgo: Vec<f32>,
}

pub struct Lstm {
    cells: Vec<LstmCell>,
    head_w: Vec<f32>,
    head_b: f32,
}

impl Lstm {
    fn new(config: &Value, weights: &[f32]) -> Result<Self> {
        let get = |k: &str| config[k].as_u64().map(|v| v as usize).ok_or_else(|| anyhow!("LSTM: '{k}' ausente"));
        let (layers, input, hidden) = (get("num_layers")?, get("input_size")?, get("hidden_size")?);
        if config["out_channels"].as_u64().unwrap_or(1) != 1 || config["in_channels"].as_u64().unwrap_or(1) != 1 {
            bail!("LSTM multicanal não suportado");
        }
        let mut w = Weights { data: weights, pos: 0 };
        let mut cells = Vec::new();
        for l in 0..layers {
            let ins = if l == 0 { input } else { hidden };
            let mat = w.take(4 * hidden * (ins + hidden))?.to_vec();
            let b = w.take(4 * hidden)?.to_vec();
            let mut xh = vec![0f32; ins + hidden];
            xh[ins..].copy_from_slice(w.take(hidden)?);
            let c = w.take(hidden)?.to_vec();
            cells.push(LstmCell { input: ins, hidden, w: mat, b, xh, c, ifgo: vec![0.0; 4 * hidden] });
        }
        let head_w = w.take(hidden)?.to_vec();
        let head_b = w.next()?;
        if w.pos != weights.len() {
            bail!("LSTM com pesos sobrando");
        }
        Ok(Self { cells, head_w, head_b })
    }

    #[inline(always)]
    fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        for (x, y) in input.iter().zip(output.iter_mut()) {
            let mut prev: Option<usize> = None;
            for li in 0..self.cells.len() {
                let (before, rest) = self.cells.split_at_mut(li);
                let cell = &mut rest[0];
                match prev {
                    None => cell.xh[0] = *x,
                    Some(p) => {
                        let src = &before[p];
                        let h = &src.xh[src.input..];
                        cell.xh[..cell.input].copy_from_slice(h);
                    }
                }
                let cols = cell.input + cell.hidden;
                for r in 0..4 * cell.hidden {
                    cell.ifgo[r] = cell.b[r] + dot(&cell.w[r * cols..(r + 1) * cols], &cell.xh);
                }
                let hs = cell.hidden;
                for i in 0..hs {
                    cell.c[i] = fast_sigmoid(cell.ifgo[i + hs]) * cell.c[i]
                        + fast_sigmoid(cell.ifgo[i]) * fast_tanh(cell.ifgo[i + 2 * hs]);
                }
                for i in 0..hs {
                    cell.xh[cell.input + i] = fast_sigmoid(cell.ifgo[i + 3 * hs]) * fast_tanh(cell.c[i]);
                }
                prev = Some(li);
            }
            let last = self.cells.last().expect("ao menos uma camada");
            *y = self.head_b + dot(&self.head_w, &last.xh[last.input..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compara com saídas geradas pelo NeuralAmpModelerCore (C++, com
    /// `enable_fast_tanh()` como no plugin) para o mesmo sinal: 0.3·sen(110 Hz) + 0.1·sen(1234 Hz), blocos de 256.
    fn check(name: &str) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/nam");
        let data = std::fs::read(dir.join(format!("{name}.nam"))).unwrap();
        let reference: Vec<f32> = std::fs::read_to_string(dir.join(format!("{name}.ref.txt")))
            .unwrap()
            .lines()
            .map(|l| l.parse().unwrap())
            .collect();
        let (mut model, _) = load(&data).unwrap();
        let input: Vec<f32> = (0..reference.len())
            .map(|i| {
                let t = i as f64 / 48000.0;
                (0.3 * (2.0 * std::f64::consts::PI * 110.0 * t).sin() + 0.1 * (2.0 * std::f64::consts::PI * 1234.0 * t).sin()) as f32
            })
            .collect();
        let mut out = vec![0f32; input.len()];
        for (i, o) in input.chunks(256).zip(out.chunks_mut(256)) {
            model.process(i, o);
        }
        let max_err = out.iter().zip(&reference).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        let peak = reference.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!(max_err < 1e-4 * peak.max(1.0), "{name}: erro máximo {max_err} (pico {peak})");
    }

    #[test]
    fn wavenet_small_matches_reference() {
        check("wavenet");
    }

    #[test]
    fn wavenet_standard_matches_reference() {
        check("wavenet_a1_standard");
    }

    #[test]
    fn lstm_matches_reference() {
        check("lstm");
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore]
    fn standard_wavenet_realtime_factor() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/nam");
        let (mut model, _) = super::load(&std::fs::read(dir.join("wavenet_a1_standard.nam")).unwrap()).unwrap();
        let input: Vec<f32> = (0..48000 * 5).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let mut out = vec![0f32; input.len()];
        let t = std::time::Instant::now();
        for (i, o) in input.chunks(128).zip(out.chunks_mut(128)) {
            model.process(i, o);
        }
        let secs = t.elapsed().as_secs_f64();
        println!("5 s de áudio em {:.3} s -> {:.1}% de um núcleo", secs, secs / 5.0 * 100.0);
    }
}
