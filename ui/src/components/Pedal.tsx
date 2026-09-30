import { memo } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { send, setParam } from '../engine';
import { colors, radius } from '../theme';
import type { EffectDescriptor, SlotState } from '../types';
import { Knob } from './Knob';

type Props = {
  slot: SlotState;
  desc: EffectDescriptor;
  index: number;
  count: number;
};

function PedalImpl({ slot, desc, index, count }: Props) {
  const on = !slot.bypass;
  const knobs = desc.params.map((p, i) => ({ p, i })).filter(({ p }) => p.kind === 'knob');
  const choices = desc.params.map((p, i) => ({ p, i })).filter(({ p }) => p.kind !== 'knob');

  return (
    <View style={[styles.pedal, { borderColor: on ? desc.color : colors.border }, !on && styles.off]}>
      <View style={[styles.stripe, { backgroundColor: desc.color }]} />
      <View style={styles.header}>
        <Text style={styles.name} numberOfLines={1}>
          {desc.name}
        </Text>
        <View style={styles.tools}>
          <Tool label="◀" disabled={index === 0} onPress={() => send({ type: 'moveEffect', slotId: slot.slotId, to: index - 1 })} />
          <Tool label="▶" disabled={index === count - 1} onPress={() => send({ type: 'moveEffect', slotId: slot.slotId, to: index + 1 })} />
          <Tool label="✕" onPress={() => send({ type: 'removeEffect', slotId: slot.slotId })} />
        </View>
      </View>

      {choices.map(({ p, i }) => (
        <View key={p.id} style={styles.choiceRow}>
          {p.kind === 'choice' &&
            p.options.map((opt, oi) => {
              const active = Math.round(slot.params[i]) === oi;
              return (
                <Pressable
                  key={opt}
                  onPress={() => setParam(slot.slotId, i, oi)}
                  style={[styles.choice, active && { backgroundColor: desc.color, borderColor: desc.color }]}
                >
                  <Text style={[styles.choiceText, active && { color: '#000' }]}>{opt}</Text>
                </Pressable>
              );
            })}
          {p.kind === 'toggle' && (
            <Pressable
              onPress={() => setParam(slot.slotId, i, slot.params[i] >= 0.5 ? 0 : 1)}
              style={[styles.choice, slot.params[i] >= 0.5 && { backgroundColor: desc.color }]}
            >
              <Text style={styles.choiceText}>{p.name}</Text>
            </Pressable>
          )}
        </View>
      ))}

      <View style={styles.knobs}>
        {knobs.map(({ p, i }) => (
          <Knob
            key={p.id}
            label={p.name}
            value={slot.params[i]}
            min={p.min}
            max={p.max}
            defaultValue={p.default}
            unit={p.unit}
            log={p.kind === 'knob' && p.log}
            color={desc.color}
            onChange={(v) => setParam(slot.slotId, i, v)}
          />
        ))}
      </View>

      <Pressable
        onPress={() => send({ type: 'setBypass', slotId: slot.slotId, bypass: on })}
        style={({ pressed }) => [styles.footswitch, pressed && { transform: [{ scale: 0.96 }] }]}
      >
        <View style={[styles.led, on && { backgroundColor: desc.color, shadowColor: desc.color }]} />
        <Text style={styles.footText}>{on ? 'ON' : 'BYPASS'}</Text>
      </Pressable>
    </View>
  );
}

function Tool({ label, onPress, disabled }: { label: string; onPress: () => void; disabled?: boolean }) {
  return (
    <Pressable onPress={onPress} disabled={disabled} style={({ hovered }: any) => [styles.tool, hovered && styles.toolHover, disabled && { opacity: 0.25 }]}>
      <Text style={styles.toolText}>{label}</Text>
    </Pressable>
  );
}

export const Pedal = memo(PedalImpl);

const styles = StyleSheet.create({
  pedal: {
    width: 252,
    backgroundColor: colors.panel,
    borderRadius: radius.lg,
    borderWidth: 1.5,
    padding: 14,
    paddingTop: 18,
    overflow: 'hidden',
    justifyContent: 'space-between',
  },
  off: { opacity: 0.55 },
  stripe: { position: 'absolute', top: 0, left: 0, right: 0, height: 5 },
  header: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 },
  name: { color: colors.text, fontWeight: '800', fontSize: 16, flex: 1, letterSpacing: 0.3 },
  tools: { flexDirection: 'row', gap: 2 },
  tool: { paddingHorizontal: 6, paddingVertical: 3, borderRadius: 6 },
  toolHover: { backgroundColor: colors.panelHi },
  toolText: { color: colors.textDim, fontSize: 12 },
  choiceRow: { flexDirection: 'row', flexWrap: 'wrap', gap: 6, marginBottom: 10 },
  choice: {
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.bgRaised,
  },
  choiceText: { color: colors.text, fontSize: 11, fontWeight: '700' },
  knobs: { flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'center', rowGap: 10, marginBottom: 12 },
  footswitch: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 10,
    backgroundColor: colors.bgRaised,
    borderRadius: radius.md,
    paddingVertical: 12,
    borderWidth: 1,
    borderColor: colors.border,
  },
  led: {
    width: 12,
    height: 12,
    borderRadius: 6,
    backgroundColor: '#301018',
    shadowOpacity: 0.9,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 0 },
  },
  footText: { color: colors.text, fontWeight: '800', letterSpacing: 2, fontSize: 12 },
});
