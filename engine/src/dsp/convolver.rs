//! Convolução de latência zero para IRs.
//!
//! Os primeiros `BLOCK` coeficientes são aplicados direto no tempo (FIR),
//! amostra por amostra. O restante usa convolução particionada uniforme
//! (overlap-save no domínio da frequência). Como a cauda começa `BLOCK`
//! amostras depois, o resultado dela fica pronto um bloco antes de ser
//! necessário: nenhuma latência é adicionada.

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

pub const BLOCK: usize = 128;
const FFT: usize = BLOCK * 2;
const BINS: usize = FFT / 2 + 1;

/// Espectros das partições da IR (compartilháveis entre instâncias L/R).
pub struct IrSpectra {
    head: Vec<f32>,
    parts: Vec<Vec<Complex32>>,
}

impl IrSpectra {
    pub fn new(ir: &[f32]) -> Arc<Self> {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT);
        let head = ir.iter().copied().take(BLOCK).chain(std::iter::repeat(0.0)).take(BLOCK).collect();
        let tail = if ir.len() > BLOCK { &ir[BLOCK..] } else { &[][..] };
        let parts = tail
            .chunks(BLOCK)
            .map(|chunk| {
                let mut buf = vec![0f32; FFT];
                buf[..chunk.len()].copy_from_slice(chunk);
                let mut spec = fft.make_output_vec();
                fft.process(&mut buf, &mut spec).expect("fft");
                // Normaliza aqui para a IFFT sair na escala certa.
                spec.iter_mut().for_each(|c| *c /= FFT as f32);
                spec
            })
            .collect();
        Arc::new(Self { head, parts })
    }
}

pub struct Convolver {
    ir: Arc<IrSpectra>,
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    /// Histórico circular para o FIR da cabeça.
    history: Vec<f32>,
    hpos: usize,
    /// Bloco de entrada anterior + atual (janela do overlap-save).
    window: Vec<f32>,
    fill: usize,
    /// Linha de atraso de espectros de entrada (um por partição).
    fdl: Vec<Vec<Complex32>>,
    fdl_pos: usize,
    acc: Vec<Complex32>,
    time: Vec<f32>,
    scratch_fwd: Vec<Complex32>,
    scratch_inv: Vec<Complex32>,
    /// Saída da cauda para o bloco corrente.
    tail_out: Vec<f32>,
}

impl Convolver {
    pub fn new(ir: Arc<IrSpectra>) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT);
        let ifft = planner.plan_fft_inverse(FFT);
        let parts = ir.parts.len().max(1);
        Self {
            scratch_fwd: fft.make_scratch_vec(),
            scratch_inv: ifft.make_scratch_vec(),
            fft,
            ifft,
            history: vec![0.0; BLOCK],
            hpos: 0,
            window: vec![0.0; FFT],
            fill: 0,
            fdl: vec![vec![Complex32::default(); BINS]; parts],
            fdl_pos: 0,
            acc: vec![Complex32::default(); BINS],
            time: vec![0.0; FFT],
            tail_out: vec![0.0; BLOCK],
            ir,
        }
    }

    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        // Cabeça: FIR direto.
        self.history[self.hpos] = x;
        let mut y = 0.0;
        let h = &self.ir.head;
        let mut idx = self.hpos;
        for &coef in h.iter() {
            y += coef * self.history[idx];
            idx = if idx == 0 { BLOCK - 1 } else { idx - 1 };
        }
        self.hpos = (self.hpos + 1) % BLOCK;

        y += self.tail_out[self.fill];
        self.window[BLOCK + self.fill] = x;
        self.fill += 1;
        if self.fill == BLOCK {
            self.fill = 0;
            self.compute_tail();
        }
        y
    }

    fn compute_tail(&mut self) {
        if self.ir.parts.is_empty() {
            self.window.copy_within(BLOCK.., 0);
            return;
        }
        let n = self.fdl.len();
        // Espectro da janela [bloco anterior | bloco atual].
        self.time.copy_from_slice(&self.window);
        let slot = &mut self.fdl[self.fdl_pos];
        let _ = self.fft.process_with_scratch(&mut self.time, slot, &mut self.scratch_fwd);
        self.window.copy_within(BLOCK.., 0);

        // Soma das partições: X(atual) * H0 + X(anterior) * H1 + ...
        self.acc.iter_mut().for_each(|c| *c = Complex32::default());
        for (k, part) in self.ir.parts.iter().enumerate() {
            let x = &self.fdl[(self.fdl_pos + n - k) % n];
            for ((a, xi), hi) in self.acc.iter_mut().zip(x).zip(part) {
                *a += xi * hi;
            }
        }
        self.fdl_pos = (self.fdl_pos + 1) % n;

        // Bins DC e Nyquist precisam ter parte imaginária zero para a IFFT real.
        self.acc[0].im = 0.0;
        self.acc[BINS - 1].im = 0.0;
        let _ = self.ifft.process_with_scratch(&mut self.acc, &mut self.time, &mut self.scratch_inv);
        // Overlap-save: a segunda metade é a saída válida.
        self.tail_out.copy_from_slice(&self.time[BLOCK..]);
    }

    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.window.fill(0.0);
        self.tail_out.fill(0.0);
        self.fdl.iter_mut().for_each(|s| s.fill(Complex32::default()));
        self.fill = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_direct_convolution() {
        let ir: Vec<f32> = (0..1000).map(|i| ((i * 37 % 101) as f32 / 50.0 - 1.0) * (-(i as f32) / 300.0).exp()).collect();
        let x: Vec<f32> = (0..3000).map(|i| (i * 13 % 29) as f32 / 14.0 - 1.0).collect();
        let mut conv = Convolver::new(IrSpectra::new(&ir));
        let got: Vec<f32> = x.iter().map(|&s| conv.process_sample(s)).collect();
        for n in 0..x.len() {
            let expected: f32 = (0..ir.len().min(n + 1)).map(|k| ir[k] * x[n - k]).sum();
            assert!((got[n] - expected).abs() < 1e-3, "n={n}: {} vs {expected}", got[n]);
        }
    }
}
