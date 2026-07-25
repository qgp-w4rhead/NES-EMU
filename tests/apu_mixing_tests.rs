//! Integration tests for M31: non-linear APU mixing, per-channel
//! volume control, mute, and the one-pole low-pass filter.
//!
//! These exercise the public `Apu` API end-to-end: raw channel samples
//! are set up via the register writes, then `Apu::output()` is called
//! and the resulting sample is checked against the expected non-linear
//! mixer value (with LPF transient tolerance).
//!
//! See: https://www.nesdev.org/wiki/APU_Mixer

use nes_emu::apu::Apu;

/// Enable a pulse channel and configure it to output `volume`
/// constantly (duty 3, constant-volume mode, sequence position 0 where
/// duty 3 bit 0 = 1). The caller is responsible for OR-ing the
/// appropriate status bits together when enabling multiple channels.
fn pulse_audible(apu: &mut Apu, pulse2: bool, volume: u8) {
    let p = if pulse2 {
        apu.pulse2_mut()
    } else {
        apu.pulse1_mut()
    };
    // duty 3 (bits 6-7 = 11), const vol (bit 4), volume bits 0-3.
    p.write_register(0, 0b1101_0000 | (volume & 0x0F));
    // period low = 0xFF (>= MIN_AUDIBLE_PERIOD to avoid mute).
    p.write_register(2, 0xFF);
    // length 10 (idx 0), timer high = 7 → timer = 0x0700 = 1792.
    // This keeps the sequence at position 0 during the ~21-tick
    // decimation window (timer won't reach 0), so the pulse output
    // is constant for box-filter averaging.
    p.write_register(3, 0x07);
}

/// Set the triangle channel to output its sequence position 0 value
/// (15) constantly: linear counter reload 127, period 0, length 10.
/// The caller must enable the triangle channel via `write_status`.
fn triangle_audible(apu: &mut Apu) {
    apu.triangle_mut().write_register(0, 0x7F); // halt + linear 127
    apu.triangle_mut().write_register(2, 0xFF); // period low 0xFF
                                                // length 10 (idx 0), timer high = 7 → timer = 0x0700 = 1792.
                                                // Keeps sequence at position 0 (output = 15) during decimation.
    apu.triangle_mut().write_register(3, 0x07);
    apu.clock_quarter_frame(); // start linear counter
}

/// Set the noise channel to output `volume` constantly (const-vol mode,
/// LFSR bit 0 = 0 so the sample is non-zero). The caller must enable
/// the noise channel via `write_status`.
fn noise_audible(apu: &mut Apu, volume: u8) {
    apu.noise_mut()
        .write_register(0, 0b0001_0000 | (volume & 0x0F));
    apu.noise_mut().write_register(3, 0x00); // length 10
    let _ = volume;
}

/// Step the APU by enough CPU cycles to produce one decimated output
/// sample (~40.58 CPU cycles = ~20.29 APU cycles). This is needed
/// because `output()` returns the most recently decimated sample from
/// `step()`, not an instantaneous mix.
fn step_one_sample(apu: &mut Apu) {
    apu.step(42, |_| 0);
}

/// Set the DMC output counter directly via $4011.
fn dmc_set(apu: &mut Apu, value: u8) {
    apu.dmc_mut().write_register(1, value);
}

/// Compute the expected non-linear mix (pre-LPF, pre-mapping) from raw
/// channel samples. Used as an independent oracle in tests.
fn expected_mix(p1: f32, p2: f32, tri: f32, noise: f32, dmc: f32) -> f32 {
    let pulse_sum = p1 + p2;
    let pulse_out = if pulse_sum > 0.0 {
        95.52 / (8128.0 / pulse_sum + 100.0)
    } else {
        0.0
    };
    let tnd_inner = tri / 8227.0 + noise / 12241.0 + dmc / 22638.0;
    let tnd_out = if tnd_inner > 0.0 {
        163.67 / (1.0 / tnd_inner + 100.0)
    } else {
        0.0
    };
    (pulse_out + tnd_out) * 2.0 - 1.0
}

/// Apply one LPF step. `alpha ≈ 0.8188` (12 kHz cutoff at 44.1 kHz).
fn lpf_step(prev: f32, input: f32) -> f32 {
    const ALPHA: f32 = 0.8188;
    ALPHA * input + (1.0 - ALPHA) * prev
}

/// Apply the DC blocker first-sample transform. The DC blocker removes
/// the -1.0 silence DC offset: first sample = lpf_out - (-1.0) + R*0.0
/// = lpf_out + 1.0, where R ≈ 0.99715.
fn dc_block_first(lpf_out: f32) -> f32 {
    lpf_out + 1.0
}

// ---- Silence ----------------------------------------------------------

