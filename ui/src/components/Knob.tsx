import { memo, useMemo, useRef, useState } from 'react';
import { PanResponder, StyleSheet, Text, View } from 'react-native';
import Svg, { Circle, Line, Path } from 'react-native-svg';
import { colors, mono } from '../theme';

const START = -135;
const END = 135;
const DRAG_PIXELS = 180; // arrastar 180px = curso completo

type Props = {
  label: string;
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  unit?: string;
  log?: boolean;
  color?: string;
  size?: number;
  onChange: (v: number) => void;
};

const toNorm = (v: number, min: number, max: number, log?: boolean) =>
  log ? Math.log(v / min) / Math.log(max / min) : (v - min) / (max - min);

const fromNorm = (n: number, min: number, max: number, log?: boolean) =>
  log ? min * Math.pow(max / min, n) : min + (max - min) * n;

function polar(cx: number, cy: number, r: number, deg: number) {
  const a = ((deg - 90) * Math.PI) / 180;
  return { x: cx + r * Math.cos(a), y: cy + r * Math.sin(a) };
}

function arc(cx: number, cy: number, r: number, from: number, to: number) {
  const s = polar(cx, cy, r, from);
  const e = polar(cx, cy, r, to);
  const large = to - from > 180 ? 1 : 0;
  return `M ${s.x} ${s.y} A ${r} ${r} 0 ${large} 1 ${e.x} ${e.y}`;
}

export function formatValue(v: number, unit = ''): string {
  if (unit === 'Hz') return v >= 1000 ? `${(v / 1000).toFixed(v >= 10000 ? 1 : 2)}k` : `${Math.round(v)}`;
  if (unit === 'ms') return v >= 1000 ? `${(v / 1000).toFixed(2)}s` : `${Math.round(v)}ms`;
  if (unit === 'dB') return `${v > 0 ? '+' : ''}${v.toFixed(1)}`;
  if (unit === '%') return `${Math.round(v)}%`;
  if (unit === ':1') return `${v.toFixed(1)}:1`;
  return v.toFixed(1);
}

function KnobImpl({ label, value, min, max, defaultValue, unit, log, color = colors.accent, size = 58, onChange }: Props) {
  // Enquanto arrasta, mostra o valor local (ecos do motor chegam atrasados).
  const [dragValue, setDragValue] = useState<number | null>(null);
  const shown = dragValue ?? value;
  const latest = useRef({ value, min, max, log, onChange, defaultValue });
  latest.current = { value, min, max, log, onChange, defaultValue };
  const startNorm = useRef(0);
  const lastTap = useRef(0);
  const moved = useRef(false);

  const responder = useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onMoveShouldSetPanResponder: () => true,
        onPanResponderGrant: () => {
          const { value, min, max, log } = latest.current;
          startNorm.current = toNorm(value, min, max, log);
          moved.current = false;
        },
        onPanResponderMove: (_, g) => {
          const { min, max, log, onChange } = latest.current;
          if (Math.abs(g.dy) + Math.abs(g.dx) > 2) moved.current = true;
          const n = Math.min(1, Math.max(0, startNorm.current + (-g.dy + g.dx * 0.5) / DRAG_PIXELS));
          const v = fromNorm(n, min, max, log);
          setDragValue(v);
          onChange(v);
        },
        onPanResponderRelease: () => {
          setDragValue(null);
          if (!moved.current) {
            // Duplo clique volta ao padrão.
            const now = Date.now();
            if (now - lastTap.current < 300) latest.current.onChange(latest.current.defaultValue);
            lastTap.current = now;
          }
        },
        onPanResponderTerminate: () => setDragValue(null),
      }),
    [],
  );

  const norm = Math.min(1, Math.max(0, toNorm(shown, min, max, log)));
  const angle = START + (END - START) * norm;
  const c = size / 2;
  const r = size / 2 - 4;
  const tip = polar(c, c, r - 9, angle);
  // Knobs bipolares (ex.: -15..+15 dB) acendem a partir do centro.
  const bipolar = min < 0 && max > 0 && !log;
  const zeroAngle = bipolar ? START + (END - START) * toNorm(0, min, max) : START;
  const [a0, a1] = angle >= zeroAngle ? [zeroAngle, angle] : [angle, zeroAngle];

  return (
    <View style={styles.wrap}>
      <View {...responder.panHandlers} style={[styles.hit, { width: size, height: size }]}>
        <Svg width={size} height={size}>
          <Path d={arc(c, c, r, START, END)} stroke={colors.border} strokeWidth={4} fill="none" strokeLinecap="round" />
          {a1 - a0 > 0.5 && (
            <Path d={arc(c, c, r, a0, a1)} stroke={color} strokeWidth={4} fill="none" strokeLinecap="round" />
          )}
          <Circle cx={c} cy={c} r={r - 7} fill={colors.panelHi} stroke="#000" strokeWidth={1} />
          <Line x1={c} y1={c} x2={tip.x} y2={tip.y} stroke={colors.text} strokeWidth={2.5} strokeLinecap="round" />
        </Svg>
      </View>
      <Text style={[styles.value, dragValue !== null && { color }]}>{formatValue(shown, unit)}</Text>
      <Text style={styles.label} numberOfLines={1}>
        {label}
      </Text>
    </View>
  );
}

export const Knob = memo(KnobImpl);

const styles = StyleSheet.create({
  wrap: { alignItems: 'center', width: 72 },
  hit: { cursor: 'ns-resize' } as object,
  value: { color: colors.text, fontSize: 11, fontFamily: mono, marginTop: 2 },
  label: { color: colors.textDim, fontSize: 10, textTransform: 'uppercase', letterSpacing: 0.8, marginTop: 1 },
});
