// Ajustes globais só para a versão web: a UI é um app, não uma página, então
// arrastar knobs não pode selecionar texto nem "puxar" imagens/SVGs.
import { Platform } from 'react-native';

if (Platform.OS === 'web' && typeof document !== 'undefined') {
  const style = document.createElement('style');
  style.textContent = `
    * {
      user-select: none;
      -webkit-user-select: none;
      -webkit-user-drag: none;
      -webkit-tap-highlight-color: transparent;
    }
    input, textarea {
      user-select: text;
      -webkit-user-select: text;
    }
  `;
  document.head.appendChild(style);
  // Impede arrastar qualquer elemento (imagens, SVG, links).
  document.addEventListener('dragstart', (e) => e.preventDefault());
}
