//! Integration tests for M37 — performance benchmarks, CI/CD pipeline,
//! packaging scripts, and documentation deliverables.
//!
//! These tests verify that the M37 deliverables exist and are structurally
//! valid without actually invoking external tools (criterion runs, GitHub
//! Actions, or shell packaging scripts). They guard against accidental
//! deletion or corruption of the release infrastructure.

use std::path::Path;

/// Repo root (the directory containing `Cargo.toml`).
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Read a file's contents, failing the test with a clear message if it
/// does not exist.
fn read_deliverable(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("M37 deliverable {rel} missing: {e}"))
}

// ---------------------------------------------------------------------------
// Benchmark harness — benches/frame_bench.rs
// ---------------------------------------------------------------------------

#[test]
fn frame_bench_source_exists() {
    let src = read_deliverable("benches/frame_bench.rs");
    assert!(!src.is_empty(), "frame_bench.rs is empty");
}

#[test]
fn frame_bench_defines_step_frame_benchmark() {
    let src = read_deliverable("benches/frame_bench.rs");
    assert!(
        src.contains("fn bench_step_frame"),
        "frame_bench.rs must define a step_frame benchmark"
    );
    assert!(
        src.contains("criterion_group"),
        "frame_bench.rs must register a criterion group"
    );
    assert!(
        src.contains("criterion_main!"),
        "frame_bench.rs must define a criterion_main entry point"
    );
}

#[test]
fn frame_bench_covers_save_state_and_render() {
    let src = read_deliverable("benches/frame_bench.rs");
    assert!(
        src.contains("bench_save_state"),
        "frame_bench.rs must benchmark save-state serialization"
    );
    assert!(
        src.contains("bench_render_frame"),
        "frame_bench.rs must benchmark the PPU renderer"
    );
    assert!(
        src.contains("bench_cpu_step"),
        "frame_bench.rs must benchmark CPU instruction throughput"
    );
}

#[test]
fn frame_bench_documents_frame_budget() {
    let src = read_deliverable("benches/frame_bench.rs");
    // The benchmark source must document the 16.67 ms NTSC frame budget
    // so future maintainers understand the performance target.
    assert!(
        src.contains("16.67"),
        "frame_bench.rs must document the 16.67 ms frame budget"
    );
}

#[test]
fn cargo_toml_declares_criterion_and_bench_harness() {
    let cargo = read_deliverable("Cargo.toml");
    assert!(
        cargo.contains("criterion"),
        "Cargo.toml must declare criterion as a dev-dependency"
    );
    assert!(
        cargo.contains("[[bench]]"),
        "Cargo.toml must declare a [[bench]] target"
    );
    assert!(
        cargo.contains("frame_bench"),
        "Cargo.toml must register the frame_bench benchmark"
    );
    // The bench harness must be disabled so criterion's own main is used.
    assert!(
        cargo.contains("harness = false"),
        "Cargo.toml must set harness = false for the criterion bench"
    );
}

// ---------------------------------------------------------------------------
// CI/CD pipeline — .github/workflows/ci.yml
// ---------------------------------------------------------------------------

#[test]
fn ci_workflow_exists() {
    let yml = read_deliverable(".github/workflows/ci.yml");
    assert!(!yml.is_empty(), "ci.yml is empty");
}

#[test]
fn ci_workflow_runs_quality_gate() {
    let yml = read_deliverable(".github/workflows/ci.yml");
    assert!(yml.contains("cargo fmt --check"), "CI must run fmt --check");
    assert!(
        yml.contains("cargo clippy") && yml.contains("-D warnings"),
        "CI must run clippy with -D warnings"
    );
    assert!(yml.contains("cargo test"), "CI must run the test suite");
    assert!(
        yml.contains("cargo bench --no-run"),
        "CI must run a bench compile check"
    );
}

#[test]
fn ci_workflow_runs_on_multiple_platforms() {
    let yml = read_deliverable(".github/workflows/ci.yml");
    assert!(yml.contains("ubuntu-latest"), "CI must run on Ubuntu");
    assert!(yml.contains("windows-latest"), "CI must run on Windows");
    assert!(yml.contains("macos-latest"), "CI must run on macOS");
}

#[test]
fn ci_workflow_has_release_on_tag() {
    let yml = read_deliverable(".github/workflows/ci.yml");
    assert!(
        yml.contains("refs/tags/v"),
        "CI must trigger a release on v* tag pushes"
    );
    assert!(yml.contains("release"), "CI must define a release job");
    assert!(
        yml.contains("cargo build --release"),
        "Release job must build a release binary"
    );
}