#[test]
fn silence_outputs_near_zero_after_warmup() {
    let mut apu = Apu::new();
    // All channels off; DMC = 0. Non-linear mix = 0 → mapped to -1.0.
    // LPF converges to -1.0, then DC blocker removes the DC offset → 0.0.
    let mut last = 0.0;
    for _ in 0..200 {
        last = apu.output();
    }
    assert!(
        last.abs() < 1e-3,
        "silence should converge to 0.0 (DC blocker), got {last}"
    );
}

// ---- Pulse-only -------------------------------------------------------

#[test]
fn pulse_only_nonlinear_value() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    pulse_audible(&mut apu, false, 15);
    // pulse1 = 15, all else 0. Expected mix (pre-LPF):
    // pulse_out = 95.52 / (8128/15 + 100) ≈ 0.14882
    // tnd_out = 0 → mixed = 0.14882*2 - 1 = -0.70236
    let expected_raw = expected_mix(15.0, 0.0, 0.0, 0.0, 0.0);
    assert!(
        (expected_raw - (-0.70236)).abs() < 1e-3,
        "raw {expected_raw}"
    );
    // LPF from silence (-1.0), then DC blocker first sample.
    step_one_sample(&mut apu);
    let out = apu.output();
    let expected = dc_block_first(lpf_step(-1.0, expected_raw));
    assert!(
        (out - expected).abs() < 1e-2,
        "expected {expected}, got {out}"
    );
}

#[test]
fn both_pulses_nonlinear_value() {
    let mut apu = Apu::new();
    apu.write_status(0x03);
    pulse_audible(&mut apu, false, 15);
    pulse_audible(&mut apu, true, 15);
    // pulse1 + pulse2 = 30. pulse_out = 95.52 / (8128/30 + 100) ≈ 0.25747.
    // mixed = 0.25747*2 - 1 = -0.48506.
    let expected_raw = expected_mix(15.0, 15.0, 0.0, 0.0, 0.0);
    assert!(
        (expected_raw - (-0.48506)).abs() < 1e-3,
        "raw {expected_raw}"
    );
    step_one_sample(&mut apu);
    let out = apu.output();
    let expected = dc_block_first(lpf_step(-1.0, expected_raw));
    assert!(
        (out - expected).abs() < 1e-2,
        "expected {expected}, got {out}"
    );
}

// ---- DMC-only ---------------------------------------------------------

#[test]
fn dmc_only_nonlinear_value() {
    let mut apu = Apu::new();
    dmc_set(&mut apu, 127);
    // tnd_out = 163.67 / (1/(127/22638) + 100) ≈ 0.58827.
    // mixed = 0.58827*2 - 1 = 0.17654.
    let expected_raw = expected_mix(0.0, 0.0, 0.0, 0.0, 127.0);
    assert!((expected_raw - 0.17654).abs() < 1e-3, "raw {expected_raw}");
    step_one_sample(&mut apu);
    let out = apu.output();
    let expected = dc_block_first(lpf_step(-1.0, expected_raw));
    assert!(
        (out - expected).abs() < 1e-2,
        "expected {expected}, got {out}"
    );
}

// ---- Triangle + DMC ---------------------------------------------------

#[test]
fn triangle_and_dmc_nonlinear_value() {
    let mut apu = Apu::new();
    apu.write_status(0x04);
    triangle_audible(&mut apu);
    dmc_set(&mut apu, 127);
    // tri = 15, dmc = 127. tnd_inner = 15/8227 + 127/22638 ≈ 0.004348.
    // tnd_out = 163.67 / (1/0.004348 + 100) ≈ 0.6883.
    // mixed = 0.6883*2 - 1 = 0.3766.
    let expected_raw = expected_mix(0.0, 0.0, 15.0, 0.0, 127.0);
    step_one_sample(&mut apu);
    let out = apu.output();
    let expected = dc_block_first(lpf_step(-1.0, expected_raw));
    assert!(
        (out - expected).abs() < 1e-2,
        "expected {expected}, got {out}"
    );
}

// ---- All channels max -------------------------------------------------

#[test]
fn all_channels_max_does_not_exceed_one() {
    let mut apu = Apu::new();
    apu.write_status(0x0F);
    pulse_audible(&mut apu, false, 15);
    pulse_audible(&mut apu, true, 15);
    triangle_audible(&mut apu);
    dmc_set(&mut apu, 127);
    // Noise may or may not be audible depending on LFSR; the mixer
    // should still clamp to [-1, 1].
    noise_audible(&mut apu, 15);
    // Warm up the LPF + DC blocker and check outputs stay in [-1, 1].
    // The DC blocker transient from silence → active audio decays with a
    // time constant of ~350 samples (~8 ms at 44.1 kHz); allow 3000
    // samples (~68 ms) for it to settle well below the [-1, 1] bounds.
    for _ in 0..3000 {
        let _ = apu.output();
    }
    for _ in 0..100 {
        let out = apu.output();
        assert!((-1.0..=1.0).contains(&out), "output out of range: {out}");
    }
}

