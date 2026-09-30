import { useState } from 'react';
import { Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { send } from '../engine';
import { colors, radius } from '../theme';
import type { EngineState } from '../types';
import { OptionRow, Section, Sheet } from './Sheet';

export function PresetSheet({ state, visible, onClose }: { state: EngineState; visible: boolean; onClose: () => void }) {
  const [name, setName] = useState('');
  const { factory, user, current } = state.presets;

  const save = () => {
    if (!name.trim()) return;
    send({ type: 'savePreset', name: name.trim() });
    setName('');
  };

  return (
    <Sheet visible={visible} title="Presets" onClose={onClose}>
      <Section label="Salvar som atual">
        <View style={styles.saveRow}>
          <TextInput
            value={name}
            onChangeText={setName}
            onSubmitEditing={save}
            placeholder="Nome do preset"
            placeholderTextColor={colors.textFaint}
            maxLength={48}
            style={styles.input}
          />
          <Pressable onPress={save} style={[styles.saveBtn, !name.trim() && { opacity: 0.4 }]}>
            <Text style={styles.saveText}>Salvar</Text>
          </Pressable>
        </View>
      </Section>

      {user.length > 0 && (
        <Section label="Meus presets">
          {user.map((p) => (
            <View key={p} style={styles.userRow}>
              <View style={{ flex: 1 }}>
                <OptionRow label={p} selected={current === p} onPress={() => send({ type: 'loadPreset', name: p })} />
              </View>
              <Pressable onPress={() => send({ type: 'deletePreset', name: p })} style={styles.del}>
                <Text style={styles.delText}>Excluir</Text>
              </Pressable>
            </View>
          ))}
        </Section>
      )}

      <Section label="Fábrica">
        {factory.map((p) => (
          <OptionRow key={p} label={p} selected={current === p} onPress={() => send({ type: 'loadPreset', name: p })} />
        ))}
      </Section>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  saveRow: { flexDirection: 'row', gap: 10 },
  input: {
    flex: 1,
    backgroundColor: colors.bgRaised,
    borderWidth: 1,
    borderColor: colors.border,
    borderRadius: radius.sm,
    color: colors.text,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 14,
  },
  saveBtn: { backgroundColor: colors.accent, borderRadius: radius.sm, paddingHorizontal: 18, justifyContent: 'center' },
  saveText: { color: '#fff', fontWeight: '800' },
  userRow: { flexDirection: 'row', alignItems: 'center', gap: 6 },
  del: { paddingHorizontal: 10, paddingVertical: 6 },
  delText: { color: colors.danger, fontSize: 12 },
});
