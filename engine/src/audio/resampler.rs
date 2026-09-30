//! Ponte entre o dispositivo de entrada (cabo USB) e o de saída (fone).
//!
//! São dois relógios de hardware diferentes: mesmo que ambos digam 48 kHz,
//! um sempre corre um pouquinho mais rápido. Sem compensação o buffer
//! esvazia (estalos) ou enche (latência crescendo). Aqui um resampler
//! adaptativo ajusta a razão em até ±0,5% para manter o buffer no alvo, e
//! também converte taxas diferentes (ex.: cabo em 44,1 kHz, fone em 48 kHz).

use rtrb::Consumer;

pub struct AdaptiveResampler {
    base_ratio: f64,
    pos: f64,
    prev: [f32; 2],
    cur: [f32; 2],
    target: f64,
    fill_avg: f64,
    primed: bool,
    pub underruns: u64,
}

impl AdaptiveResampler {
    /// `target_fill` em frames na taxa de entrada.
    pub fn new(in_rate: u32, out_rate: u32, target_fill: usize) -> Self {
        Self {
            base_ratio: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            prev: [0.0; 2],
            cur: [0.0; 2],
            target: target_fill.max(16) as f64,
            fill_avg: target_fill as f64,
            primed: false,
            underruns: 0,
        }
    }

    /// Preenche `out` (estéreo intercalado) lendo frames estéreo de `rx`.
    pub fn render(&mut self, rx: &mut Consumer<f32>, out: &mut [f32]) {
        let fill = (rx.slots() / 2) as f64;
        if !self.primed {
            if fill >= self.target {
                self.primed = true;
                self.fill_avg = fill;
            } else {
                out.fill(0.0);
                return;
            }
        }

        // Muito atrasado (ex.: a thread de saída travou): descarta o excesso
        // para a latência voltar ao alvo em vez de ficar alta para sempre.
        if fill > self.target * 4.0 + 256.0 {
            let skip = (fill - self.target) as usize * 2;
            if let Ok(chunk) = rx.read_chunk(skip) {
                chunk.commit_all();
            }
            self.fill_avg = self.target;
        }

        self.fill_avg += (fill - self.fill_avg) * 0.02;
        let err = (self.fill_avg - self.target) / self.target;
        let ratio = self.base_ratio * (1.0 + (err * 0.004).clamp(-0.005, 0.005));

        for frame in out.chunks_exact_mut(2) {
            if !self.primed {
                frame.fill(0.0);
                continue;
            }
            self.pos += ratio;
            while self.pos >= 1.0 {
                self.pos -= 1.0;
                self.prev = self.cur;
                match (rx.pop(), rx.pop()) {
                    (Ok(l), Ok(r)) => self.cur = [l, r],
                    _ => {
                        // Buffer vazio: silêncio e re-preenche antes de voltar.
                        self.underruns += 1;
                        self.primed = false;
                        self.cur = [0.0; 2];
                        self.prev = [0.0; 2];
                        self.pos = 0.0;
                        break;
                    }
                }
            }
            if !self.primed {
                frame.fill(0.0);
                continue;
            }
            let t = self.pos as f32;
            frame[0] = self.prev[0] + (self.cur[0] - self.prev[0]) * t;
            frame[1] = self.prev[1] + (self.cur[1] - self.prev[1]) * t;
        }
    }
}
