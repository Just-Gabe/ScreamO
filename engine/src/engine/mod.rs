//! Lado de controle do motor: guarda o estado (cadeia, configurações),
//! gerencia a sessão de áudio e traduz ações da UI em comandos lock-free
//! para a thread de áudio.

pub mod presets;
pub mod processor;
pub mod tuner;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow, bail};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::audio::resampler::AdaptiveResampler;
use crate::audio::wasapi::{self, DeviceInfo, Flow, InputChannel, Session, SessionConfig};
use crate::dsp::{self, util::db_to_lin};
use presets::{Preset, PresetSlot, PresetStore};
use processor::{Command, Garbage, Meters, Processor, ProcessorInit, Slot};
use tuner::Tuner;

/// Palavras que indicam um cabo/interface de guitarra no nome do device.
const GUITAR_HINTS: &[&str] = &["guitar", "guitarra", "instrument", "link", "ucg", "irig", "hi-z", "usb audio codec", "line 6", "rocksmith"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub exclusive: bool,
    pub input_channel: InputChannel,
    /// Folga extra no buffer entre entrada e saída. Aumente se ouvir estalos.
    pub safety_ms: f32,
    pub input_gain_db: f32,
    pub output_gain_db: f32,
    pub muted: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            exclusive: false,
            input_channel: InputChannel::Auto,
            safety_ms: 2.0,
            input_gain_db: 0.0,
            output_gain_db: -6.0,
            muted: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotState {
    pub slot_id: u64,
    pub effect_id: String,
    pub params: Vec<f32>,
    pub bypass: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Persisted {
    settings: Settings,
    chain: Option<Vec<PresetSlot>>,
    current_preset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub running: bool,
    pub error: Option<String>,
    pub input: Option<wasapi::StreamInfo>,
    pub output: Option<wasapi::StreamInfo>,
    pub latency_ms: f32,
}

/// Ações que a UI pode pedir (JSON: `{"type": "addEffect", ...}`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Action {
    GetState,
    RefreshDevices,
    SetDevices { input_device: Option<String>, output_device: Option<String> },
    SetAudioOptions { exclusive: Option<bool>, input_channel: Option<InputChannel>, safety_ms: Option<f32> },
    RestartAudio,
    SetInputGain { db: f32 },
    SetOutputGain { db: f32 },
    SetMute { muted: bool },
    SetTuner { enabled: bool },
    AddEffect { effect_id: String, index: Option<usize> },
    RemoveEffect { slot_id: u64 },
    MoveEffect { slot_id: u64, to: usize },
    SetParam { slot_id: u64, index: usize, value: f32 },
    SetBypass { slot_id: u64, bypass: bool },
    LoadPreset { name: String },
    SavePreset { name: String },
    DeletePreset { name: String },
}

/// O que mudou após uma ação, para decidir o que transmitir às UIs.
pub enum Outcome {
    State,
    Param { slot_id: u64, index: usize, value: f32 },
}

struct Running {
    session: Session,
    commands: Producer<Command>,
    garbage: Consumer<Garbage>,
    _tuner: Tuner,
    tuner_reading: Arc<parking_lot::Mutex<tuner::TunerReading>>,
    sample_rate: f32,
}

pub struct Controller {
    settings: Settings,
    chain: Vec<SlotState>,
    next_id: u64,
    current_preset: Option<String>,
    status: AudioStatus,
    running: Option<Running>,
    meters: Arc<Meters>,
    inputs: Vec<DeviceInfo>,
    outputs: Vec<DeviceInfo>,
    store: PresetStore,
    config_path: PathBuf,
    dirty: bool,
    /// Mudo automático quando nenhuma entrada de guitarra foi escolhida nem
    /// detectada: evita microfonia do microfone padrão nos alto-falantes.
    safety_mute: bool,
}

impl Controller {
    pub fn new(data_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&data_dir);
        let config_path = data_dir.join("config.json");
        let persisted: Persisted = presets::read_json(&config_path).unwrap_or_default();
        let mut c = Self {
            settings: persisted.settings,
            chain: Vec::new(),
            next_id: 1,
            current_preset: persisted.current_preset,
            status: AudioStatus::default(),
            running: None,
            meters: Arc::new(Meters::default()),
            inputs: Vec::new(),
            outputs: Vec::new(),
            store: PresetStore::new(data_dir.join("presets")),
            config_path,
            dirty: false,
            safety_mute: false,
        };
        let chain = persisted.chain.unwrap_or_else(presets::default_chain);
        c.chain = c.chain_from_preset(&chain);
        if c.current_preset.is_none() && c.chain.len() == chain.len() {
            c.current_preset = Some("Crunch Clássico".into());
        }
        c.refresh_devices();
        c
    }

    fn save_config(&self) {
        let persisted = Persisted {
            settings: self.settings.clone(),
            chain: Some(self.chain_to_preset()),
            current_preset: self.current_preset.clone(),
        };
        if let Err(e) = presets::write_json(&self.config_path, &persisted) {
            eprintln!("[screamo] erro salvando config: {e:#}");
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn chain_from_preset(&mut self, slots: &[PresetSlot]) -> Vec<SlotState> {
        slots
            .iter()
            .filter_map(|s| {
                let desc = dsp::find(&s.effect)?;
                let mut params = desc.defaults();
                for (k, v) in &s.params {
                    if let Some(i) = desc.param_index(k) {
                        params[i] = desc.params[i].clamp(*v);
                    }
                }
                Some(SlotState { slot_id: self.alloc_id(), effect_id: desc.id.into(), params, bypass: s.bypass })
            })
            .collect()
    }

    fn chain_to_preset(&self) -> Vec<PresetSlot> {
        self.chain
            .iter()
            .filter_map(|s| {
                let desc = dsp::find(&s.effect_id)?;
                Some(PresetSlot {
                    effect: s.effect_id.clone(),
                    bypass: s.bypass,
                    params: desc.params.iter().zip(&s.params).map(|(p, v)| (p.id.to_string(), *v)).collect(),
                })
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Áudio

    pub fn refresh_devices(&mut self) {
        self.inputs = wasapi::list_devices(Flow::Capture).unwrap_or_default();
        self.outputs = wasapi::list_devices(Flow::Render).unwrap_or_default();
    }

    /// Cabo de guitarra conhecido > dispositivo padrão.
    fn guess_input(&self) -> Option<String> {
        self.inputs
            .iter()
            .find(|d| {
                let n = d.name.to_lowercase();
                GUITAR_HINTS.iter().any(|h| n.contains(h))
            })
            .map(|d| d.id.clone())
    }

    pub fn start_audio(&mut self) {
        self.stop_audio();
        self.refresh_devices();

        let input_id = self.settings.input_device.clone().or_else(|| self.guess_input());
        self.safety_mute = input_id.is_none();
        if let Some(id) = &input_id {
            if !self.inputs.iter().any(|d| &d.id == id) {
                self.status.error = Some("Entrada selecionada não encontrada. O cabo está conectado?".into());
                return;
            }
        }
        if let Some(id) = &self.settings.output_device {
            if !self.outputs.iter().any(|d| &d.id == id) {
                self.status.error = Some("Saída selecionada não encontrada.".into());
                return;
            }
        }

        let cfg = SessionConfig {
            input_id,
            output_id: self.settings.output_device.clone(),
            exclusive: self.settings.exclusive,
            input_channel: self.settings.input_channel,
        };

        let meters = self.meters.clone();
        let chain = self.chain.clone();
        let settings = self.settings.clone();
        let safety_mute = self.safety_mute;
        let mut control_side = None;
        let mut target_fill = 0usize;

        let result = Session::start(&cfg, |inp, out| {
            let sr = inp.sample_rate as f32;
            let (cmd_tx, cmd_rx) = RingBuffer::<Command>::new(1024);
            let (garbage_tx, garbage_rx) = RingBuffer::<Garbage>::new(256);
            let (audio_tx, mut audio_rx) = RingBuffer::<f32>::new(inp.sample_rate as usize * 2);
            let (tuner_tx, tuner_rx) = RingBuffer::<f32>::new(16384);

            let slots = build_slots(&chain, sr);
            let mut processor = Processor::new(ProcessorInit {
                sample_rate: sr,
                slots,
                commands: cmd_rx,
                garbage: garbage_tx,
                output: audio_tx,
                tuner: tuner_tx,
                meters: meters.clone(),
                input_gain: db_to_lin(settings.input_gain_db),
                output_gain: db_to_lin(settings.output_gain_db),
                muted: settings.muted || safety_mute,
            });

            // Alvo do buffer: o maior período dos dois lados + folga configurável.
            let out_period_in = out.period_frames as f32 * inp.sample_rate as f32 / out.sample_rate as f32;
            let safety = settings.safety_ms * 0.001 * sr;
            target_fill = (inp.period_frames as f32).max(out_period_in) as usize + safety as usize;
            let mut resampler = AdaptiveResampler::new(inp.sample_rate, out.sample_rate, target_fill);

            let tuner = Tuner::start(tuner_rx, sr, meters.clone());
            control_side = Some((cmd_tx, garbage_rx, tuner, sr));

            let meters_out = meters.clone();
            let mut last_underruns = 0;
            (
                Box::new(move |x: &[f32]| processor.process(x)),
                Box::new(move |buf: &mut [f32]| {
                    resampler.render(&mut audio_rx, buf);
                    if resampler.underruns != last_underruns {
                        meters_out.underruns.fetch_add(resampler.underruns - last_underruns, Ordering::Relaxed);
                        last_underruns = resampler.underruns;
                    }
                }),
            )
        });

        match result {
            Ok(session) => {
                let (commands, garbage, tuner, sample_rate) = control_side.expect("callbacks construídos");
                let (i, o) = (&session.input, &session.output);
                let ms = |frames: f32, sr: u32| frames * 1000.0 / sr as f32;
                // Estimativa ida-e-volta: período de captura + buffer da ponte +
                // buffer de saída + latências reportadas pelos drivers.
                let latency = ms(i.period_frames as f32, i.sample_rate)
                    + ms(target_fill as f32, i.sample_rate)
                    + ms(o.buffer_frames as f32, o.sample_rate)
                    + ms(i.device_latency_frames as f32, i.sample_rate).min(10.0)
                    + ms(o.device_latency_frames as f32, o.sample_rate).min(10.0);
                self.status = AudioStatus {
                    running: true,
                    error: None,
                    input: Some(i.clone()),
                    output: Some(o.clone()),
                    latency_ms: latency,
                };
                let tuner_reading = tuner.reading.clone();
                self.running = Some(Running { session, commands, garbage, _tuner: tuner, tuner_reading, sample_rate });
            }
            Err(e) => {
                self.status = AudioStatus { error: Some(format!("{e:#}")), ..Default::default() };
            }
        }
    }

    pub fn stop_audio(&mut self) {
        if let Some(r) = self.running.take() {
            r.session.stop();
        }
        self.status.running = false;
    }

    /// Chamado periodicamente: limpa lixo da thread de áudio e detecta queda
    /// do dispositivo. Retorna true se o estado mudou.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if self.dirty {
            self.dirty = false;
            self.save_config();
        }
        if let Some(r) = &mut self.running {
            while r.garbage.pop().is_ok() {}
            if let Some(err) = r.session.error() {
                self.stop_audio();
                self.status.error = Some(err);
                changed = true;
            }
        }
        changed
    }

    /// Tenta religar se o áudio caiu (ex.: cabo reconectado).
    pub fn try_recover(&mut self) -> bool {
        if self.running.is_none() && self.status.error.is_some() {
            self.start_audio();
            return self.running.is_some();
        }
        false
    }

    fn send(&mut self, cmd: Command) {
        if let Some(r) = &mut self.running {
            let _ = r.commands.push(cmd);
        }
    }

    fn restart_if_running(&mut self) {
        self.start_audio();
    }

    // -----------------------------------------------------------------------
    // Ações

    pub fn apply(&mut self, action: Action) -> Result<Outcome> {
        let outcome = match action {
            Action::GetState => Outcome::State,
            Action::RefreshDevices => {
                self.refresh_devices();
                Outcome::State
            }
            Action::SetDevices { input_device, output_device } => {
                self.settings.input_device = input_device;
                self.settings.output_device = output_device;
                self.restart_if_running();
                Outcome::State
            }
            Action::SetAudioOptions { exclusive, input_channel, safety_ms } => {
                if let Some(v) = exclusive {
                    self.settings.exclusive = v;
                }
                if let Some(v) = input_channel {
                    self.settings.input_channel = v;
                }
                if let Some(v) = safety_ms {
                    self.settings.safety_ms = v.clamp(0.0, 50.0);
                }
                self.restart_if_running();
                Outcome::State
            }
            Action::RestartAudio => {
                self.start_audio();
                Outcome::State
            }
            Action::SetInputGain { db } => {
                self.settings.input_gain_db = db.clamp(-24.0, 24.0);
                self.send(Command::SetInputGain(db_to_lin(self.settings.input_gain_db)));
                Outcome::State
            }
            Action::SetOutputGain { db } => {
                self.settings.output_gain_db = db.clamp(-60.0, 12.0);
                self.send(Command::SetOutputGain(db_to_lin(self.settings.output_gain_db)));
                Outcome::State
            }
            Action::SetMute { muted } => {
                self.settings.muted = muted;
                if !muted {
                    self.safety_mute = false;
                }
                self.send(Command::SetMute(muted || self.safety_mute));
                Outcome::State
            }
            Action::SetTuner { enabled } => {
                self.meters.tuner_enabled.store(enabled, Ordering::Relaxed);
                Outcome::State
            }
            Action::AddEffect { effect_id, index } => {
                let desc = dsp::find(&effect_id).ok_or_else(|| anyhow!("efeito desconhecido: {effect_id}"))?;
                if self.chain.len() >= processor::MAX_SLOTS {
                    bail!("limite de {} efeitos na cadeia", processor::MAX_SLOTS);
                }
                let state = SlotState { slot_id: self.alloc_id(), effect_id: desc.id.into(), params: desc.defaults(), bypass: false };
                let index = index.unwrap_or(self.chain.len()).min(self.chain.len());
                if let Some(sr) = self.running.as_ref().map(|r| r.sample_rate) {
                    let slot = Slot::new(state.slot_id, desc.instantiate(sr, &state.params), false, sr);
                    self.send(Command::Insert { index, slot });
                }
                self.chain.insert(index, state);
                self.current_preset = None;
                Outcome::State
            }
            Action::RemoveEffect { slot_id } => {
                self.chain.retain(|s| s.slot_id != slot_id);
                self.send(Command::Remove { slot: slot_id });
                self.current_preset = None;
                Outcome::State
            }
            Action::MoveEffect { slot_id, to } => {
                let from = self.chain.iter().position(|s| s.slot_id == slot_id).ok_or_else(|| anyhow!("slot inexistente"))?;
                let s = self.chain.remove(from);
                let to = to.min(self.chain.len());
                self.chain.insert(to, s);
                self.send(Command::Move { slot: slot_id, to });
                self.current_preset = None;
                Outcome::State
            }
            Action::SetParam { slot_id, index, value } => {
                let slot = self.chain.iter_mut().find(|s| s.slot_id == slot_id).ok_or_else(|| anyhow!("slot inexistente"))?;
                let desc = dsp::find(&slot.effect_id).ok_or_else(|| anyhow!("efeito desconhecido"))?;
                let spec = desc.params.get(index).ok_or_else(|| anyhow!("parâmetro inexistente"))?;
                let value = spec.clamp(value);
                slot.params[index] = value;
                self.send(Command::SetParam { slot: slot_id, index, value });
                Outcome::Param { slot_id, index, value }
            }
            Action::SetBypass { slot_id, bypass } => {
                let slot = self.chain.iter_mut().find(|s| s.slot_id == slot_id).ok_or_else(|| anyhow!("slot inexistente"))?;
                slot.bypass = bypass;
                self.send(Command::SetBypass { slot: slot_id, bypass });
                Outcome::State
            }
            Action::LoadPreset { name } => {
                let preset = self.store.load(&name)?;
                self.chain = self.chain_from_preset(&preset.chain);
                if let Some(sr) = self.running.as_ref().map(|r| r.sample_rate) {
                    let slots = build_slots(&self.chain, sr);
                    self.send(Command::ReplaceChain(slots));
                }
                self.current_preset = Some(preset.name);
                Outcome::State
            }
            Action::SavePreset { name } => {
                let name = presets::sanitize_name(&name)?;
                self.store.save(&Preset { name: name.clone(), chain: self.chain_to_preset() })?;
                self.current_preset = Some(name);
                Outcome::State
            }
            Action::DeletePreset { name } => {
                self.store.delete(&name)?;
                if self.current_preset.as_deref() == Some(name.as_str()) {
                    self.current_preset = None;
                }
                Outcome::State
            }
        };
        self.dirty = true;
        Ok(outcome)
    }

    // -----------------------------------------------------------------------
    // Serialização para a UI

    pub fn state_json(&self) -> Value {
        let factory: Vec<String> = presets::factory().into_iter().map(|p| p.name).collect();
        json!({
            "type": "state",
            "settings": self.settings,
            "audio": self.status,
            "devices": { "inputs": self.inputs, "outputs": self.outputs, "guessedInput": self.guess_input() },
            "effects": dsp::effects::REGISTRY,
            "chain": self.chain,
            "presets": { "factory": factory, "user": self.store.list_user(), "current": self.current_preset },
            "tunerEnabled": self.meters.tuner_enabled.load(Ordering::Relaxed),
            "safetyMute": self.safety_mute,
        })
    }

    pub fn meters_json(&self) -> Value {
        let (inp, out) = self.meters.take_peaks();
        let tuner = self.running.as_ref().map(|r| r.tuner_reading.lock().clone());
        json!({
            "type": "meters",
            "input": inp,
            "output": out,
            "dspLoad": self.meters.dsp_load(),
            "underruns": self.meters.underruns.load(Ordering::Relaxed),
            "overflows": self.meters.overflows.load(Ordering::Relaxed),
            "tuner": tuner,
        })
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop_audio();
        if self.dirty {
            self.save_config();
        }
    }
}

fn build_slots(chain: &[SlotState], sr: f32) -> Vec<Slot> {
    let mut slots = Vec::with_capacity(processor::MAX_SLOTS);
    for s in chain {
        if let Some(desc) = dsp::find(&s.effect_id) {
            slots.push(Slot::new(s.slot_id, desc.instantiate(sr, &s.params), s.bypass, sr));
        }
    }
    slots
}
