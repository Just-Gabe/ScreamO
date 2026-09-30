import { Pressable, StyleSheet, Text, View } from 'react-native';
import { send } from '../engine';
import { categoryLabel, colors, radius } from '../theme';
import type { EffectDescriptor } from '../types';
import { Section, Sheet } from './Sheet';

/** Biblioteca de efeitos, agrupada por categoria (vem do registro do motor). */
export function LibrarySheet({
  effects,
  visible,
  onClose,
}: {
  effects: EffectDescriptor[];
  visible: boolean;
  onClose: () => void;
}) {
  const groups = new Map<string, EffectDescriptor[]>();
  for (const e of effects) {
    groups.set(e.category, [...(groups.get(e.category) ?? []), e]);
  }

  return (
    <Sheet visible={visible} title="Adicionar efeito" onClose={onClose} width={720}>
      {[...groups.entries()].map(([cat, list]) => (
        <Section key={cat} label={categoryLabel[cat] ?? cat}>
          <View style={styles.grid}>
            {list.map((e) => (
              <Pressable
                key={e.id}
                onPress={() => {
                  send({ type: 'addEffect', effectId: e.id });
                  onClose();
                }}
                style={({ hovered }: any) => [styles.card, hovered && { borderColor: e.color }]}
              >
                <View style={[styles.dot, { backgroundColor: e.color }]} />
                <Text style={styles.name}>{e.name}</Text>
                <Text style={styles.desc}>{e.description}</Text>
              </Pressable>
            ))}
          </View>
        </Section>
      ))}
    </Sheet>
  );
}

const styles = StyleSheet.create({
  grid: { flexDirection: 'row', flexWrap: 'wrap', gap: 10 },
  card: {
    width: 206,
    backgroundColor: colors.bgRaised,
    borderRadius: radius.md,
    padding: 14,
    borderWidth: 1,
    borderColor: colors.border,
  },
  dot: { width: 10, height: 10, borderRadius: 5, marginBottom: 8 },
  name: { color: colors.text, fontWeight: '800', fontSize: 14, marginBottom: 4 },
  desc: { color: colors.textDim, fontSize: 12, lineHeight: 16 },
});
