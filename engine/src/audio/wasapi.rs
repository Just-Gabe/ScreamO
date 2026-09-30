//! Backend de áudio WASAPI (nativo do Windows, sem ASIO e sem driver extra).
//!
//! Estratégia de latência, da melhor para a mais compatível:
//! 1. **Exclusivo** (opcional): acesso direto ao dispositivo com o menor
//!    período de hardware (~3 ms). Bloqueia o som de outros apps nesse device.
//! 2. **Compartilhado de baixa latência** (`IAudioClient3`, Windows 10+):
//!    períodos de até 2,67 ms sem tirar o som dos outros apps.
//! 3. **Compartilhado padrão**: período de ~10 ms, funciona em qualquer driver.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, WaitForSingleObject,
};
use windows::core::{GUID, HSTRING, PCWSTR, w};

const WAVE_FORMAT_PCM_TAG: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const SPEAKER_FRONT_LEFT_RIGHT: u32 = 0x3;

/// Maior pacote que convertemos de uma vez (frames).
const MAX_FRAMES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Capture,
    Render,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Qual canal do dispositivo de entrada carrega a guitarra.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputChannel {
    /// Escolhe automaticamente o canal com mais sinal.
    #[default]
    Auto,
    Left,
    Right,
    Mix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    F32,
    I16,
    I24,
    I32,
}

#[derive(Debug, Clone, Copy)]
struct Format {
    sample_rate: u32,
    channels: u16,
    kind: SampleKind,
    block_align: usize,
}

impl Format {
    fn describe(&self) -> String {
        let k = match self.kind {
            SampleKind::F32 => "32-bit float",
            SampleKind::I16 => "16-bit",
            SampleKind::I24 => "24-bit",
            SampleKind::I32 => "32-bit",
        };
        format!("{} Hz · {} · {} ch", self.sample_rate, k, self.channels)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
    pub period_frames: u32,
    pub buffer_frames: u32,
    /// Latência reportada pelo driver/engine (frames).
    pub device_latency_frames: u32,
    pub mode: String,
}

struct ComGuard(bool);

impl ComGuard {
    fn init() -> Self {
        let ok = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self(ok)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    Ok(unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? })
}

fn data_flow(flow: Flow) -> EDataFlow {
    match flow {
        Flow::Capture => eCapture,
        Flow::Render => eRender,
    }
}

unsafe fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let p = device.GetId()?;
        let s = p.to_string()?;
        CoTaskMemFree(Some(p.0 as _));
        Ok(s)
    }
}

unsafe fn device_name(device: &IMMDevice) -> String {
    unsafe {
        device
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "Dispositivo desconhecido".into())
    }
}

pub fn list_devices(flow: Flow) -> Result<Vec<DeviceInfo>> {
    let _com = ComGuard::init();
    unsafe {
        let en = enumerator()?;
        let default_id = en
            .GetDefaultAudioEndpoint(data_flow(flow), eConsole)
            .ok()
            .and_then(|d| device_id(&d).ok());
        let collection = en.EnumAudioEndpoints(data_flow(flow), DEVICE_STATE_ACTIVE)?;
        let mut out = Vec::new();
        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;
            let id = device_id(&device)?;
            out.push(DeviceInfo {
                is_default: Some(&id) == default_id.as_ref(),
                name: device_name(&device),
                id,
            });
        }
        Ok(out)
    }
}

unsafe fn get_device(en: &IMMDeviceEnumerator, flow: Flow, id: Option<&str>) -> Result<IMMDevice> {
    unsafe {
        match id {
            Some(id) => en.GetDevice(&HSTRING::from(id)).with_context(|| "dispositivo não encontrado (desconectado?)"),
            None => en
                .GetDefaultAudioEndpoint(data_flow(flow), eConsole)
                .with_context(|| "nenhum dispositivo padrão"),
        }
    }
}

unsafe fn parse_format(p: *const WAVEFORMATEX) -> Result<Format> {
    unsafe {
        let wf = *p;
        let tag = wf.wFormatTag;
        let bits = wf.wBitsPerSample;
        let kind = if tag == WAVE_FORMAT_EXTENSIBLE_TAG {
            let ext = *(p as *const WAVEFORMATEXTENSIBLE);
            let sub = ext.SubFormat;
            if sub == SUBTYPE_FLOAT {
                SampleKind::F32
            } else if sub == SUBTYPE_PCM {
                pcm_kind(bits)?
            } else {
                bail!("formato de áudio não suportado");
            }
        } else if tag == WAVE_FORMAT_IEEE_FLOAT_TAG {
            SampleKind::F32
        } else if tag == WAVE_FORMAT_PCM_TAG {
            pcm_kind(bits)?
        } else {
            bail!("formato de áudio não suportado (tag {tag})");
        };
        Ok(Format {
            sample_rate: wf.nSamplesPerSec,
            channels: wf.nChannels,
            kind,
            block_align: wf.nBlockAlign as usize,
        })
    }
}

