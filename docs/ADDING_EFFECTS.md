# Criando um efeito novo

Cada efeito é um módulo Rust independente. A UI lê os parâmetros do motor e
desenha o pedal sozinha: **não precisa mexer em nada no `ui/`**.

## 1. Crie o arquivo

`engine/src/dsp/effects/boost.rs`:

```rust
use crate::dsp::util::*;
use crate::dsp::*;

pub static DESCRIPTOR: EffectDescriptor = EffectDescriptor {
    id: "boost",                 // id estável, usado nos presets. Nunca mude.
    name: "Clean Boost",
    category: Category::Drive,   // agrupa na biblioteca
    description: "Aumenta o volume sem colorir o timbre.",
    color: "#f43f5e",            // cor do pedal na UI
    params: &[
        ParamSpec::knob("level", "Level", -12.0, 20.0, 6.0, "dB"),
        ParamSpec::log_knob("tone", "Tone", 1000.0, 12000.0, 8000.0, "Hz"),
        // ParamSpec::choice("mode", "Mode", &["A", "B"], 0),
    ],
    // Dual:: dá suporte estéreo grátis para efeitos mono.
    create: |sr| Dual::boxed(|| Boost::new(sr)),
};

struct Boost {
    sr: f32,
    gain: Smoother,  // suaviza mudanças para não estalar
    tone: OnePole,
}

impl Boost {
    fn new(sr: f32) -> Self {
        Self { sr, gain: Smoother::new(1.0, 20.0, sr), tone: OnePole::lowpass(sr, 8000.0) }
    }
}

impl MonoEffect for Boost {
    // Os índices seguem a ordem de `params`; o valor chega na unidade declarada.
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.gain.set(db_to_lin(value)),
            1 => self.tone.set_freq(self.sr, value),
            _ => {}
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        for x in buf.iter_mut() {
            *x = self.tone.process(*x) * self.gain.next();
        }
    }

    fn reset(&mut self) {
        self.tone.reset();
        self.gain.snap();
    }
}
```

## 2. Registre

Em `engine/src/dsp/effects/mod.rs`:

```rust
pub mod boost;
// ...
pub static REGISTRY: &[&EffectDescriptor] = &[
    // ...
    &boost::DESCRIPTOR,
];
```

## 3. Teste

```powershell
cd engine
cargo test --release    # roda todos os efeitos com sinal de teste
cargo run --release
```

Os testes em `dsp/mod.rs` passam sinal por **todos** os efeitos registrados em
44,1/48/96 kHz, com parâmetros no mínimo e no máximo, e falham se aparecer
NaN, volume explosivo ou saída muda. O efeito novo entra nesses testes
automaticamente.

## Regras de ouro (tempo real)

`process` e `set_param` rodam na thread de áudio:

- **Não aloque** (`Vec::new`, `Box::new`, `format!`...). Aloque tudo em `new()`.
- **Não trave** (sem `Mutex`, sem I/O, sem `println!`).
- Suavize parâmetros que mexem em ganho (`Smoother`) para não estalar.
- Não-linearidades fortes? Use `Oversampler4` para evitar aliasing.
- Efeito que cria estéreo (chorus, delay, reverb)? Implemente `Effect`
  diretamente e retorne `true` em `process`.

## Blocos prontos (`dsp/util.rs`)

`Biquad` (LP/HP/BP/peak/shelf/notch), `OnePole`, `Smoother`, `DelayLine`
(leitura fracionária Hermite), `Oversampler4`, `tube()` (saturação
assimétrica), `diode()`, `db_to_lin`, `knob()`/`knob_exp()` (mapeiam 0..10).
