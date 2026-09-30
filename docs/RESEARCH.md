# Projetos existentes e o que o ScreamO faz diferente

## O que já existe

| Projeto | Stack | Pontos fortes | Limitações relevantes pra gente |
|---|---|---|---|
| [Guitarix](https://guitarix.org) | C++ + Faust, JACK | Enorme biblioteca de efeitos, DSP gerado em Faust (bem otimizado) | Focado em Linux/JACK; no Windows não é prático |
| [Rakarrack](https://rakarrack.sourceforge.net) | C++, JACK/ALSA | Muitos efeitos clássicos | Linux, projeto parado |
| [Neural Amp Modeler (NAM)](https://github.com/sdatkinson/neural-amp-modeler) | C++ (plugin), PyTorch (treino) | Captura por rede neural de amps reais; qualidade de ponta | É um plugin VST: precisa de DAW/host e, no Windows, normalmente ASIO |
| [go-dsp-guitar](https://github.com/andrepxx/go-dsp-guitar) | Go, JACK, UI web | Arquitetura parecida com a nossa (motor headless + UI web), cabinet por IR | Usa JACK; latência depende do servidor JACK |
| [Rustortion](https://github.com/Avarise/rustortion) | Rust, JACK/PipeWire | Amp sim em Rust com oversampling | Só Linux |
| [WebAudio Guitar Amp Sim](https://github.com/micbuffa/WebAudio-Guitar-Amplifier-Simulator) | JS, Web Audio API | Roda no navegador, bem didático | No Windows o navegador fica preso ao WASAPI compartilhado padrão (~10 ms só de saída), mais o overhead do browser |
| Comerciais (Amplitube, Guitar Rig, BIAS FX, Neural DSP) | C++ | Qualidade de estúdio | Pagos; baixa latência depende de ASIO/driver da interface |

## Padrões que copiamos

- **Motor separado da UI** (go-dsp-guitar, Guitarix): a thread de áudio não
  depende da UI. Se a janela travar ou fechar, o som continua.
- **Oversampling nas não-linearidades** (Rustortion, plugins comerciais):
  distorção sem oversampling gera aliasing e aquele som "áspero/digital".
- **Cadeia de estágios do amp** (Guitarix/Faust): pré de válvula com bias
  assimétrico → acoplamento → segundo estágio → tone stack → power amp →
  presence. Não usamos uma "curva de distorção" única.
- **Freeverb** (domínio público) para o reverb.
- **YIN** para o afinador (mesma família de algoritmo usada por afinadores de
  software).

## O que fazemos melhor/diferente

1. **Sem ASIO e sem driver.** A maioria dos projetos no Windows depende de ASIO
   (ou ASIO4ALL) para ter latência aceitável. O ScreamO usa `IAudioClient3`
   (WASAPI de baixa latência, Windows 10+), que dá períodos de ~3 ms **em modo
   compartilhado**: os outros apps continuam tocando. Modo exclusivo é
   opcional.
2. **Entrada e saída em dispositivos diferentes, sem estalos.** O caso comum é
   cabo USB de guitarra + fone no P2 do PC: dois relógios de hardware. Hosts
   simples ou ignoram isso (e estalam de tempos em tempos) ou exigem o mesmo
   dispositivo. Nós temos um resampler adaptativo que corrige o drift
   continuamente e também converte taxas diferentes.
3. **Detecção automática do cabo e do canal.** Muitos cabos USB baratos entregam
   a guitarra só no canal esquerdo; outros nos dois. O modo "Automático" escolhe
   o canal com sinal. O nome do dispositivo é usado para achar o cabo sozinho.
4. **Segurança para os ouvidos**: soft-clipper na saída (nunca passa de 0 dBFS)
   e mudo automático se só houver microfone (evita microfonia).
5. **Tempo real correto**: zero alocações/locks na thread de áudio. Mudanças
   de parâmetro e da cadeia viajam por filas lock-free (`rtrb`), e objetos
   removidos voltam por uma fila de "lixo" para serem liberados fora da thread
   de áudio. Threads registradas no MMCSS como "Pro Audio", com denormals
   desligados (FTZ/DAZ).
6. **Bypass sem clique**: crossfade de 8 ms ao ligar/desligar pedal.
7. **Módulos auto-descritos**: cada efeito declara parâmetros (faixa, unidade,
   escala log, opções), categoria e cor. A UI é gerada a partir disso, então
   um efeito novo não exige mexer na UI.
8. **Presets por id de parâmetro** (não por posição): continuam válidos quando
   um efeito ganha knobs novos.
9. **Segurança do servidor local**: só escuta em 127.0.0.1 e recusa WebSockets
   de outras origens. Sem isso, qualquer site aberto no navegador poderia
   controlar o motor (cross-site WebSocket hijacking). Nomes de preset são
   validados contra path traversal.

## Próximos passos inspirados nos projetos acima

- **IR de caixa** (go-dsp-guitar, todos os comerciais): convolução particionada
  uniforme via FFT para carregar `.wav` de caixas reais.
- **NAM**: carregar arquivos `.nam` (WaveNet/LSTM). Existe uma implementação
  C++ de referência (`NeuralAmpModelerCore`) que pode ser portada ou
  vinculada.
- **Faust** (Guitarix): gerar código Rust a partir de DSP em Faust para
  prototipar efeitos mais rápido.
