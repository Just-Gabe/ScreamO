import { useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { send } from '../engine';
import { colors, radius } from '../theme';
import type { EngineState, MidiTarget } from '../types';
import { OptionRow, Section, Sheet } from './Sheet';

function describe(target: MidiTarget, state: EngineState): string {
  if (target.kind === 'outputVolume') return 'Volume geral';
  const desc = state.effects.find((e) => e.id === target.effectId);
  const name = desc?.name ?? target.effectId;
  if (target.kind === 'bypass') return `${name} · liga/desliga`;
  const param = desc?.params.find((p) => p.id === target.paramId);
  return `${name} · ${param?.name ?? target.paramId}`;
}

export function MidiSheet({ state, visible, onClose }: { state: EngineState; visible: boolean; onClose: () => void }) {
  const [effectId, setEffectId] = useState<string | null>(null);
  const { midi, settings } = state;
  const chainEffects = [...new Set(state.chain.map((s) => s.effectId))]
    .map((id) => state.effects.find((e) => e.id === id))
    .filter((e) => !!e);
  const picked = chainEffects.find((e) => e.id === effectId);
  const learn = (target: MidiTarget) => {
    send({ type: 'midiLearn', target });
    setEffectId(null);
  };

  return (
    <Sheet visible={visible} title="MIDI" onClose={onClose} width={620}>
      <Section label="Dispositivos">
        {midi.devices.length === 0 ? (
          <Text style={styles.dim}>Nenhum controlador MIDI conectado. Conecte por USB; ele aparece aqui sozinho.</Text>
        ) : (
          midi.devices.map((d) => (
            <Text key={d} style={styles.device}>
              ● {d}
            </Text>
          ))
        )}
      </Section>

      {midi.learning && (
        <View style={styles.learning}>
          <Text style={styles.learningText}>Mexa no pedal/botão MIDI agora para ligar a: {describe(midi.learning, state)}</Text>
          <Pressable onPress={() => send({ type: 'midiLearn', target: null })} style={styles.cancel}>
            <Text style={styles.cancelText}>Cancelar</Text>
          </Pressable>
        </View>
      )}

      <Section label="Mapeamentos">
        {settings.midiBindings.length === 0 && <Text style={styles.dim}>Nenhum ainda.</Text>}
        {settings.midiBindings.map((b, i) => (
          <View key={`${b.cc}-${i}`} style={styles.binding}>
            <Text style={styles.cc}>CC {b.cc}</Text>
            <Text style={styles.target}>{describe(b.target, state)}</Text>
            <Pressable onPress={() => send({ type: 'midiUnbind', index: i })}>
              <Text style={styles.remove}>Remover</Text>
            </Pressable>
          </View>
        ))}
      </Section>

      <Section label="Novo mapeamento (MIDI Learn)">
        {!picked ? (
          <View style={styles.grid}>
            <Chip label="Volume geral" onPress={() => learn({ kind: 'outputVolume' })} />
            {chainEffects.map((e) => (
              <Chip key={e.id} label={e.name} color={e.color} onPress={() => setEffectId(e.id)} />
            ))}
          </View>
        ) : (
          <View>
            <Pressable onPress={() => setEffectId(null)}>
              <Text style={styles.back}>‹ {picked.name}</Text>
            </Pressable>
            <OptionRow label="Liga/desliga (footswitch)" hint="Alterna a cada pisada" onPress={() => learn({ kind: 'bypass', effectId: picked.id })} />
            {picked.params
              .filter((p) => p.kind !== 'actions')
              .map((p) => (
                <OptionRow
                  key={p.id}
                  label={p.name}
                  hint={p.kind === 'knob' ? 'Pedal de expressão ou knob' : 'Seletor'}
                  onPress={() => learn({ kind: 'param', effectId: picked.id, paramId: p.id })}
                />
              ))}
          </View>
        )}
      </Section>

      <Section label="Troca de preset">
        <OptionRow
          label="Program Change carrega presets"
          hint="PC 0 = primeiro preset de fábrica, depois os seus, na ordem da lista"
          selected={settings.midiProgramChange}
          onPress={() => send({ type: 'setMidiProgramChange', enabled: !settings.midiProgramChange })}
        />
      </Section>
    </Sheet>
  );
}

function Chip({ label, color, onPress }: { label: string; color?: string; onPress: () => void }) {
  return (
    <Pressable onPress={onPress} style={({ hovered }: any) => [styles.chip, hovered && { borderColor: color ?? colors.accent }]}>
      {color && <View style={[styles.dot, { backgroundColor: color }]} />}
      <Text style={styles.chipText}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  dim: { color: colors.textDim, fontSize: 13 },
  device: { color: colors.ok, fontSize: 13, marginBottom: 4 },
  learning: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 10,
    padding: 12,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: colors.warn,
    marginBottom: 16,
  },
  learningText: { color: colors.warn, flex: 1, fontWeight: '700' },
  cancel: { paddingHorizontal: 12, paddingVertical: 6, borderRadius: 999, backgroundColor: colors.panelHi },
  cancelText: { color: colors.text, fontWeight: '700', fontSize: 12 },
  binding: { flexDirection: 'row', alignItems: 'center', gap: 12, paddingVertical: 8 },
  cc: { color: colors.accent, fontWeight: '800', width: 56 },
  target: { color: colors.text, flex: 1 },
  remove: { color: colors.danger, fontSize: 12 },
  grid: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.bgRaised,
  },
  dot: { width: 8, height: 8, borderRadius: 4 },
  chipText: { color: colors.text, fontWeight: '700', fontSize: 12 },
  back: { color: colors.accent, fontWeight: '800', marginBottom: 8 },
});