#[test]
fn ci_workflow_installs_sdl2() {
    let yml = read_deliverable(".github/workflows/ci.yml");
    assert!(
        yml.contains("libsdl2-dev"),
        "CI must install SDL2 dev libraries on Ubuntu"
    );
    assert!(
        yml.contains("brew install sdl2"),
        "CI must install SDL2 on macOS via Homebrew"
    );
}

// ---------------------------------------------------------------------------
// Packaging scripts — packaging/build_*.sh
// ---------------------------------------------------------------------------

#[test]
fn packaging_scripts_exist() {
    for script in [
        "packaging/build_linux.sh",
        "packaging/build_windows.sh",
        "packaging/build_macos.sh",
    ] {
        let src = read_deliverable(script);
        assert!(!src.is_empty(), "{script} is empty");
        assert!(
            src.starts_with("#!/usr/bin/env bash"),
            "{script} must start with a bash shebang"
        );
    }
}

#[test]
fn packaging_scripts_build_release_binary() {
    for script in [
        "packaging/build_linux.sh",
        "packaging/build_windows.sh",
        "packaging/build_macos.sh",
    ] {
        let src = read_deliverable(script);
        assert!(
            src.contains("cargo build --release"),
            "{script} must build a release binary"
        );
        assert!(
            src.contains("set -euo pipefail"),
            "{script} must use strict bash mode"
        );
    }
}

#[test]
fn linux_packaging_produces_tarball() {
    let src = read_deliverable("packaging/build_linux.sh");
    assert!(
        src.contains("tar -czf"),
        "Linux packaging must produce a tar.gz"
    );
    assert!(
        src.contains(".tar.gz"),
        "Linux packaging must produce a .tar.gz artifact"
    );
}

#[test]
fn windows_packaging_produces_zip() {
    let src = read_deliverable("packaging/build_windows.sh");
    assert!(
        src.contains(".zip"),
        "Windows packaging must produce a .zip artifact"
    );
}

#[test]
fn macos_packaging_produces_app_bundle() {
    let src = read_deliverable("packaging/build_macos.sh");
    assert!(
        src.contains("nes-emu.app"),
        "macOS packaging must produce a .app bundle"
    );
    assert!(
        src.contains("Info.plist"),
        "macOS packaging must generate an Info.plist"
    );
}

// ---------------------------------------------------------------------------
// README — build instructions + usage guide
// ---------------------------------------------------------------------------

#[test]
fn readme_exists_and_is_substantial() {
    let readme = read_deliverable("README.md");
    assert!(
        readme.len() > 2000,
        "README.md must be a substantial document (>2KB), got {} bytes",
        readme.len()
    );
}

#[test]
fn readme_has_build_instructions() {
    let readme = read_deliverable("README.md");
    assert!(
        readme.contains("cargo build"),
        "README must document how to build the emulator"
    );
    assert!(
        readme.contains("libsdl2-dev") || readme.contains("SDL2"),
        "README must document the SDL2 dependency"
    );
    assert!(
        readme.contains("rustup") || readme.contains("Rust"),
        "README must document the Rust toolchain requirement"
    );
}

#[test]
fn readme_has_usage_guide_and_hotkeys() {
    let readme = read_deliverable("README.md");
    assert!(
        readme.contains("--rom"),
        "README must document the --rom CLI option"
    );
    assert!(
        readme.contains("F5") && readme.contains("F7"),
        "README must document save-state hotkeys (F5/F7)"
    );
    assert!(
        readme.contains("Alt+Enter"),
        "README must document the fullscreen hotkey"
    );
}

#[test]
fn readme_lists_supported_mappers() {
    let readme = read_deliverable("README.md");
    assert!(
        readme.contains("NROM") && readme.contains("MMC1") && readme.contains("MMC3"),
        "README must list supported mappers"
    );
}

#[test]
fn readme_documents_benchmarks() {
    let readme = read_deliverable("README.md");
    assert!(
        readme.contains("cargo bench"),
        "README must document how to run benchmarks"
    );
    assert!(
        readme.contains("frame_bench") || readme.contains("criterion"),
        "README must reference the criterion benchmark"
    );
}

#[test]
fn readme_documents_packaging() {
    let readme = read_deliverable("README.md");
    assert!(
        readme.contains("packaging/"),
        "README must reference the packaging scripts"
    );
    assert!(
        readme.contains("GitHub Actions") || readme.contains("ci.yml"),
        "README must reference the CI/CD pipeline"
    );
}
