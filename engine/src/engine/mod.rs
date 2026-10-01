//! Lado de controle do motor: guarda o estado (cadeia, configurações),
//! gerencia a sessão de áudio e traduz ações da UI em comandos lock-free
//! para a thread de áudio.

pub mod files;
pub mod midi;
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
use files::FileLibrary;
use midi::{Midi, MidiBinding, MidiEvent, MidiTarget};
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
    pub bpm: f32,
    pub midi_bindings: Vec<MidiBinding>,
    /// Program Change N carrega o preset N (fábrica primeiro, depois os seus).
    pub midi_program_change: bool,
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
            bpm: 120.0,
            midi_bindings: Vec::new(),
            midi_program_change: true,
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
    /// Arquivo da biblioteca usado pelo efeito (IR, captura NAM).
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, skip_deserializing)]
    pub file_error: Option<String>,
    #[serde(default, skip_deserializing)]
    pub file_warning: Option<String>,
}

impl SlotState {
    fn new(slot_id: u64, effect_id: &str, params: Vec<f32>, bypass: bool, file: Option<String>) -> Self {
        Self { slot_id, effect_id: effect_id.into(), params, bypass, file, file_error: None, file_warning: None }
    }
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
    SetBpm { bpm: f32 },
    /// Próximo CC recebido é ligado a `target` (None cancela).
    MidiLearn { target: Option<MidiTarget> },
    MidiUnbind { index: usize },
    SetMidiProgramChange { enabled: bool },
    AddEffect { effect_id: String, index: Option<usize> },
    RemoveEffect { slot_id: u64 },
    MoveEffect { slot_id: u64, to: usize },
    SetParam { slot_id: u64, index: usize, value: f32 },
    SetBypass { slot_id: u64, bypass: bool },
    LoadPreset { name: String },
    /// `data` em base64 (o arquivo vem do navegador pelo WebSocket).
    /// Com `slot_id`, já aplica o arquivo naquele pedal.
    UploadFile { name: String, data: String, slot_id: Option<u64> },
    DeleteFile { name: String },
    SetFile { slot_id: u64, file: Option<String> },
    SavePreset { name: String },
    DeletePreset { name: String },
}

/// O que mudou após uma ação, para decidir o que transmitir às UIs.
pub enum Outcome {
    State,
    Param { slot_id: u64, index: usize, value: f32 },
    /// Ação sem mudança de estado (botões do looper).
    Nothing,
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
    library: FileLibrary,
    config_path: PathBuf,
    dirty: bool,
    /// Mudo automático quando nenhuma entrada de guitarra foi escolhida nem
    /// detectada: evita microfonia do microfone padrão nos alto-falantes.
    safety_mute: bool,
    midi: Midi,
    midi_learn: Option<MidiTarget>,
    /// Último estado (>= 64) de cada CC, para footswitch agir só ao pisar.
    cc_high: [bool; 128],
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
            library: FileLibrary::new(data_dir.join("files")),
            config_path,
            dirty: false,
            safety_mute: false,
            midi: Midi::new(),
            midi_learn: None,
            cc_high: [false; 128],
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
                Some(SlotState::new(self.alloc_id(), desc.id, params, s.bypass, s.file.clone()))
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
                    file: s.file.clone(),
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
        let library = self.library.clone();
        let mut file_status = Vec::new();
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

            let (slots, status) = build_slots(&library, &chain, sr);
            file_status = status;
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
                self.apply_file_status(file_status);
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
        for ev in self.midi.drain() {
            changed |= match ev {
                MidiEvent::ControlChange { cc, value } => self.apply_cc(cc, value),
                MidiEvent::ProgramChange { program } => self.apply_program_change(program),
            };
        }
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

    /// Procura dispositivos MIDI novos/desconectados. Retorna se mudou.
    pub fn midi_rescan(&mut self) -> bool {
        self.midi.rescan()
    }