// ---- Per-channel volume -----------------------------------------------

#[test]
fn per_channel_volume_scales_pulse() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    pulse_audible(&mut apu, false, 15);
    apu.set_channel_volume(0, 0.5); // pulse1 at 50%
                                    // Effective pulse1 = 15 * 0.5 = 7.5.
    let expected_raw = expected_mix(7.5, 0.0, 0.0, 0.0, 0.0);
    step_one_sample(&mut apu);
    let out = apu.output();
    let expected = dc_block_first(lpf_step(-1.0, expected_raw));
    assert!(
        (out - expected).abs() < 1e-2,
        "expected {expected}, got {out}"
    );
}

#[test]
fn per_channel_volume_zero_silences_channel() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    pulse_audible(&mut apu, false, 15);
    apu.set_channel_volume(0, 0.0);
    // pulse1 effectively 0 → silence → DC blocker converges to 0.0.
    let mut last = 0.0;
    for _ in 0..200 {
        last = apu.output();
    }
    assert!(
        last.abs() < 1e-3,
        "zero volume should silence channel (DC blocker → 0.0), got {last}"
    );
}

#[test]
fn per_channel_volume_clamps_to_one() {
    let mut apu = Apu::new();
    apu.set_channel_volume(0, 2.0);
    assert_eq!(apu.channel_volume(0), 1.0);
    apu.set_channel_volume(2, -0.5);
    assert_eq!(apu.channel_volume(2), 0.0);
}

#[test]
fn per_channel_volume_out_of_range_index_ignored() {
    let mut apu = Apu::new();
    apu.set_channel_volume(99, 0.5);
    assert_eq!(apu.channel_volume(99), 0.0); // OOB read returns 0.0
}

// ---- Mute -------------------------------------------------------------

#[test]
fn mute_zeros_channel_contribution() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    pulse_audible(&mut apu, false, 15);
    dmc_set(&mut apu, 127);
    apu.set_channel_muted(0, true); // mute pulse1
                                    // pulse1 muted → only DMC contributes.
    let expected_raw = expected_mix(0.0, 0.0, 0.0, 0.0, 127.0);
    step_one_sample(&mut apu);
    let out = apu.output();
    let expected = dc_block_first(lpf_step(-1.0, expected_raw));
    assert!(
        (out - expected).abs() < 1e-2,
        "expected {expected}, got {out}"
    );
}

#[test]
fn toggle_channel_mute_flips_state() {
    let mut apu = Apu::new();
    assert!(!apu.channel_muted_at(0));
    assert_eq!(apu.toggle_channel_mute(0), Some(true));
    assert!(apu.channel_muted_at(0));
    assert_eq!(apu.toggle_channel_mute(0), Some(false));
    assert!(!apu.channel_muted_at(0));
}

#[test]
fn toggle_channel_mute_out_of_range_returns_none() {
    let mut apu = Apu::new();
    assert_eq!(apu.toggle_channel_mute(99), None);
}

#[test]
fn reset_channel_mix_clears_mutes_and_volumes() {
    let mut apu = Apu::new();
    apu.set_channel_muted(0, true);
    apu.set_channel_muted(3, true);
    apu.set_channel_volume(1, 0.3);
    apu.set_channel_volume(4, 0.0);
    apu.reset_channel_mix();
    for i in 0..5 {
        assert!(!apu.channel_muted_at(i), "channel {i} should be unmuted");
        assert_eq!(apu.channel_volume(i), 1.0, "channel {i} should be 1.0");
    }
}

// ---- apply_channel_volumes from config --------------------------------

#[test]
fn apply_channel_volumes_sets_all_five() {
    let mut apu = Apu::new();
    let vols = [0.1, 0.2, 0.3, 0.4, 0.5];
    apu.apply_channel_volumes(&vols);
    assert_eq!(apu.channel_volume(0), 0.1);
    assert_eq!(apu.channel_volume(1), 0.2);
    assert_eq!(apu.channel_volume(2), 0.3);
    assert_eq!(apu.channel_volume(3), 0.4);
    assert_eq!(apu.channel_volume(4), 0.5);
}

#[test]
fn apply_channel_volumes_clamps_out_of_range() {
    let mut apu = Apu::new();
    apu.apply_channel_volumes(&[2.0, -1.0, 0.5, 1.5, 0.0]);
    assert_eq!(apu.channel_volume(0), 1.0);
    assert_eq!(apu.channel_volume(1), 0.0);
    assert_eq!(apu.channel_volume(2), 0.5);
    assert_eq!(apu.channel_volume(3), 1.0);
    assert_eq!(apu.channel_volume(4), 0.0);
}

