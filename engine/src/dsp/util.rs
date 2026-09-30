//! Blocos básicos de DSP reutilizados pelos efeitos.

use std::f32::consts::PI;

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn lin_to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-9).log10()
}

/// Mapeia um knob 0..10 para um intervalo linear.
#[inline]
pub fn knob(v: f32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * (v / 10.0).clamp(0.0, 1.0)
}

/// Mapeia um knob 0..10 para um intervalo exponencial (frequências).
#[inline]
pub fn knob_exp(v: f32, lo: f32, hi: f32) -> f32 {
    lo * (hi / lo).powf((v / 10.0).clamp(0.0, 1.0))
}

/// Coeficiente de um filtro de um polo para uma constante de tempo em ms.
#[inline]
pub fn time_coef(ms: f32, sample_rate: f32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sample_rate)).exp()
    }
}

/// Suaviza mudanças de parâmetro para evitar "zipper noise".
#[derive(Clone, Copy)]
pub struct Smoother {
    current: f32,
    target: f32,
    coef: f32,
}

impl Smoother {
    pub fn new(value: f32, ms: f32, sample_rate: f32) -> Self {
        Self { current: value, target: value, coef: time_coef(ms, sample_rate) }
    }

    #[inline]
    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    #[inline]
    pub fn snap(&mut self) {
        self.current = self.target;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        self.current = self.target + (self.current - self.target) * self.coef;
        self.current
    }

    #[inline]
    pub fn is_settled(&self) -> bool {
        (self.current - self.target).abs() < 1e-5
    }

    #[inline]
    pub fn target(&self) -> f32 {
        self.target
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum FilterKind {
    LowPass,
    HighPass,
    BandPass,
    Peak,
    LowShelf,
    HighShelf,
    Notch,
}

/// Biquad (RBJ Audio EQ Cookbook) em forma direta transposta II.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn new(kind: FilterKind, sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let mut b = Self::default();
        b.set(kind, sample_rate, freq, q, gain_db);
        b
    }

    pub fn set(&mut self, kind: FilterKind, sample_rate: f32, freq: f32, q: f32, gain_db: f32) {
        let freq = freq.clamp(10.0, sample_rate * 0.49);
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(0.05));
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Peak => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
            FilterKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            FilterKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Filtro de um polo (6 dB/oitava), passa-baixa ou passa-alta.
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    a: f32,
    z: f32,
    highpass: bool,
}

impl OnePole {
    pub fn lowpass(sample_rate: f32, freq: f32) -> Self {
        let mut f = Self { highpass: false, ..Default::default() };
        f.set_freq(sample_rate, freq);
        f
    }

    pub fn highpass(sample_rate: f32, freq: f32) -> Self {
        let mut f = Self { highpass: true, ..Default::default() };
        f.set_freq(sample_rate, freq);
        f
    }

    pub fn set_freq(&mut self, sample_rate: f32, freq: f32) {
        let freq = freq.clamp(1.0, sample_rate * 0.49);
        self.a = (-2.0 * PI * freq / sample_rate).exp();
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.z = x + (self.z - x) * self.a;
        if self.highpass { x - self.z } else { self.z }
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
    }
}

/// Oversampling 4x com filtros Butterworth de 6ª ordem. Usado em volta de
/// não-linearidades (drive, amp) para reduzir aliasing, que é o que deixa
/// distorção digital com aquele som "áspero".
pub struct Oversampler4 {
    up: [Biquad; 3],
    down: [Biquad; 3],
}

impl Oversampler4 {
    pub const FACTOR: usize = 4;

    pub fn new(sample_rate: f32) -> Self {
        let os_rate = sample_rate * Self::FACTOR as f32;
        let cutoff = sample_rate * 0.42;
        let qs = [0.517_638_1, 0.707_106_8, 1.931_851_6];
        let make = || qs.map(|q| Biquad::new(FilterKind::LowPass, os_rate, cutoff, q, 0.0));
        Self { up: make(), down: make() }
    }

    /// Processa uma amostra passando `f` na taxa 4x.
    #[inline]
    pub fn process(&mut self, x: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        let mut out = 0.0;
        for k in 0..Self::FACTOR {
            let mut s = if k == 0 { x * Self::FACTOR as f32 } else { 0.0 };
            for b in &mut self.up {
                s = b.process(s);
            }
            let mut y = f(s);
            for b in &mut self.down {
                y = b.process(y);
            }
            out = y;
        }
        out
    }

    pub fn reset(&mut self) {
        self.up.iter_mut().chain(self.down.iter_mut()).for_each(Biquad::reset);
    }
}

/// Linha de atraso circular com leitura fracionária (Hermite cúbica).
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4) + 4], pos: 0 }
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.pos = if self.pos + 1 >= self.buf.len() { 0 } else { self.pos + 1 };
        self.buf[self.pos] = x;
    }

    #[inline]
    fn at(&self, delay: usize) -> f32 {
        let len = self.buf.len();
        self.buf[(self.pos + len - (delay % len)) % len]
    }

    /// Lê com atraso fracionário em amostras (>= 1).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let delay = delay.clamp(1.0, (self.buf.len() - 3) as f32);
        let i = delay.floor() as usize;
        let t = delay - i as f32;
        let (xm1, x0, x1, x2) = (self.at(i - 1), self.at(i), self.at(i + 1), self.at(i + 2));
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Saturação assimétrica estilo válvula: gera harmônicos pares e não
/// desloca DC quando a entrada é zero.
#[inline]
pub fn tube(x: f32, bias: f32) -> f32 {
    (x + bias).tanh() - bias.tanh()
}

/// Clipping suave de diodo.
#[inline]
pub fn diode(x: f32) -> f32 {
    x / (1.0 + x.abs())
}

/// Soft clipper de segurança para a saída (protege os ouvidos no fone).
#[inline]
pub fn safety_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = (a - KNEE) / (1.0 - KNEE);
        x.signum() * (KNEE + (1.0 - KNEE) * over.tanh())
    }
}

/// Liga flush-to-zero/denormals-are-zero na thread atual (evita picos de CPU
/// quando o sinal decai para valores minúsculos em filtros e reverbs).
pub fn enable_flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)]
    unsafe {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
}