fn pcm_kind(container_bits: u16) -> Result<SampleKind> {
    Ok(match container_bits {
        16 => SampleKind::I16,
        24 => SampleKind::I24,
        32 => SampleKind::I32,
        b => bail!("PCM de {b} bits não suportado"),
    })
}

fn make_format(sample_rate: u32, channels: u16, kind: SampleKind) -> WAVEFORMATEXTENSIBLE {
    let (container, valid, sub) = match kind {
        SampleKind::F32 => (32, 32, SUBTYPE_FLOAT),
        SampleKind::I32 => (32, 24, SUBTYPE_PCM),
        SampleKind::I24 => (24, 24, SUBTYPE_PCM),
        SampleKind::I16 => (16, 16, SUBTYPE_PCM),
    };
    let block_align = channels * container / 8;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE_TAG,
            nChannels: channels,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: sample_rate * block_align as u32,
            nBlockAlign: block_align,
            wBitsPerSample: container,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 { wValidBitsPerSample: valid },
        dwChannelMask: if channels == 2 { SPEAKER_FRONT_LEFT_RIGHT } else { 0 },
        SubFormat: sub,
    }
}

struct OpenStream {
    client: IAudioClient,
    event: HANDLE,
    format: Format,
    buffer_frames: u32,
    exclusive: bool,
    info: StreamInfo,
}

impl Drop for OpenStream {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

const HNS_PER_SEC: f64 = 10_000_000.0;

unsafe fn open_stream(flow: Flow, device_id_opt: Option<&str>, exclusive: bool) -> Result<OpenStream> {
    unsafe {
        let en = enumerator()?;
        let device = get_device(&en, flow, device_id_opt)?;
        let name = device_name(&device);

        let (client, format, period_frames, mode) = if exclusive {
            match init_exclusive(&device) {
                Ok(v) => v,
                Err(e) => {
                    log::warn(&format!("modo exclusivo indisponível em '{name}': {e:#}; usando compartilhado"));
                    init_shared(&device)?
                }
            }
        } else {
            init_shared(&device)?
        };

        let event = CreateEventW(None, false, false, PCWSTR::null())?;
        client.SetEventHandle(event)?;
        let buffer_frames = client.GetBufferSize()?;
        let latency_hns = client.GetStreamLatency().unwrap_or(0);
        let device_latency_frames = (latency_hns as f64 / HNS_PER_SEC * format.sample_rate as f64) as u32;
        let is_exclusive = mode.starts_with("Exclusivo");
        let info = StreamInfo {
            device_name: name,
            sample_rate: format.sample_rate,
            channels: format.channels,
            format: format.describe(),
            period_frames,
            buffer_frames,
            device_latency_frames,
            mode,
        };
        Ok(OpenStream { client, event, format, buffer_frames, exclusive: is_exclusive, info })
    }
}

unsafe fn init_shared(device: &IMMDevice) -> Result<(IAudioClient, Format, u32, String)> {
    unsafe {
        // 1ª tentativa: IAudioClient3 com o menor período suportado pelo engine.
        if let Ok(client3) = device.Activate::<IAudioClient3>(CLSCTX_ALL, None) {
            let mix = client3.GetMixFormat()?;
            let format = parse_format(mix);
            let (mut def, mut fund, mut min, mut max) = (0u32, 0u32, 0u32, 0u32);
            let res = client3
                .GetSharedModeEnginePeriod(mix, &mut def, &mut fund, &mut min, &mut max)
                .and_then(|_| client3.InitializeSharedAudioStream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK, min, mix, None));
            CoTaskMemFree(Some(mix as _));
            if let (Ok(()), Ok(format)) = (res, format) {
                let mode = if min < def { "Compartilhado baixa latência" } else { "Compartilhado" };
                return Ok((client3.into(), format, min, mode.to_string()));
            }
        }

        // Fallback: IAudioClient clássico com o período padrão.
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mix = client.GetMixFormat()?;
        let format = parse_format(mix);
        let res = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 0, 0, mix, None);
        CoTaskMemFree(Some(mix as _));
        res?;
        let format = format?;
        let mut def = 0i64;
        client.GetDevicePeriod(Some(&mut def), None)?;
        let period = (def as f64 / HNS_PER_SEC * format.sample_rate as f64) as u32;
        Ok((client, format, period, "Compartilhado".into()))
    }
}