#[test]
fn apply_channel_volumes_short_slice_leaves_rest() {
    let mut apu = Apu::new();
    apu.set_channel_volume(4, 0.7);
    apu.apply_channel_volumes(&[0.1, 0.2]); // only first two
    assert_eq!(apu.channel_volume(0), 0.1);
    assert_eq!(apu.channel_volume(1), 0.2);
    assert_eq!(apu.channel_volume(4), 0.7); // unchanged
}

// ---- Selected channel -------------------------------------------------

#[test]
fn selected_channel_defaults_zero_and_clamps() {
    let mut apu = Apu::new();
    assert_eq!(apu.selected_channel(), 0);
    apu.set_selected_channel(3);
    assert_eq!(apu.selected_channel(), 3);
    apu.set_selected_channel(99);
    assert_eq!(apu.selected_channel(), 4); // clamped to 4
}

// ---- LPF smoothing ----------------------------------------------------

#[test]
fn lpf_smooths_step_input() {
    let mut apu = Apu::new();
    // Set DMC to 127 → raw mix ≈ 0.1765 every sample. The LPF should
    // exponentially approach this value rather than jumping there
    // immediately. The DC blocker then removes the DC offset.
    dmc_set(&mut apu, 127);
    let expected_raw = expected_mix(0.0, 0.0, 0.0, 0.0, 127.0);
    // LPF initializes to -1.0 (silence level) to avoid boot click.
    // DC blocker initializes with dc_prev_x = -1.0, dc_prev_y = 0.0.
    let mut prev_lpf = -1.0;
    let mut prev_dc_x = -1.0;
    let mut prev_dc_y = 0.0;
    const DC_R: f32 = 0.99715;
    for _ in 0..5 {
        step_one_sample(&mut apu);
        let out = apu.output();
        let lpf = lpf_step(prev_lpf, expected_raw);
        let dc = lpf - prev_dc_x + DC_R * prev_dc_y;
        assert!(
            (out - dc).abs() < 1e-2,
            "LPF+DC step mismatch: expected {dc}, got {out}"
        );
        prev_lpf = lpf;
        prev_dc_x = lpf;
        prev_dc_y = dc;
    }
}

#[test]
fn lpf_converges_to_steady_input() {
    let mut apu = Apu::new();
    dmc_set(&mut apu, 64);
    // With a constant input, the LPF converges to the raw mix value,
    // but the DC blocker removes the DC component → converges to 0.0.
    // The DC blocker time constant is ~350 samples; use 3000 to converge
    // well within 1e-2.
    let mut last = 0.0;
    for _ in 0..3000 {
        last = apu.output();
    }
    assert!(
        last.abs() < 1e-2,
        "LPF+DC should converge to 0.0 for constant input (DC removed), got {last}"
    );
}

// ---- Determinism ------------------------------------------------------

#[test]
fn output_is_deterministic_across_runs() {
    let mut a = Apu::new();
    let mut b = Apu::new();
    pulse_audible(&mut a, false, 10);
    pulse_audible(&mut b, false, 10);
    dmc_set(&mut a, 50);
    dmc_set(&mut b, 50);
    for _ in 0..100 {
        let oa = a.output();
        let ob = b.output();
        assert_eq!(oa.to_bits(), ob.to_bits(), "non-deterministic output");
    }
}

// ---- Save state preserves mix state -----------------------------------

#[test]
fn save_state_round_trip_preserves_channel_volumes_and_mutes() {
    use nes_emu::cartridge::Cartridge;
    use nes_emu::emulator::EmulatorState;

    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA);
    bytes[16 + 0x3FFC] = 0x00;
    bytes[16 + 0x3FFD] = 0xC0;
    let cart = Cartridge::from_bytes(&bytes).expect("build cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();

    emu.bus_mut().apu_mut().set_channel_volume(0, 0.42);
    emu.bus_mut().apu_mut().set_channel_volume(3, 0.17);
    emu.bus_mut().apu_mut().set_channel_muted(2, true);
    emu.bus_mut().apu_mut().set_selected_channel(4);

    let blob = emu.save_state().expect("save");

    // Mutate the live state.
    emu.bus_mut().apu_mut().reset_channel_mix();
    emu.bus_mut().apu_mut().set_selected_channel(0);

    emu.load_state(&blob).expect("restore");

    let vols = emu.bus().apu().channel_volumes();
    assert!((vols[0] - 0.42).abs() < 1e-6, "pulse1 volume {vols:?}");
    assert!((vols[3] - 0.17).abs() < 1e-6, "noise volume {vols:?}");
    let muted = emu.bus().apu().channel_muted();
    assert!(muted[2], "triangle should be muted");
    assert!(!muted[0], "pulse1 should not be muted");
    assert_eq!(emu.bus().apu().selected_channel(), 4);
}