    fn apply_cc(&mut self, cc: u8, value: u8) -> bool {
        let cc_i = (cc & 0x7F) as usize;
        if let Some(target) = self.midi_learn.take() {
            self.settings.midi_bindings.retain(|b| b.cc != cc && b.target != target);
            self.settings.midi_bindings.push(MidiBinding { cc, target });
            self.cc_high[cc_i] = value >= 64;
            self.dirty = true;
            return true;
        }
        let rising = value >= 64 && !self.cc_high[cc_i];
        self.cc_high[cc_i] = value >= 64;
        let n = value as f32 / 127.0;
        let mut actions = Vec::new();
        for b in self.settings.midi_bindings.iter().filter(|b| b.cc == cc) {
            match &b.target {
                MidiTarget::Param { effect_id, param_id } => {
                    let Some(slot) = self.chain.iter().find(|s| &s.effect_id == effect_id) else { continue };
                    let Some(desc) = dsp::find(effect_id) else { continue };
                    let Some(index) = desc.param_index(param_id) else { continue };
                    let spec = &desc.params[index];
                    let value = match spec.kind {
                        dsp::ParamKind::Knob { log: true } if spec.min > 0.0 => spec.min * (spec.max / spec.min).powf(n),
                        dsp::ParamKind::Knob { .. } | dsp::ParamKind::Choice { .. } => spec.min + (spec.max - spec.min) * n,
                        dsp::ParamKind::Toggle => if value >= 64 { 1.0 } else { 0.0 },
                        dsp::ParamKind::Actions { .. } => continue,
                    };
                    actions.push(Action::SetParam { slot_id: slot.slot_id, index, value });
                }
                // Footswitch: alterna a cada pisada (momentâneo ou latching).
                MidiTarget::Bypass { effect_id } if rising => {
                    if let Some(slot) = self.chain.iter().find(|s| &s.effect_id == effect_id) {
                        actions.push(Action::SetBypass { slot_id: slot.slot_id, bypass: !slot.bypass });
                    }
                }
                MidiTarget::Bypass { .. } => {}
                MidiTarget::OutputVolume => actions.push(Action::SetOutputGain { db: -60.0 + 72.0 * n }),
            }
        }
        let changed = !actions.is_empty();
        for a in actions {
            let _ = self.apply(a);
        }
        changed
    }

