//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

/// Runs one headless egui pass. Nothing uploads the output's texture deltas, so they are
/// cleared here: egui asserts that a `TexturesDelta` is never dropped with deltas pending.
pub(crate) fn headless_pass(
    ctx: &egui::Context,
    input: egui::RawInput,
    run_ui: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let mut output = ctx.run_ui(input, run_ui);
    output.textures_delta.clear();
    output
}