unsafe fn init_exclusive(device: &IMMDevice) -> Result<(IAudioClient, Format, u32, String)> {
    unsafe {
        let probe: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mix = probe.GetMixFormat()?;
        let mix_fmt = parse_format(mix);
        CoTaskMemFree(Some(mix as _));
        let mix_fmt = mix_fmt?;
        let mut min_period = 0i64;
        probe.GetDevicePeriod(None, Some(&mut min_period))?;

        let rates = [mix_fmt.sample_rate, 48000, 44100];
        let channel_opts = [mix_fmt.channels, 2, 1];
        let kinds = [SampleKind::F32, SampleKind::I32, SampleKind::I24, SampleKind::I16];
        let mut chosen = None;
        'search: for &rate in &rates {
            for &ch in &channel_opts {
                for &kind in &kinds {
                    let wfx = make_format(rate, ch, kind);
                    let p = &wfx as *const _ as *const WAVEFORMATEX;
                    if probe.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, p, None) == windows::Win32::Foundation::S_OK {
                        chosen = Some(wfx);
                        break 'search;
                    }
                }
            }
        }
        drop(probe);
        let wfx = chosen.ok_or_else(|| anyhow!("nenhum formato aceito em modo exclusivo"))?;
        let p = &wfx as *const _ as *const WAVEFORMATEX;
        let format = parse_format(p)?;

        let mut period = min_period;
        for _ in 0..2 {
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            match client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, period, p, None) {
                Ok(()) => {
                    let frames = (period as f64 / HNS_PER_SEC * format.sample_rate as f64).round() as u32;
                    return Ok((client, format, frames, "Exclusivo".into()));
                }
                Err(e) if e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => {
                    // Recalcula o período alinhado ao tamanho de buffer do hardware.
                    let frames = client.GetBufferSize()?;
                    period = (HNS_PER_SEC * frames as f64 / format.sample_rate as f64 + 0.5) as i64;
                }
                Err(e) => return Err(e.into()),
            }
        }
        bail!("não foi possível alinhar o buffer em modo exclusivo")
    }
}

// ---------------------------------------------------------------------------
// Conversão de amostras

#[inline]
fn read_sample(bytes: &[u8], kind: SampleKind) -> f32 {
    match kind {
        SampleKind::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        SampleKind::I16 => i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32768.0,
        SampleKind::I24 => (i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8) as f32 / 8_388_608.0,
        SampleKind::I32 => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32 / 2_147_483_648.0,
    }
}

#[inline]
fn write_sample(bytes: &mut [u8], kind: SampleKind, v: f32) {
    let v = v.clamp(-1.0, 1.0);
    match kind {
        SampleKind::F32 => bytes[..4].copy_from_slice(&v.to_le_bytes()),
        SampleKind::I16 => bytes[..2].copy_from_slice(&((v * 32767.0) as i16).to_le_bytes()),
        SampleKind::I24 => bytes[..3].copy_from_slice(&((v * 8_388_607.0) as i32).to_le_bytes()[..3]),
        SampleKind::I32 => bytes[..4].copy_from_slice(&((v as f64 * 2_147_483_647.0) as i32).to_le_bytes()),
    }
}

fn sample_bytes(kind: SampleKind) -> usize {
    match kind {
        SampleKind::F32 | SampleKind::I32 => 4,
        SampleKind::I24 => 3,
        SampleKind::I16 => 2,
    }
}

/// Seleção automática de canal com histerese (cabos USB de guitarra às vezes
/// entregam o sinal só na esquerda, às vezes nos dois).
struct ChannelPicker {
    mode: InputChannel,
    energy: [f32; 2],
    current: usize,
}

