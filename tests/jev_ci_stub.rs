//! The secret-free CI gate, as a test: build is already done by the harness,
//! so this only starts the stub and drives the real `clank-jev` against it.
//! Same path the `jev` workflow job runs via `scripts/jev-ci-stub.sh`.

use std::process::Command;

#[test]
fn the_checks_file_and_the_gate_wire_against_a_stub() {
    let bin = env!("CARGO_BIN_EXE_clank-jev");
    let status = Command::new("sh")
        .arg("scripts/jev-ci-stub.sh")
        .env("CLANK_JEV_BIN", bin)
        .status()
        .expect("spawn scripts/jev-ci-stub.sh");
    assert!(status.success(), "scripts/jev-ci-stub.sh failed: {status}");
}
