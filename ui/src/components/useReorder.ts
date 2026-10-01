import { useRef, useState } from 'react';
import { type LayoutChangeEvent, PanResponder, type PanResponderInstance } from 'react-native';

type Layout = { x: number; w: number };
export type DragState = { id: number; dx: number; target: number } | null;

/**
 * Reordenação horizontal por arrastar e soltar. Cada item informa seu
 * layout (onLayout); o arraste usa o centro do item para achar a posição
 * de destino entre os demais.
 */
export function useReorder(ids: number[], onMove: (id: number, to: number) => void) {
  const layouts = useRef(new Map<number, Layout>());
  const responders = useRef(new Map<number, PanResponderInstance>());
  const [drag, setDrag] = useState<DragState>(null);
  const latest = useRef({ ids, onMove });
  latest.current = { ids, onMove };

  const targetFor = (id: number, dx: number) => {
    const me = layouts.current.get(id);
    if (!me) return latest.current.ids.indexOf(id);
    const center = me.x + me.w / 2 + dx;
    return latest.current.ids.filter((o) => {
      if (o === id) return false;
      const l = layouts.current.get(o);
      return !!l && l.x + l.w / 2 < center;
    }).length;
  };

  const handlersFor = (id: number) => {
    let r = responders.current.get(id);
    if (!r) {
      r = PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onMoveShouldSetPanResponder: () => true,
        onPanResponderGrant: () => setDrag({ id, dx: 0, target: latest.current.ids.indexOf(id) }),
        onPanResponderMove: (_, g) => setDrag({ id, dx: g.dx, target: targetFor(id, g.dx) }),
        onPanResponderRelease: (_, g) => {
          const target = targetFor(id, g.dx);
          if (target !== latest.current.ids.indexOf(id)) latest.current.onMove(id, target);
          setDrag(null);
        },
        onPanResponderTerminate: () => setDrag(null),
      });
      responders.current.set(id, r);
    }
    return r.panHandlers;
  };

  const onLayout = (id: number) => (e: LayoutChangeEvent) => {
    const { x, width } = e.nativeEvent.layout;
    layouts.current.set(id, { x, w: width });
  };

  return { drag, handlersFor, onLayout };
}
