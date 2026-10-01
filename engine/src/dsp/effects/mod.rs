//! Biblioteca de efeitos. Para adicionar um novo:
//! 1. crie `meu_efeito.rs` com um `pub static DESCRIPTOR: EffectDescriptor`;
//! 2. declare o módulo abaixo e acrescente `&meu_efeito::DESCRIPTOR` no REGISTRY.
//! A UI descobre parâmetros, categoria e cor automaticamente.

use super::EffectDescriptor;

pub mod amp;
pub mod cabinet;
pub mod chorus;
pub mod compressor;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod fuzz;
pub mod gate;
pub mod ir_cab;
pub mod nam_amp;
pub mod overdrive;
pub mod reverb;
pub mod tremolo;

/// Ordem = ordem de exibição na biblioteca da UI.
pub static REGISTRY: &[&EffectDescriptor] = &[
    &gate::DESCRIPTOR,
    &compressor::DESCRIPTOR,
    &overdrive::DESCRIPTOR,
    &distortion::DESCRIPTOR,
    &fuzz::DESCRIPTOR,
    &amp::DESCRIPTOR,
    &nam_amp::DESCRIPTOR,
    &cabinet::DESCRIPTOR,
    &ir_cab::DESCRIPTOR,
    &eq::DESCRIPTOR,
    &chorus::DESCRIPTOR,
    &tremolo::DESCRIPTOR,
    &delay::DESCRIPTOR,
    &reverb::DESCRIPTOR,
];
