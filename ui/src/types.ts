// Espelho dos tipos que o motor (engine/src) envia por WebSocket.

export type ParamKind =
  | { kind: 'knob'; log: boolean }
  | { kind: 'choice'; options: string[] }
  | { kind: 'toggle' };

export type ParamSpec = {
  id: string;
  name: string;
  min: number;
  max: number;
  default: number;
  unit: string;
} & ParamKind;

export type Category = 'dynamics' | 'drive' | 'amp' | 'cabinet' | 'eq' | 'modulation' | 'time';

export type EffectDescriptor = {
  id: string;
  name: string;
  category: Category;
  description: string;
  color: string;
  params: ParamSpec[];
};

export type SlotState = {
  slotId: number;
  effectId: string;
  params: number[];
  bypass: boolean;
};

export type DeviceInfo = { id: string; name: string; isDefault: boolean };

export type StreamInfo = {
  deviceName: string;
  sampleRate: number;
  channels: number;
  format: string;
  periodFrames: number;
  bufferFrames: number;
  deviceLatencyFrames: number;
  mode: string;
};

export type InputChannel = 'auto' | 'left' | 'right' | 'mix';

export type Settings = {
  inputDevice: string | null;
  outputDevice: string | null;
  exclusive: boolean;
  inputChannel: InputChannel;
  safetyMs: number;
  inputGainDb: number;
  outputGainDb: number;
  muted: boolean;
};

export type EngineState = {
  settings: Settings;
  audio: {
    running: boolean;
    error: string | null;
    input: StreamInfo | null;
    output: StreamInfo | null;
    latencyMs: number;
  };
  devices: { inputs: DeviceInfo[]; outputs: DeviceInfo[]; guessedInput: string | null };
  effects: EffectDescriptor[];
  chain: SlotState[];
  presets: { factory: string[]; user: string[]; current: string | null };
  tunerEnabled: boolean;
  safetyMute: boolean;
};

export type TunerReading = {
  active: boolean;
  frequency: number;
  note: string;
  octave: number;
  cents: number;
};

export type Meters = {
  input: number;
  output: number;
  dspLoad: number;
  underruns: number;
  overflows: number;
  tuner: TunerReading | null;
};

export type Action =
  | { type: 'getState' }
  | { type: 'refreshDevices' }
  | { type: 'setDevices'; inputDevice: string | null; outputDevice: string | null }
  | { type: 'setAudioOptions'; exclusive?: boolean; inputChannel?: InputChannel; safetyMs?: number }
  | { type: 'restartAudio' }
  | { type: 'setInputGain'; db: number }
  | { type: 'setOutputGain'; db: number }
  | { type: 'setMute'; muted: boolean }
  | { type: 'setTuner'; enabled: boolean }
  | { type: 'addEffect'; effectId: string; index?: number }
  | { type: 'removeEffect'; slotId: number }
  | { type: 'moveEffect'; slotId: number; to: number }
  | { type: 'setParam'; slotId: number; index: number; value: number }
  | { type: 'setBypass'; slotId: number; bypass: boolean }
  | { type: 'loadPreset'; name: string }
  | { type: 'savePreset'; name: string }
  | { type: 'deletePreset'; name: string };
