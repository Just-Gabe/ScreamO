import { useState } from 'react';
import { Platform, Pressable, StyleSheet, Text, View } from 'react-native';
import { send } from '../engine';
import { colors, radius } from '../theme';
import type { FileKind } from '../types';
import { OptionRow, Section, Sheet } from './Sheet';

const LABEL: Record<FileKind, { title: string; ext: string; hint: string }> = {
  ir: { title: 'IR de caixa', ext: '.wav', hint: 'Respostas a impulso (.wav) de caixas reais. Muitas são gratuitas.' },
  nam: { title: 'Captura NAM', ext: '.nam', hint: 'Capturas de amps e pedais reais. Milhares grátis em tonehunt.org.' },
};

/** Abre o seletor de arquivos do sistema e envia o arquivo em base64. */
function pickAndUpload(kind: FileKind, slotId: number, onStart: () => void) {
  if (Platform.OS !== 'web' || typeof document === 'undefined') return;
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = LABEL[kind].ext;
  input.onchange = () => {
    const file = input.files?.[0];
    if (!file) return;
    onStart();
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      const data = url.slice(url.indexOf(',') + 1);
      send({ type: 'uploadFile', name: file.name, data, slotId });
    };
    reader.readAsDataURL(file);
  };
  input.click();
}

export function FileSheet({
  kind,
  slotId,
  current,
  files,
  visible,
  onClose,
}: {
  kind: FileKind;
  slotId: number;
  current: string | null;
  files: string[];
  visible: boolean;
  onClose: () => void;
}) {
  const [uploading, setUploading] = useState(false);
  const info = LABEL[kind];

  return (
    <Sheet visible={visible} title={info.title} onClose={onClose}>
      <Text style={styles.hint}>{info.hint}</Text>
      <Pressable
        style={styles.upload}
        onPress={() =>
          pickAndUpload(kind, slotId, () => {
            setUploading(true);
            setTimeout(() => {
              setUploading(false);
              onClose();
            }, 600);
          })
        }
      >
        <Text style={styles.uploadText}>{uploading ? 'Enviando...' : `Carregar arquivo ${info.ext} do computador`}</Text>
      </Pressable>

      <Section label="Biblioteca">
        {files.length === 0 && <Text style={styles.empty}>Nenhum arquivo ainda.</Text>}
        <OptionRow label="Nenhum (som passa direto)" selected={current === null} onPress={() => send({ type: 'setFile', slotId, file: null })} />
        {files.map((f) => (
          <View key={f} style={styles.row}>
            <View style={{ flex: 1 }}>
              <OptionRow label={f} selected={current === f} onPress={() => send({ type: 'setFile', slotId, file: f })} />
            </View>
            <Pressable onPress={() => send({ type: 'deleteFile', name: f })} style={styles.del}>
              <Text style={styles.delText}>Excluir</Text>
            </Pressable>
          </View>
        ))}
      </Section>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  hint: { color: colors.textDim, fontSize: 13, marginBottom: 12 },
  upload: {
    backgroundColor: colors.accent,
    borderRadius: radius.md,
    paddingVertical: 12,
    alignItems: 'center',
    marginBottom: 18,
  },
  uploadText: { color: '#fff', fontWeight: '800' },
  empty: { color: colors.textFaint, fontSize: 13, marginBottom: 6 },
  row: { flexDirection: 'row', alignItems: 'center', gap: 6 },
  del: { paddingHorizontal: 10, paddingVertical: 6 },
  delText: { color: colors.danger, fontSize: 12 },
});
