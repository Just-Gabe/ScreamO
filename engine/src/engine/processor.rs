//! Processamento em tempo real. Roda na thread de captura: recebe o sinal
//! mono da guitarra, passa pela cadeia de efeitos e empurra estéreo para o
//! buffer da saída.
//!
//! A comunicação com a UI é 100% lock-free: mudanças chegam por uma fila de
//! [`Command`] e objetos removidos voltam por uma fila de "lixo" para serem
//! desalocados fora da thread de áudio.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use rtrb::{Consumer, Producer};

use crate::dsp::util::{Smoother, safety_clip};
use crate::dsp::{Effect, MAX_BLOCK};

pub const MAX_SLOTS: usize = 64;

pub struct Slot {
    pub id: u64,
    pub effect: Box<dyn Effect>,
    /// 1.0 = ativo, 0.0 = bypass. Suavizado para a troca não estalar.
    mix: Smoother,
}

impl Slot {
    pub fn new(id: u64, effect: Box<dyn Effect>, bypass: bool, sample_rate: f32) -> Self {
        let v = if bypass { 0.0 } else { 1.0 };
        Self { id, effect, mix: Smoother::new(v, 8.0, sample_rate) }
    }
}

pub enum Command {
    SetParam { slot: u64, index: usize, value: f32 },
    SetBypass { slot: u64, bypass: bool },
    Insert { index: usize, slot: Slot },
    Remove { slot: u64 },
    Move { slot: u64, to: usize },
    ReplaceChain(Vec<Slot>),
    SetInputGain(f32),
    SetOutputGain(f32),
    SetMute(bool),
}

/// Objetos removidos da cadeia; só existem para serem desalocados fora da
/// thread de áudio.
#[allow(dead_code)]
pub enum Garbage {
    Slot(Slot),
    Chain(Vec<Slot>),
}

/// Medidores compartilhados com a UI (f32 guardado como bits em atômicos).
#[derive(Default)]
pub struct Meters {
    input_peak: AtomicU32,
    output_peak: AtomicU32,
    dsp_load: AtomicU32,
    pub underruns: AtomicU64,
    pub overflows: AtomicU64,
    pub tuner_enabled: AtomicBool,
}

impl Meters {
    fn raise(a: &AtomicU32, v: f32) {
        let cur = f32::from_bits(a.load(Ordering::Relaxed));
        if v > cur {
            a.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    /// Lê e zera os picos desde a última leitura.
    pub fn take_peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.input_peak.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.output_peak.swap(0, Ordering::Relaxed)),
        )
    }

    pub fn dsp_load(&self) -> f32 {
        f32::from_bits(self.dsp_load.load(Ordering::Relaxed))
    }
}

pub struct Processor {
    sample_rate: f32,
    slots: Vec<Slot>,
    commands: Consumer<Command>,
    garbage: Producer<Garbage>,
    output: Producer<f32>,
    tuner: Producer<f32>,
    meters: Arc<Meters>,
    input_gain: Smoother,
    output_gain: Smoother,
    mute: Smoother,
    l: Vec<f32>,
    r: Vec<f32>,
    dry_l: Vec<f32>,
    dry_r: Vec<f32>,
    load_avg: f32,
}

pub struct ProcessorInit {
    pub sample_rate: f32,
    pub slots: Vec<Slot>,
    pub commands: Consumer<Command>,
    pub garbage: Producer<Garbage>,
    pub output: Producer<f32>,
    pub tuner: Producer<f32>,
    pub meters: Arc<Meters>,
    pub input_gain: f32,
    pub output_gain: f32,
    pub muted: bool,
}

impl Processor {
    pub fn new(init: ProcessorInit) -> Self {
        let sr = init.sample_rate;
        let mut slots = init.slots;
        slots.reserve(MAX_SLOTS.saturating_sub(slots.len()));
        Self {
            sample_rate: sr,
            slots,
            commands: init.commands,
            garbage: init.garbage,
            output: init.output,
            tuner: init.tuner,
            meters: init.meters,
            input_gain: Smoother::new(init.input_gain, 20.0, sr),
            output_gain: Smoother::new(init.output_gain, 20.0, sr),
            mute: Smoother::new(if init.muted { 0.0 } else { 1.0 }, 15.0, sr),
            l: vec![0.0; MAX_BLOCK],
            r: vec![0.0; MAX_BLOCK],
            dry_l: vec![0.0; MAX_BLOCK],
            dry_r: vec![0.0; MAX_BLOCK],
            load_avg: 0.0,
        }
    }

    fn dispose(&mut self, g: Garbage) {
        // Se a fila de lixo estiver cheia, vaza em vez de desalocar aqui.
        if let Err(rtrb::PushError::Full(g)) = self.garbage.push(g) {
            std::mem::forget(g);
        }
    }