    fn apply_program_change(&mut self, program: u8) -> bool {
        if !self.settings.midi_program_change {
            return false;
        }
        let names: Vec<String> = presets::factory().into_iter().map(|p| p.name).chain(self.store.list_user()).collect();
        match names.get(program as usize) {
            Some(name) => self.apply(Action::LoadPreset { name: name.clone() }).is_ok(),
            None => false,
        }
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
            Action::SetBpm { bpm } => {
                self.settings.bpm = bpm.clamp(30.0, 300.0);
                self.apply_tempo();
                Outcome::State
            }
            Action::MidiLearn { target } => {
                self.midi_learn = target;
                Outcome::State
            }
            Action::MidiUnbind { index } => {
                if index < self.settings.midi_bindings.len() {
                    self.settings.midi_bindings.remove(index);
                }
                Outcome::State
            }
            Action::SetMidiProgramChange { enabled } => {
                self.settings.midi_program_change = enabled;
                Outcome::State
            }
            Action::SetTuner { enabled } => {
                self.meters.tuner_enabled.store(enabled, Ordering::Relaxed);
                Outcome::State
            }
            Action::AddEffect { effect_id, index } => {
                let desc = dsp::find(&effect_id).ok_or_else(|| anyhow!("efeito desconhecido: {effect_id}"))?;
                if desc.id == "looper" && self.chain.iter().any(|s| s.effect_id == "looper") {
                    bail!("só um Looper por vez na cadeia");
                }
                if self.chain.len() >= processor::MAX_SLOTS {
                    bail!("limite de {} efeitos na cadeia", processor::MAX_SLOTS);
                }
                let state = SlotState::new(self.alloc_id(), desc.id, desc.defaults(), false, None);
                let index = index.unwrap_or(self.chain.len()).min(self.chain.len());
                if let Some(sr) = self.running.as_ref().map(|r| r.sample_rate) {
                    let slot = Slot::new(state.slot_id, desc.instantiate(sr, &state.params), false, sr);
                    self.send(Command::Insert { index, slot });
                }
                self.chain.insert(index, state);
                self.current_preset = None;
                self.apply_tempo();
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
                if spec.is_action() {
                    self.send(Command::SetParam { slot: slot_id, index, value });
                    return Ok(Outcome::Nothing);
                }
                slot.params[index] = value;
                // Mexer no knob controlado pelo BPM desliga o sync daquele pedal.
                let mut outcome = Outcome::Param { slot_id, index, value };
                if let Some(t) = desc.tempo {
                    if index == t.target_param && slot.params[t.sync_param] != 0.0 {
                        slot.params[t.sync_param] = 0.0;
                        outcome = Outcome::State;
                    } else if index == t.sync_param {
                        outcome = Outcome::State;
                    }
                }
                self.send(Command::SetParam { slot: slot_id, index, value });
                self.apply_tempo();
                outcome
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
                    let (slots, status) = build_slots(&self.library, &self.chain, sr);
                    self.apply_file_status(status);
                    self.send(Command::ReplaceChain(slots));
                }
                self.current_preset = Some(preset.name);
                self.apply_tempo();
                Outcome::State
            }
            Action::SavePreset { name } => {
                let name = presets::sanitize_name(&name)?;
                self.store.save(&Preset { name: name.clone(), chain: self.chain_to_preset() })?;
                self.current_preset = Some(name);
                Outcome::State
            }
            Action::UploadFile { name, data, slot_id } => {
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data.as_bytes())
                    .map_err(|_| anyhow!("upload corrompido"))?;
                let saved = self.library.save(&name, &bytes)?;
                match slot_id {
                    Some(slot_id) => return self.apply(Action::SetFile { slot_id, file: Some(saved) }),
                    None => Outcome::State,
                }
            }
            Action::DeleteFile { name } => {
                self.library.delete(&name)?;
                Outcome::State
            }
            Action::SetFile { slot_id, file } => {
                let idx = self.chain.iter().position(|s| s.slot_id == slot_id).ok_or_else(|| anyhow!("slot inexistente"))?;
                if let Some(f) = &file {
                    files::sanitize(f)?;
                }
                self.chain[idx].file = file;
                self.chain[idx].file_error = None;
                self.chain[idx].file_warning = None;
                if let Some(sr) = self.running.as_ref().map(|r| r.sample_rate) {
                    let (slot, status) = instantiate_slot(&self.library, &self.chain[idx], sr);
                    self.apply_file_status(vec![status]);
                    self.send(Command::ReplaceSlot(slot));
                }
                self.current_preset = None;
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
        if !matches!(outcome, Outcome::Nothing) {
            self.dirty = true;
        }
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
            "files": self.library.list(),
            "midi": { "devices": self.midi.devices(), "learning": self.midi_learn },
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
            "looper": {
                "state": dsp::effects::looper::LoopState::label(dsp::effects::looper::STATE.load(Ordering::Relaxed)),
                "position": dsp::effects::looper::POSITION.load(Ordering::Relaxed),
                "length": dsp::effects::looper::LENGTH.load(Ordering::Relaxed),
                "sampleRate": dsp::effects::looper::SAMPLE_RATE.load(Ordering::Relaxed),
            },
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

/// Resultado do carregamento de arquivo de um slot: (id, erro, aviso).
type FileStatus = (u64, Option<String>, Option<String>);

impl Controller {
    /// Atualiza tempo/rate dos pedais com Sync ligado a partir do BPM.
    fn apply_tempo(&mut self) {
        let bpm = self.settings.bpm;
        let mut changes = Vec::new();
        for slot in &mut self.chain {
            let Some(desc) = dsp::find(&slot.effect_id) else { continue };
            let Some(t) = desc.tempo else { continue };
            if let Some(v) = t.value(bpm, slot.params[t.sync_param]) {
                let v = desc.params[t.target_param].clamp(v);
                if (slot.params[t.target_param] - v).abs() > 1e-4 {
                    slot.params[t.target_param] = v;
                    changes.push((slot.slot_id, t.target_param, v));
                }
            }
        }
        for (slot, index, value) in changes {
            self.send(Command::SetParam { slot, index, value });
        }
    }

    fn apply_file_status(&mut self, status: Vec<FileStatus>) {
        for (id, error, warning) in status {
            if let Some(s) = self.chain.iter_mut().find(|s| s.slot_id == id) {
                s.file_error = error;
                s.file_warning = warning;
            }
        }
    }
}

/// Aviso quando a captura NAM foi treinada em outra taxa de amostragem.
fn nam_rate_warning(data: &[u8], sr: f32) -> Option<String> {
    let json: Value = serde_json::from_slice(data).ok()?;
    let rate = dsp::nam::parse_info(&json).sample_rate?;
    ((rate - sr as f64).abs() > 1.0)
        .then(|| format!("Captura feita em {rate} Hz, mas o áudio está em {sr} Hz: o timbre pode mudar um pouco."))
}

/// Cria a instância de um slot, carregando o arquivo se houver.
fn instantiate_slot(library: &FileLibrary, s: &SlotState, sr: f32) -> (Slot, FileStatus) {
    let desc = dsp::find(&s.effect_id).expect("efeito validado ao entrar na cadeia");
    let data = match &s.file {
        Some(name) => match library.read(name) {
            Ok(d) => Some(d),
            Err(e) => {
                let slot = Slot::new(s.slot_id, desc.instantiate(sr, &s.params), s.bypass, sr);
                return (slot, (s.slot_id, Some(format!("{e:#}")), None));
            }
        },
        None => None,
    };
    let warning = match (&data, &desc.file) {
        (Some(d), Some(fs)) if fs.kind == dsp::FileKind::Nam => nam_rate_warning(d, sr),
        _ => None,
    };
    let (fx, error) = desc.instantiate_with(sr, &s.params, data.as_deref());
    (Slot::new(s.slot_id, fx, s.bypass, sr), (s.slot_id, error, warning))
}

fn build_slots(library: &FileLibrary, chain: &[SlotState], sr: f32) -> (Vec<Slot>, Vec<FileStatus>) {
    let mut slots = Vec::with_capacity(processor::MAX_SLOTS);
    let mut status = Vec::new();
    for s in chain.iter().filter(|s| dsp::find(&s.effect_id).is_some()) {
        let (slot, st) = instantiate_slot(library, s, sr);
        slots.push(slot);
        status.push(st);
    }
    (slots, status)
}

#[cfg(test)]
mod midi_tests {
    use super::*;

    fn controller() -> Controller {
        let dir = std::env::temp_dir().join(format!("screamo-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Controller::new(dir)
    }

    #[test]
    fn learn_then_cc_moves_param_and_footswitch_toggles() {
        let mut c = controller();
        c.apply(Action::LoadPreset { name: "ScreamO Lead".into() }).unwrap();
        // Pedal de expressão (CC 11) no Time do delay (knob log 20..2000 ms).
        c.apply(Action::MidiLearn { target: Some(MidiTarget::Param { effect_id: "delay".into(), param_id: "time".into() }) }).unwrap();
        assert!(c.apply_cc(11, 0));
        assert_eq!(c.settings.midi_bindings.len(), 1);
        c.apply_cc(11, 127);
        let delay = c.chain.iter().find(|s| s.effect_id == "delay").unwrap();
        assert!((delay.params[0] - 2000.0).abs() < 0.5, "{}", delay.params[0]);

        // Footswitch (CC 80) no bypass do overdrive: alterna só ao pisar.
        c.apply(Action::MidiLearn { target: Some(MidiTarget::Bypass { effect_id: "overdrive".into() }) }).unwrap();
        c.apply_cc(80, 0);
        let bypass = |c: &Controller| c.chain.iter().find(|s| s.effect_id == "overdrive").unwrap().bypass;
        let before = bypass(&c);
        c.apply_cc(80, 127);
        assert_ne!(bypass(&c), before);
        c.apply_cc(80, 127); // segurando: não alterna de novo
        assert_ne!(bypass(&c), before);
        c.apply_cc(80, 0);
        c.apply_cc(80, 127);
        assert_eq!(bypass(&c), before);

        // Program Change 0 = primeiro preset de fábrica.
        assert!(c.apply_program_change(0));
        assert_eq!(c.current_preset.as_deref(), Some(presets::factory()[0].name.as_str()));
    }
}