impl ChannelPicker {
    fn pick(&mut self, data: &[u8], fmt: &Format, frames: usize, out: &mut [f32]) {
        let bps = sample_bytes(fmt.kind);
        let ch = fmt.channels as usize;
        let get = |f: usize, c: usize| read_sample(&data[f * fmt.block_align + c * bps..], fmt.kind);
        if ch == 1 {
            for (f, o) in out[..frames].iter_mut().enumerate() {
                *o = get(f, 0);
            }
            return;
        }
        match self.mode {
            InputChannel::Left => (0..frames).for_each(|f| out[f] = get(f, 0)),
            InputChannel::Right => (0..frames).for_each(|f| out[f] = get(f, 1)),
            InputChannel::Mix => (0..frames).for_each(|f| out[f] = 0.5 * (get(f, 0) + get(f, 1))),
            InputChannel::Auto => {
                let mut e = [0f32; 2];
                for f in 0..frames {
                    let (a, b) = (get(f, 0), get(f, 1));
                    e[0] += a * a;
                    e[1] += b * b;
                }
                for c in 0..2 {
                    self.energy[c] = self.energy[c] * 0.95 + e[c];
                }
                let other = 1 - self.current;
                if self.energy[other] > self.energy[self.current] * 4.0 {
                    self.current = other;
                }
                let c = self.current;
                (0..frames).for_each(|f| out[f] = get(f, c));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sessão (captura + reprodução)

pub type InputCallback = Box<dyn FnMut(&[f32]) + Send>;
/// Recebe um buffer estéreo intercalado (L,R,L,R...) para preencher.
pub type OutputCallback = Box<dyn FnMut(&mut [f32]) + Send>;

pub struct SessionConfig {
    pub input_id: Option<String>,
    pub output_id: Option<String>,
    pub exclusive: bool,
    pub input_channel: InputChannel,
}

pub struct Session {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    pub input: StreamInfo,
    pub output: StreamInfo,
}

impl Session {
    /// Abre os dois dispositivos e chama `build` com os formatos reais para
    /// montar os callbacks (o motor precisa saber a taxa de amostragem).
    pub fn start(
        cfg: &SessionConfig,
        build: impl FnOnce(&StreamInfo, &StreamInfo) -> (InputCallback, OutputCallback),
    ) -> Result<Session> {
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));

        let (in_info_tx, in_info_rx) = std::sync::mpsc::channel::<Result<StreamInfo>>();
        let (in_cb_tx, in_cb_rx) = std::sync::mpsc::channel::<Option<InputCallback>>();
        let (out_info_tx, out_info_rx) = std::sync::mpsc::channel::<Result<StreamInfo>>();
        let (out_cb_tx, out_cb_rx) = std::sync::mpsc::channel::<Option<OutputCallback>>();

        let capture = {
            let (stop, error) = (stop.clone(), error.clone());
            let (id, exclusive, channel) = (cfg.input_id.clone(), cfg.exclusive, cfg.input_channel);
            std::thread::Builder::new().name("screamo-capture".into()).spawn(move || {
                let _com = ComGuard::init();
                let stream = match unsafe { open_stream(Flow::Capture, id.as_deref(), exclusive) } {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = in_info_tx.send(Err(e));
                        return;
                    }
                };
                let _ = in_info_tx.send(Ok(stream.info.clone()));
                let Ok(Some(cb)) = in_cb_rx.recv() else { return };
                let picker = ChannelPicker { mode: channel, energy: [0.0; 2], current: 0 };
                if let Err(e) = unsafe { run_capture(&stream, &stop, picker, cb) } {
                    fail(&stop, &error, format!("Entrada: {e:#}"));
                }
            })?
        };

        let render = {
            let (stop, error) = (stop.clone(), error.clone());
            let (id, exclusive) = (cfg.output_id.clone(), cfg.exclusive);
            std::thread::Builder::new().name("screamo-render".into()).spawn(move || {
                let _com = ComGuard::init();
                let stream = match unsafe { open_stream(Flow::Render, id.as_deref(), exclusive) } {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = out_info_tx.send(Err(e));
                        return;
                    }
                };
                let _ = out_info_tx.send(Ok(stream.info.clone()));
                let Ok(Some(cb)) = out_cb_rx.recv() else { return };
                if let Err(e) = unsafe { run_render(&stream, &stop, cb) } {
                    fail(&stop, &error, format!("Saída: {e:#}"));
                }
            })?
        };

        let timeout = Duration::from_secs(5);
        let in_info = in_info_rx.recv_timeout(timeout).map_err(|_| anyhow!("entrada não respondeu")).and_then(|r| r.context("abrindo entrada"));
        let out_info = out_info_rx.recv_timeout(timeout).map_err(|_| anyhow!("saída não respondeu")).and_then(|r| r.context("abrindo saída"));

        let (input, output) = match (in_info, out_info) {
            (Ok(i), Ok(o)) => (i, o),
            (i, o) => {
                let _ = in_cb_tx.send(None);
                let _ = out_cb_tx.send(None);
                let _ = capture.join();
                let _ = render.join();
                return Err(i.err().or(o.err()).unwrap());
            }
        };

        let (in_cb, out_cb) = build(&input, &output);
        let _ = in_cb_tx.send(Some(in_cb));
        let _ = out_cb_tx.send(Some(out_cb));

        Ok(Session { stop, threads: vec![capture, render], error, input, output })
    }

