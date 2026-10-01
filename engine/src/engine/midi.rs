//! Entrada MIDI (WinMM via midir, sem driver): abre todas as portas,
//! reconecta quando um dispositivo aparece e entrega as mensagens ao
//! controlador, que aplica os mapeamentos.

use std::collections::HashMap;
use std::sync::Arc;

use midir::{MidiInput, MidiInputConnection};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// O que um controle MIDI comanda. Endereçado por efeito+parâmetro (não por
/// pedal), então vale para qualquer preset que tenha aquele efeito.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum MidiTarget {
    Param { effect_id: String, param_id: String },
    Bypass { effect_id: String },
    OutputVolume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiBinding {
    pub cc: u8,
    pub target: MidiTarget,
}

#[derive(Debug, Clone, Copy)]
pub enum MidiEvent {
    ControlChange { cc: u8, value: u8 },
    ProgramChange { program: u8 },
}

fn parse(msg: &[u8]) -> Option<MidiEvent> {
    match (msg.first()? & 0xF0, msg.get(1), msg.get(2)) {
        (0xB0, Some(&cc), Some(&value)) => Some(MidiEvent::ControlChange { cc, value }),
        (0xC0, Some(&program), _) => Some(MidiEvent::ProgramChange { program }),
        _ => None,
    }
}

pub struct Midi {
    connections: HashMap<String, MidiInputConnection<()>>,
    queue: Arc<Mutex<Vec<MidiEvent>>>,
}

impl Midi {
    pub fn new() -> Self {
        let mut m = Self { connections: HashMap::new(), queue: Arc::new(Mutex::new(Vec::new())) };
        m.rescan();
        m
    }

    /// Conecta portas novas e esquece as que sumiram. Retorna se mudou.
    pub fn rescan(&mut self) -> bool {
        let Ok(probe) = MidiInput::new("ScreamO probe") else { return false };
        let ports = probe.ports();
        let names: Vec<String> = ports.iter().filter_map(|p| probe.port_name(p).ok()).collect();
        let before = self.connections.len();
        self.connections.retain(|name, _| names.contains(name));
        let mut changed = self.connections.len() != before;
        for (port, name) in ports.iter().zip(&names) {
            if self.connections.contains_key(name) {
                continue;
            }
            let Ok(input) = MidiInput::new("ScreamO") else { continue };
            let queue = self.queue.clone();
            let conn = input.connect(
                port,
                "screamo-in",
                move |_, msg, _| {
                    if let Some(ev) = parse(msg) {
                        let mut q = queue.lock();
                        // Pedal de expressão manda muitas mensagens: limita a fila.
                        if q.len() < 1024 {
                            q.push(ev);
                        }
                    }
                },
                (),
            );
            if let Ok(c) = conn {
                self.connections.insert(name.clone(), c);
                changed = true;
            }
        }
        changed
    }

    pub fn devices(&self) -> Vec<String> {
        let mut v: Vec<String> = self.connections.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn drain(&self) -> Vec<MidiEvent> {
        std::mem::take(&mut *self.queue.lock())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cc_and_program_change_on_any_channel() {
        assert!(matches!(parse(&[0xB3, 11, 100]), Some(MidiEvent::ControlChange { cc: 11, value: 100 })));
        assert!(matches!(parse(&[0xC0, 4]), Some(MidiEvent::ProgramChange { program: 4 })));
        assert!(parse(&[0x90, 60, 100]).is_none()); // note on: ignorado
    }
}
