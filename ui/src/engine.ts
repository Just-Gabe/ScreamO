// Cliente WebSocket do motor de áudio. Mantém o estado global da app e um
// store separado para medidores (30 fps) para não re-renderizar a tela toda.

import { useEffect, useSyncExternalStore } from 'react';
import { Platform } from 'react-native';
import type { Action, EngineState, Meters } from './types';

const ENGINE_PORT = 7777;
const EXPO_DEV_PORT = '8081';

function engineUrl(): string {
  if (Platform.OS === 'web' && typeof window !== 'undefined') {
    const { hostname, port, host } = window.location;
    // No dev server do Expo a UI fala com o motor na porta padrão; servida
    // pelo próprio motor (qualquer --port), usa o mesmo host.
    const target = port === EXPO_DEV_PORT ? `${hostname}:${ENGINE_PORT}` : host;
    return `ws://${target}/ws`;
  }
  return `ws://127.0.0.1:${ENGINE_PORT}/ws`;
}

type Listener = () => void;

function createStore<T>(initial: T) {
  let value = initial;
  const listeners = new Set<Listener>();
  return {
    get: () => value,
    set: (v: T) => {
      value = v;
      listeners.forEach((l) => l());
    },
    subscribe: (l: Listener) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
  };
}

export type Connection = 'connecting' | 'open' | 'closed';

const stateStore = createStore<EngineState | null>(null);
const meterStore = createStore<Meters>({
  input: 0,
  output: 0,
  dspLoad: 0,
  underruns: 0,
  overflows: 0,
  tuner: null,
  looper: { state: 'empty', position: 0, length: 0, sampleRate: 48000 },
});
const connStore = createStore<Connection>('connecting');
const errorStore = createStore<{ message: string; at: number } | null>(null);

let socket: WebSocket | null = null;
let started = false;

function connect() {
  connStore.set('connecting');
  const ws = new WebSocket(engineUrl());
  socket = ws;
  ws.onopen = () => connStore.set('open');
  ws.onclose = () => {
    connStore.set('closed');
    socket = null;
    setTimeout(connect, 1000);
  };
  ws.onerror = () => ws.close();
  ws.onmessage = (ev) => {
    const msg = JSON.parse(String(ev.data));
    switch (msg.type) {
      case 'state':
        stateStore.set(msg as EngineState);
        break;
      case 'meters':
        meterStore.set(msg as Meters);
        break;
      case 'param': {
        const s = stateStore.get();
        if (!s) break;
        const chain = s.chain.map((slot) =>
          slot.slotId === msg.slotId
            ? { ...slot, params: slot.params.map((v, i) => (i === msg.index ? msg.value : v)) }
            : slot,
        );
        stateStore.set({ ...s, chain });
        break;
      }
      case 'error':
        errorStore.set({ message: msg.message, at: Date.now() });
        break;
    }
  };
}

export function send(action: Action) {
  if (socket?.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(action));
  }
}

/** Atualização otimista de parâmetro: a UI responde na hora, o motor confirma. */
export function setParam(slotId: number, index: number, value: number) {
  const s = stateStore.get();
  if (s) {
    const chain = s.chain.map((slot) =>
      slot.slotId === slotId ? { ...slot, params: slot.params.map((v, i) => (i === index ? value : v)) } : slot,
    );
    stateStore.set({ ...s, chain });
  }
  send({ type: 'setParam', slotId, index, value });
}

export function useEngine() {
  useEffect(() => {
    if (!started) {
      started = true;
      connect();
    }
  }, []);
  return useSyncExternalStore(stateStore.subscribe, stateStore.get);
}

export const useMeters = () => useSyncExternalStore(meterStore.subscribe, meterStore.get);
export const useConnection = () => useSyncExternalStore(connStore.subscribe, connStore.get);
export const useEngineError = () => useSyncExternalStore(errorStore.subscribe, errorStore.get);
export const clearError = () => errorStore.set(null);