    /// Erro que derrubou a sessão (ex.: cabo desconectado), se houver.
    pub fn error(&self) -> Option<String> {
        self.error.lock().clone()
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn fail(stop: &AtomicBool, error: &Mutex<Option<String>>, msg: String) {
    log::warn(&msg);
    error.lock().get_or_insert(msg);
    stop.store(true, Ordering::SeqCst);
}

/// Sobe a prioridade da thread via MMCSS ("Pro Audio"), como DAWs fazem.
struct MmcssGuard(Option<HANDLE>);

impl MmcssGuard {
    fn enter() -> Self {
        let mut task = 0u32;
        let h = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) }.ok();
        crate::dsp::util::enable_flush_denormals();
        Self(h)
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        if let Some(h) = self.0 {
            unsafe {
                let _ = AvRevertMmThreadCharacteristics(h);
            }
        }
    }
}

unsafe fn run_capture(s: &OpenStream, stop: &AtomicBool, mut picker: ChannelPicker, mut cb: InputCallback) -> Result<()> {
    unsafe {
        let _mmcss = MmcssGuard::enter();
        let capture: IAudioCaptureClient = s.client.GetService()?;
        let mut mono = vec![0f32; MAX_FRAMES];
        s.client.Start()?;
        while !stop.load(Ordering::Relaxed) {
            if WaitForSingleObject(s.event, 200) != WAIT_OBJECT_0 {
                continue;
            }
            loop {
                let packet = capture.GetNextPacketSize()?;
                if packet == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let n = (frames as usize).min(MAX_FRAMES);
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                    mono[..n].fill(0.0);
                } else {
                    let bytes = std::slice::from_raw_parts(data, n * s.format.block_align);
                    picker.pick(bytes, &s.format, n, &mut mono);
                }
                capture.ReleaseBuffer(frames)?;
                cb(&mono[..n]);
            }
        }
        Ok(())
    }
}

unsafe fn run_render(s: &OpenStream, stop: &AtomicBool, mut cb: OutputCallback) -> Result<()> {
    unsafe {
        let _mmcss = MmcssGuard::enter();
        let render: IAudioRenderClient = s.client.GetService()?;
        let total = s.buffer_frames;
        let mut stereo = vec![0f32; total as usize * 2];
        let fmt = s.format;
        let bps = sample_bytes(fmt.kind);

        // Pré-carrega silêncio para o engine não começar com lixo.
        let _ = render.GetBuffer(total).and_then(|_| render.ReleaseBuffer(total, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32));
        s.client.Start()?;

        while !stop.load(Ordering::Relaxed) {
            if WaitForSingleObject(s.event, 200) != WAIT_OBJECT_0 {
                continue;
            }
            let frames = if s.exclusive { total } else { total - s.client.GetCurrentPadding()? };
            if frames == 0 {
                continue;
            }
            let n = frames as usize;
            let buf = &mut stereo[..n * 2];
            cb(buf);
            let data = render.GetBuffer(frames)?;
            let bytes = std::slice::from_raw_parts_mut(data, n * fmt.block_align);
            let ch = fmt.channels as usize;
            for f in 0..n {
                let (l, r) = (buf[f * 2], buf[f * 2 + 1]);
                let frame = &mut bytes[f * fmt.block_align..];
                if ch == 1 {
                    write_sample(frame, fmt.kind, 0.5 * (l + r));
                } else {
                    for c in 0..ch {
                        // Canais extras (5.1/7.1) recebem L/R alternados.
                        let v = if c % 2 == 0 { l } else { r };
                        write_sample(&mut frame[c * bps..], fmt.kind, v);
                    }
                }
            }
            render.ReleaseBuffer(frames, 0)?;
        }
        Ok(())
    }
}

mod log {
    pub fn warn(msg: &str) {
        eprintln!("[screamo] {msg}");
    }
}
