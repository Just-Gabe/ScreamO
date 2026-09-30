import { StyleSheet, Text, View } from 'react-native';
import { useMeters } from '../engine';
import { colors, mono } from '../theme';

const toDb = (v: number) => (v > 0 ? 20 * Math.log10(v) : -120);

/** Barra de nível horizontal (-60..0 dBFS) com indicação de clip. */
function Bar({ label, peak }: { label: string; peak: number }) {
  const db = toDb(peak);
  const pct = Math.max(0, Math.min(1, (db + 60) / 60));
  const color = db > -1 ? colors.danger : db > -9 ? colors.warn : colors.ok;
  return (
    <View style={styles.row}>
      <Text style={styles.label}>{label}</Text>
      <View style={styles.track}>
        <View style={[styles.fill, { width: `${pct * 100}%`, backgroundColor: color }]} />
      </View>
      <Text style={[styles.db, db > -1 && { color: colors.danger }]}>{db < -59 ? '-∞' : db.toFixed(0)}</Text>
    </View>
  );
}

export function LevelMeters() {
  const m = useMeters();
  return (
    <View style={styles.box}>
      <Bar label="IN" peak={m.input} />
      <Bar label="OUT" peak={m.output} />
    </View>
  );
}

export function EngineStats() {
  const m = useMeters();
  const load = Math.round(m.dspLoad * 100);
  return (
    <View style={styles.stats}>
      <Text style={[styles.stat, load > 70 && { color: colors.warn }]}>DSP {load}%</Text>
      <Text style={[styles.stat, m.underruns > 0 && { color: colors.warn }]}>Dropouts {m.underruns}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  box: { gap: 6, width: 220 },
  row: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  label: { color: colors.textDim, fontSize: 10, width: 26, fontWeight: '700' },
  track: { flex: 1, height: 8, backgroundColor: colors.border, borderRadius: 4, overflow: 'hidden' },
  fill: { height: '100%', borderRadius: 4 },
  db: { color: colors.textDim, fontSize: 10, width: 26, textAlign: 'right', fontFamily: mono },
  stats: { gap: 3 },
  stat: { color: colors.textFaint, fontSize: 11, fontFamily: mono },
});
