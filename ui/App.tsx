import { StatusBar } from 'expo-status-bar';
import { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { Knob } from './src/components/Knob';
import { LibrarySheet } from './src/components/LibrarySheet';
import { EngineStats, LevelMeters } from './src/components/Meters';
import { Pedal } from './src/components/Pedal';
import { PresetSheet } from './src/components/PresetSheet';
import { SettingsSheet } from './src/components/SettingsSheet';
import { Tuner } from './src/components/Tuner';
import { clearError, send, useConnection, useEngine, useEngineError } from './src/engine';
import { colors, radius } from './src/theme';

type Panel = 'none' | 'settings' | 'library' | 'presets' | 'tuner';

export default function App() {
  const state = useEngine();
  const conn = useConnection();
  const [panel, setPanel] = useState<Panel>('none');
  const tunerPrevMute = useRef(false);

  if (!state) {
    return (
      <View style={[styles.root, styles.center]}>
        <StatusBar style="light" />
        <Logo size={44} />
        <ActivityIndicator color={colors.accent} style={{ marginTop: 24 }} />
        <Text style={styles.waiting}>
          {conn === 'closed' ? 'Motor de áudio não encontrado. Abra o screamo.exe.' : 'Conectando ao motor de áudio...'}
        </Text>
      </View>
    );
  }

  const { settings, audio } = state;
  const descById = new Map(state.effects.map((e) => [e.id, e]));
  const close = () => setPanel('none');

  const openTuner = () => {
    tunerPrevMute.current = settings.muted;
    send({ type: 'setTuner', enabled: true });
    send({ type: 'setMute', muted: true });
    setPanel('tuner');
  };
  const closeTuner = () => {
    send({ type: 'setTuner', enabled: false });
    send({ type: 'setMute', muted: tunerPrevMute.current });
    close();
  };

  const muted = settings.muted || state.safetyMute;

  return (
    <View style={styles.root}>
      <StatusBar style="light" />

      {/* Barra superior */}
      <View style={styles.topbar}>
        <Logo size={26} />
        <Pressable onPress={() => setPanel('presets')} style={styles.presetBtn}>
          <Text style={styles.presetLabel}>PRESET</Text>
          <Text style={styles.presetName} numberOfLines={1}>
            {state.presets.current ?? 'Personalizado'} ▾
          </Text>
        </Pressable>

        <Pressable onPress={() => setPanel('settings')} style={styles.status}>
          <View style={[styles.statusDot, { backgroundColor: audio.running ? colors.ok : colors.danger }]} />
          <View>
            <Text style={styles.statusText} numberOfLines={1}>
              {audio.running ? `${audio.input?.deviceName ?? ''}` : 'Áudio parado'}
            </Text>
            <Text style={styles.statusSub} numberOfLines={1}>
              {audio.running
                ? `~${audio.latencyMs.toFixed(1)} ms · ${audio.input?.sampleRate ?? 0} Hz · ${audio.input?.mode}`
                : 'Clique para configurar'}
            </Text>
          </View>
        </Pressable>

        <View style={{ flex: 1 }} />
        <LevelMeters />
        <Knob
          label="Input"
          value={settings.inputGainDb}
          min={-24}
          max={24}
          defaultValue={0}
          unit="dB"
          size={46}
          onChange={(db) => send({ type: 'setInputGain', db })}
        />
        <Knob
          label="Volume"
          value={settings.outputGainDb}
          min={-60}
          max={12}
          defaultValue={-6}
          unit="dB"
          size={46}
          onChange={(db) => send({ type: 'setOutputGain', db })}
        />
        <TopButton label="Afinador" onPress={openTuner} />
        <TopButton label={muted ? 'Mudo' : 'Som on'} active={!muted} danger={muted} onPress={() => send({ type: 'setMute', muted: !muted })} />
        <TopButton label="Áudio" onPress={() => setPanel('settings')} />
      </View>

      {/* Avisos */}
      {conn !== 'open' && <Banner tone="danger" text="Conexão com o motor perdida. Tentando reconectar..." />}
      {audio.error && (
        <Banner tone="danger" text={audio.error} action="Configurar áudio" onAction={() => setPanel('settings')} />
      )}
      {state.safetyMute && audio.running && (
        <Banner
          tone="warn"
          text="Nenhum cabo de guitarra detectado; o som está mudo para evitar microfonia. Escolha a entrada da guitarra."
          action="Escolher entrada"
          onAction={() => setPanel('settings')}
        />
      )}
      <ErrorToast />

      {/* Pedalboard */}
      <ScrollView horizontal style={styles.board} contentContainerStyle={styles.boardContent}>
        <Endpoint label="GUITARRA" sub={audio.input?.deviceName} />
        {state.chain.map((slot, i) => {
          const desc = descById.get(slot.effectId);
          if (!desc) return null;
          return (
            <View key={slot.slotId} style={styles.flow}>
              <Cable />
              <Pedal slot={slot} desc={desc} index={i} count={state.chain.length} />
            </View>
          );
        })}
        <Cable />
        <Pressable onPress={() => setPanel('library')} style={({ hovered }: any) => [styles.add, hovered && styles.addHover]}>
          <Text style={styles.addPlus}>+</Text>
          <Text style={styles.addText}>Adicionar efeito</Text>
        </Pressable>
        <Cable />
        <Endpoint label="FONE" sub={audio.output?.deviceName} />
      </ScrollView>

      <View style={styles.footer}>
        <Text style={styles.hint}>Arraste os knobs para cima/baixo · duplo clique volta ao padrão · ◀ ▶ reordena</Text>
        <EngineStats />
      </View>

      <SettingsSheet state={state} visible={panel === 'settings'} onClose={close} />
      <LibrarySheet effects={state.effects} visible={panel === 'library'} onClose={close} />
      <PresetSheet state={state} visible={panel === 'presets'} onClose={close} />
      <Tuner visible={panel === 'tuner'} onClose={closeTuner} />
    </View>
  );
}

function Logo({ size }: { size: number }) {
  return (
    <Text style={[styles.logo, { fontSize: size }]}>
      Scream<Text style={{ color: colors.accent }}>O</Text>
    </Text>
  );
}

function TopButton({ label, onPress, active, danger }: { label: string; onPress: () => void; active?: boolean; danger?: boolean }) {
  return (
    <Pressable
      onPress={onPress}
      style={({ hovered }: any) => [
        styles.topBtn,
        hovered && { backgroundColor: colors.panelHi },
        active && { borderColor: colors.ok },
        danger && { borderColor: colors.danger, backgroundColor: 'rgba(255,77,77,0.12)' },
      ]}
    >
      <Text style={styles.topBtnText}>{label}</Text>
    </Pressable>
  );
}

function Banner({ tone, text, action, onAction }: { tone: 'warn' | 'danger'; text: string; action?: string; onAction?: () => void }) {
  const c = tone === 'warn' ? colors.warn : colors.danger;
  return (
    <View style={[styles.banner, { borderColor: c }]}>
      <Text style={[styles.bannerText, { color: c }]}>{text}</Text>
      {action && (
        <Pressable onPress={onAction} style={[styles.bannerBtn, { backgroundColor: c }]}>
          <Text style={styles.bannerBtnText}>{action}</Text>
        </Pressable>
      )}
    </View>
  );
}

function ErrorToast() {
  const err = useEngineError();
  useEffect(() => {
    if (!err) return;
    const t = setTimeout(clearError, 4000);
    return () => clearTimeout(t);
  }, [err]);
  if (!err) return null;
  return <Banner tone="danger" text={err.message} action="OK" onAction={clearError} />;
}

function Endpoint({ label, sub }: { label: string; sub?: string }) {
  return (
    <View style={styles.endpoint}>
      <Text style={styles.endpointLabel}>{label}</Text>
      {sub ? (
        <Text style={styles.endpointSub} numberOfLines={3}>
          {sub}
        </Text>
      ) : null}
    </View>
  );
}

function Cable() {
  return <View style={styles.cable} />;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  center: { alignItems: 'center', justifyContent: 'center' },
  waiting: { color: colors.textDim, marginTop: 14 },
  logo: { color: colors.text, fontWeight: '900', letterSpacing: -1, fontStyle: 'italic' },
  topbar: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 16,
    paddingHorizontal: 20,
    paddingVertical: 12,
    backgroundColor: colors.bgRaised,
    borderBottomWidth: 1,
    borderBottomColor: colors.border,
    flexWrap: 'wrap',
  },
  presetBtn: {
    backgroundColor: colors.panel,
    borderRadius: radius.md,
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderWidth: 1,
    borderColor: colors.border,
    minWidth: 170,
  },
  presetLabel: { color: colors.textFaint, fontSize: 9, fontWeight: '800', letterSpacing: 1.5 },
  presetName: { color: colors.text, fontWeight: '800', fontSize: 15 },
  status: { flexDirection: 'row', alignItems: 'center', gap: 10, maxWidth: 320 },
  statusDot: { width: 10, height: 10, borderRadius: 5 },
  statusText: { color: colors.text, fontSize: 13, fontWeight: '600' },
  statusSub: { color: colors.textDim, fontSize: 11 },
  topBtn: {
    paddingHorizontal: 14,
    paddingVertical: 9,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: colors.border,
  },
  topBtnText: { color: colors.text, fontWeight: '700', fontSize: 13 },
  banner: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
    marginHorizontal: 20,
    marginTop: 12,
    padding: 12,
    borderRadius: radius.md,
    borderWidth: 1,
    backgroundColor: colors.panel,
  },
  bannerText: { flex: 1, fontSize: 13, fontWeight: '600' },
  bannerBtn: { paddingHorizontal: 12, paddingVertical: 6, borderRadius: 999 },
  bannerBtnText: { color: '#000', fontWeight: '800', fontSize: 12 },
  board: { flex: 1 },
  boardContent: { alignItems: 'center', paddingHorizontal: 24, paddingVertical: 24 },
  flow: { flexDirection: 'row', alignItems: 'center' },
  cable: { width: 26, height: 3, backgroundColor: colors.border, borderRadius: 2 },
  endpoint: {
    width: 110,
    paddingVertical: 18,
    paddingHorizontal: 10,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: colors.border,
    borderStyle: 'dashed',
    alignItems: 'center',
  },
  endpointLabel: { color: colors.textDim, fontWeight: '900', letterSpacing: 2, fontSize: 11 },
  endpointSub: { color: colors.textFaint, fontSize: 10, textAlign: 'center', marginTop: 6 },
  add: {
    width: 150,
    height: 200,
    borderRadius: radius.lg,
    borderWidth: 2,
    borderStyle: 'dashed',
    borderColor: colors.border,
    alignItems: 'center',
    justifyContent: 'center',
  },
  addHover: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  addPlus: { color: colors.accent, fontSize: 40, fontWeight: '300' },
  addText: { color: colors.textDim, fontWeight: '700', marginTop: 4 },
  footer: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 20,
    paddingVertical: 10,
    borderTopWidth: 1,
    borderTopColor: colors.border,
  },
  hint: { color: colors.textFaint, fontSize: 12 },
});
