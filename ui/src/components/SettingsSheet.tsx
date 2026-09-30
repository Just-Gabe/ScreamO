import { Pressable, StyleSheet, Text, View } from 'react-native';
import { send } from '../engine';
import { colors, mono, radius } from '../theme';
import type { EngineState, InputChannel } from '../types';
import { OptionRow, Section, Sheet } from './Sheet';

const CHANNELS: { id: InputChannel; label: string; hint: string }[] = [
  { id: 'auto', label: 'Automático', hint: 'Usa o canal com mais sinal (recomendado)' },
  { id: 'left', label: 'Esquerdo', hint: 'Canal 1' },
  { id: 'right', label: 'Direito', hint: 'Canal 2' },
  { id: 'mix', label: 'Mistura', hint: 'Média dos dois canais' },
];

const SAFETY = [0, 1, 2, 4, 8, 15];

export function SettingsSheet({ state, visible, onClose }: { state: EngineState; visible: boolean; onClose: () => void }) {
  const { settings, devices, audio } = state;
  const effectiveInput = settings.inputDevice ?? devices.guessedInput;

  return (
    <Sheet visible={visible} title="Áudio" onClose={onClose} width={620}>
      <Section label="Entrada (cabo da guitarra)">
        {devices.inputs.length === 0 && <Text style={styles.empty}>Nenhuma entrada encontrada. Conecte o cabo USB.</Text>}
        {devices.inputs.map((d) => (
          <OptionRow
            key={d.id}
            label={d.name}
            hint={[d.isDefault && 'padrão do Windows', d.id === devices.guessedInput && 'parece um cabo de guitarra']
              .filter(Boolean)
              .join(' · ')}
            selected={effectiveInput === d.id}
            onPress={() => send({ type: 'setDevices', inputDevice: d.id, outputDevice: settings.outputDevice })}
          />
        ))}
      </Section>

      <Section label="Saída (fone)">
        <OptionRow
          label="Padrão do Windows"
          hint="Segue o dispositivo padrão"
          selected={settings.outputDevice === null}
          onPress={() => send({ type: 'setDevices', inputDevice: settings.inputDevice, outputDevice: null })}
        />
        {devices.outputs.map((d) => (
          <OptionRow
            key={d.id}
            label={d.name}
            hint={d.isDefault ? 'padrão do Windows' : undefined}
            selected={settings.outputDevice === d.id}
            onPress={() => send({ type: 'setDevices', inputDevice: settings.inputDevice, outputDevice: d.id })}
          />
        ))}
      </Section>

      <Section label="Canal de entrada">
        <View style={styles.pills}>
          {CHANNELS.map((c) => (
            <Pill
              key={c.id}
              label={c.label}
              active={settings.inputChannel === c.id}
              onPress={() => send({ type: 'setAudioOptions', inputChannel: c.id })}
            />
          ))}
        </View>
      </Section>

      <Section label="Latência">
        <OptionRow
          label="Modo exclusivo"
          hint="Latência mínima absoluta, mas outros apps ficam sem som nesse dispositivo enquanto o ScreamO estiver aberto."
          selected={settings.exclusive}
          onPress={() => send({ type: 'setAudioOptions', exclusive: !settings.exclusive })}
        />
        <Text style={styles.sub}>Buffer de segurança (aumente se ouvir estalos)</Text>
        <View style={styles.pills}>
          {SAFETY.map((ms) => (
            <Pill
              key={ms}
              label={`${ms} ms`}
              active={Math.abs(settings.safetyMs - ms) < 0.1}
              onPress={() => send({ type: 'setAudioOptions', safetyMs: ms })}
            />
          ))}
        </View>
      </Section>

      <Section label="Status">
        <View style={styles.status}>
          {audio.input && <Info label="Entrada" value={`${audio.input.deviceName}\n${audio.input.format} · ${audio.input.mode} · período ${audio.input.periodFrames}`} />}
          {audio.output && <Info label="Saída" value={`${audio.output.deviceName}\n${audio.output.format} · ${audio.output.mode} · buffer ${audio.output.bufferFrames}`} />}
          {audio.running && <Info label="Latência" value={`~${audio.latencyMs.toFixed(1)} ms ida e volta (estimada)`} />}
          {audio.error && <Text style={styles.error}>{audio.error}</Text>}
        </View>
        <View style={[styles.pills, { marginTop: 12 }]}>
          <Pill label="Reiniciar áudio" onPress={() => send({ type: 'restartAudio' })} />
          <Pill label="Procurar dispositivos" onPress={() => send({ type: 'refreshDevices' })} />
        </View>
      </Section>
    </Sheet>
  );
}

function Pill({ label, active, onPress }: { label: string; active?: boolean; onPress: () => void }) {
  return (
    <Pressable onPress={onPress} style={[styles.pill, active && styles.pillActive]}>
      <Text style={[styles.pillText, active && { color: '#fff' }]}>{label}</Text>
    </Pressable>
  );
}

function Info({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.info}>
      <Text style={styles.infoLabel}>{label}</Text>
      <Text style={styles.infoValue}>{value}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  empty: { color: colors.warn, fontSize: 13 },
  pills: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  pill: {
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderRadius: 999,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.bgRaised,
  },
  pillActive: { backgroundColor: colors.accent, borderColor: colors.accent },
  pillText: { color: colors.textDim, fontWeight: '700', fontSize: 12 },
  sub: { color: colors.textDim, fontSize: 12, marginTop: 12, marginBottom: 8 },
  status: { gap: 10, backgroundColor: colors.bgRaised, borderRadius: radius.md, padding: 14 },
  info: { flexDirection: 'row', gap: 12 },
  infoLabel: { color: colors.textDim, width: 70, fontSize: 12 },
  infoValue: { color: colors.text, flex: 1, fontSize: 12, fontFamily: mono },
  error: { color: colors.danger, fontSize: 13 },
});