    fn handle_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::SetParam { slot, index, value } => {
                    if let Some(s) = self.slots.iter_mut().find(|s| s.id == slot) {
                        s.effect.set_param(index, value);
                    }
                }
                Command::SetBypass { slot, bypass } => {
                    if let Some(s) = self.slots.iter_mut().find(|s| s.id == slot) {
                        s.mix.set(if bypass { 0.0 } else { 1.0 });
                    }
                }
                Command::Insert { index, slot } => {
                    if self.slots.len() < self.slots.capacity() {
                        let i = index.min(self.slots.len());
                        self.slots.insert(i, slot);
                    } else {
                        self.dispose(Garbage::Slot(slot));
                    }
                }
                Command::Remove { slot } => {
                    if let Some(i) = self.slots.iter().position(|s| s.id == slot) {
                        let s = self.slots.remove(i);
                        self.dispose(Garbage::Slot(s));
                    }
                }
                Command::Move { slot, to } => {
                    if let Some(i) = self.slots.iter().position(|s| s.id == slot) {
                        let s = self.slots.remove(i);
                        self.slots.insert(to.min(self.slots.len()), s);
                    }
                }
                Command::ReplaceChain(mut new) => {
                    std::mem::swap(&mut self.slots, &mut new);
                    self.dispose(Garbage::Chain(new));
                }
                Command::SetInputGain(g) => self.input_gain.set(g),
                Command::SetOutputGain(g) => self.output_gain.set(g),
                Command::SetMute(m) => self.mute.set(if m { 0.0 } else { 1.0 }),
            }
        }
    }

    /// Processa um pacote vindo do dispositivo de entrada.
    pub fn process(&mut self, input: &[f32]) {
        let started = Instant::now();
        self.handle_commands();
        for chunk in input.chunks(MAX_BLOCK) {
            self.process_block(chunk);
        }
        // Carga de DSP = tempo gasto / duração do áudio processado.
        if !input.is_empty() {
            let budget = input.len() as f32 / self.sample_rate;
            let load = started.elapsed().as_secs_f32() / budget;
            self.load_avg += (load - self.load_avg) * 0.05;
            self.meters.dsp_load.store(self.load_avg.to_bits(), Ordering::Relaxed);
        }
    }

    fn process_block(&mut self, input: &[f32]) {
        let n = input.len();
        let (l, r) = (&mut self.l[..n], &mut self.r[..n]);

        let mut peak_in = 0f32;
        let tuner_on = self.meters.tuner_enabled.load(Ordering::Relaxed);
        for (i, &x) in input.iter().enumerate() {
            peak_in = peak_in.max(x.abs());
            l[i] = x * self.input_gain.next();
        }
        if tuner_on {
            if let Ok(mut chunk) = self.tuner.write_chunk(n.min(self.tuner.slots())) {
                let (a, b) = chunk.as_mut_slices();
                let len_a = a.len();
                a.copy_from_slice(&l[..len_a]);
                b.copy_from_slice(&l[len_a..len_a + b.len()]);
                chunk.commit_all();
            }
        }
        r.copy_from_slice(l);
        Meters::raise(&self.meters.input_peak, peak_in);

        let mut stereo = false;
        for slot in self.slots.iter_mut() {
            let active = slot.mix.target() > 0.5;
            if slot.mix.is_settled() {
                if active {
                    stereo = slot.effect.process(l, r, stereo);
                }
                continue;
            }
            // Em transição de bypass: crossfade entre seco e processado.
            let (dl, dr) = (&mut self.dry_l[..n], &mut self.dry_r[..n]);
            dl.copy_from_slice(l);
            dr.copy_from_slice(r);
            let out_stereo = slot.effect.process(l, r, stereo);
            for i in 0..n {
                let m = slot.mix.next();
                l[i] = dl[i] + (l[i] - dl[i]) * m;
                r[i] = dr[i] + (r[i] - dr[i]) * m;
            }
            stereo |= out_stereo;
            if !active && slot.mix.is_settled() {
                slot.effect.reset();
            }
        }

        let mut peak_out = 0f32;
        for i in 0..n {
            let g = self.output_gain.next() * self.mute.next();
            l[i] = safety_clip(l[i] * g);
            r[i] = safety_clip(r[i] * g);
            peak_out = peak_out.max(l[i].abs()).max(r[i].abs());
        }
        Meters::raise(&self.meters.output_peak, peak_out);

        match self.output.write_chunk(n * 2) {
            Ok(mut chunk) => {
                let (a, b) = chunk.as_mut_slices();
                for (i, v) in a.iter_mut().chain(b.iter_mut()).enumerate() {
                    *v = if i % 2 == 0 { l[i / 2] } else { r[i / 2] };
                }
                chunk.commit_all();
            }
            Err(_) => {
                self.meters.overflows.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
