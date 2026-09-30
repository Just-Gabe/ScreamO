import type { ReactNode } from 'react';
import { Modal, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { colors, radius } from '../theme';

/** Painel modal genérico usado por configurações, biblioteca e presets. */
export function Sheet({
  visible,
  title,
  onClose,
  children,
  width = 560,
}: {
  visible: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  width?: number;
}) {
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.backdrop} onPress={onClose}>
        <Pressable style={[styles.card, { width }]} onPress={() => {}}>
          <View style={styles.header}>
            <Text style={styles.title}>{title}</Text>
            <Pressable onPress={onClose} style={styles.x}>
              <Text style={styles.xText}>✕</Text>
            </Pressable>
          </View>
          <ScrollView style={styles.body} contentContainerStyle={{ paddingBottom: 8 }}>
            {children}
          </ScrollView>
        </Pressable>
      </Pressable>
    </Modal>
  );
}

export function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <View style={styles.section}>
      <Text style={styles.sectionLabel}>{label}</Text>
      {children}
    </View>
  );
}

export function OptionRow({
  label,
  hint,
  selected,
  onPress,
}: {
  label: string;
  hint?: string;
  selected?: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ hovered }: any) => [styles.option, hovered && styles.optionHover, selected && styles.optionSelected]}
    >
      <View style={[styles.radio, selected && styles.radioOn]} />
      <View style={{ flex: 1 }}>
        <Text style={styles.optionText}>{label}</Text>
        {hint ? <Text style={styles.optionHint}>{hint}</Text> : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  backdrop: { flex: 1, backgroundColor: 'rgba(0,0,0,0.7)', alignItems: 'center', justifyContent: 'center', padding: 16 },
  card: {
    maxWidth: '100%',
    maxHeight: '90%',
    backgroundColor: colors.panel,
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: colors.border,
    overflow: 'hidden',
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 22,
    paddingVertical: 16,
    borderBottomWidth: 1,
    borderBottomColor: colors.border,
  },
  title: { color: colors.text, fontSize: 18, fontWeight: '800' },
  x: { padding: 6 },
  xText: { color: colors.textDim, fontSize: 16 },
  body: { paddingHorizontal: 22, paddingTop: 12 },
  section: { marginBottom: 18 },
  sectionLabel: { color: colors.textDim, fontSize: 11, fontWeight: '800', letterSpacing: 1.5, marginBottom: 8, textTransform: 'uppercase' },
  option: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
    paddingVertical: 10,
    paddingHorizontal: 12,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: 'transparent',
  },
  optionHover: { backgroundColor: colors.panelHi },
  optionSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  radio: { width: 14, height: 14, borderRadius: 7, borderWidth: 2, borderColor: colors.textFaint },
  radioOn: { borderColor: colors.accent, backgroundColor: colors.accent },
  optionText: { color: colors.text, fontSize: 14 },
  optionHint: { color: colors.textDim, fontSize: 12, marginTop: 2 },
});
