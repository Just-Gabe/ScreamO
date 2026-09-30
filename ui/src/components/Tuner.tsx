import { Modal, Pressable, StyleSheet, Text, View } from 'react-native';
import { useMeters } from '../engine';
import { colors, mono, radius } from '../theme';

/** Afinador em tela cheia; silencia a saída enquanto aberto. */
export function Tuner({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { tuner } = useMeters();
  const active = !!tuner?.active;
  const cents = active ? tuner!.cents : 0;
  const inTune = active && Math.abs(cents) < 4;
  const color = !active ? colors.textFaint : inTune ? colors.ok : Math.abs(cents) < 15 ? colors.warn : colors.danger;

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.backdrop} onPress={onClose}>
        <Pressable style={styles.card} onPress={() => {}}>
          <Text style={styles.title}>AFINADOR</Text>
          <Text style={[styles.note, { color }]}>
            {active ? tuner!.note : '—'}
            {active && <Text style={styles.octave}>{tuner!.octave}</Text>}
          </Text>
          <View style={styles.scale}>
            {Array.from({ length: 21 }, (_, i) => i - 10).map((t) => (
              <View key={t} style={[styles.tick, t === 0 && styles.tickCenter, t % 5 === 0 && styles.tickMajor]} />
            ))}
            <View style={[styles.needle, { left: `${50 + Math.max(-50, Math.min(50, cents))}%`, backgroundColor: color }]} />
          </View>
          <Text style={styles.readout}>
            {active ? `${tuner!.frequency.toFixed(1)} Hz · ${cents > 0 ? '+' : ''}${cents.toFixed(0)} cents` : 'Toque uma corda solta'}
          </Text>
          <Pressable style={styles.close} onPress={onClose}>
            <Text style={styles.closeText}>Fechar</Text>
          </Pressable>
        </Pressable>
      </Pressable>
    </Modal>
  );
}

const styles = StyleSheet.create({
  backdrop: { flex: 1, backgroundColor: 'rgba(0,0,0,0.75)', alignItems: 'center', justifyContent: 'center' },
  card: { backgroundColor: colors.panel, borderRadius: radius.lg, padding: 32, width: 440, alignItems: 'center', borderWidth: 1, borderColor: colors.border },
  title: { color: colors.textDim, letterSpacing: 4, fontWeight: '800', fontSize: 12 },
  note: { fontSize: 110, fontWeight: '900', marginVertical: 4 },
  octave: { fontSize: 32, color: colors.textDim },
  scale: { width: '100%', height: 44, flexDirection: 'row', justifyContent: 'space-between', alignItems: 'flex-end', marginVertical: 12 },
  tick: { width: 2, height: 12, backgroundColor: colors.border },
  tickMajor: { height: 20, backgroundColor: colors.textFaint },
  tickCenter: { height: 32, backgroundColor: colors.ok },
  needle: { position: 'absolute', bottom: 0, width: 4, height: 44, marginLeft: -2, borderRadius: 2 },
  readout: { color: colors.textDim, fontFamily: mono, fontSize: 13 },
  close: { marginTop: 22, paddingHorizontal: 22, paddingVertical: 10, borderRadius: 999, backgroundColor: colors.panelHi },
  closeText: { color: colors.text, fontWeight: '700' },
});
