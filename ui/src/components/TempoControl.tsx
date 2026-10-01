import { useRef, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { send } from '../engine';
import { colors, mono, radius } from '../theme';

const RESET_MS = 2000;
const MAX_TAPS = 5;

/** BPM global: botão Tap (média das últimas batidas) e ajuste fino ±1. */
export function TempoControl({ bpm }: { bpm: number }) {
  const taps = useRef<number[]>([]);
  const [flash, setFlash] = useState(false);

  const tap = () => {
    const now = Date.now();
    const last = taps.current[taps.current.length - 1];
    if (last !== undefined && now - last > RESET_MS) taps.current = [];
    taps.current = [...taps.current, now].slice(-MAX_TAPS);
    setFlash(true);
    setTimeout(() => setFlash(false), 90);
    if (taps.current.length >= 2) {
      const t = taps.current;
      const avg = (t[t.length - 1] - t[0]) / (t.length - 1);
      send({ type: 'setBpm', bpm: Math.round((60000 / avg) * 10) / 10 });
    }
  };

  return (
    <View style={styles.box}>
      <Pressable onPress={tap} style={[styles.tap, flash && styles.tapFlash]}>
        <Text style={styles.tapText}>TAP</Text>
      </Pressable>
      <View>
        <Text style={styles.bpm}>{bpm.toFixed(bpm % 1 === 0 ? 0 : 1)}</Text>
        <Text style={styles.label}>BPM</Text>
      </View>
      <View style={styles.steps}>
        <Step label="+" onPress={() => send({ type: 'setBpm', bpm: Math.round(bpm) + 1 })} />
        <Step label="−" onPress={() => send({ type: 'setBpm', bpm: Math.round(bpm) - 1 })} />
      </View>
    </View>
  );
}

function Step({ label, onPress }: { label: string; onPress: () => void }) {
  return (
    <Pressable onPress={onPress} style={({ hovered }: any) => [styles.step, hovered && { backgroundColor: colors.panelHi }]}>
      <Text style={styles.stepText}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  box: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  tap: {
    width: 46,
    height: 46,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: colors.border,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: colors.panel,
  },
  tapFlash: { backgroundColor: colors.accent, borderColor: colors.accent },
  tapText: { color: colors.text, fontWeight: '900', fontSize: 12, letterSpacing: 1 },
  bpm: { color: colors.text, fontFamily: mono, fontSize: 16, fontWeight: '700', minWidth: 44, textAlign: 'center' },
  label: { color: colors.textDim, fontSize: 9, letterSpacing: 1.2, textAlign: 'center' },
  steps: { gap: 2 },
  step: { paddingHorizontal: 6, borderRadius: 4 },
  stepText: { color: colors.textDim, fontSize: 13, fontWeight: '700' },
});
