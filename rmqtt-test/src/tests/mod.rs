//! Test suites

use std::path::PathBuf;

pub mod chaos;
pub mod functional;
pub mod stress;

/// Resolve a test broker config path, e.g. `config_path("retain-disabled")`
/// -> `<workspace>/rmqtt-test/configs/retain-disabled/rmqtt.toml`.
///
/// Built from `CARGO_MANIFEST_DIR` (the `rmqtt-test` crate dir), so the
/// returned path is absolute and independent of the process working
/// directory. Config files must live under `rmqtt-test/configs/<name>/` and
/// be self-contained (their own `plugins/` sub-dir).
pub fn config_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join(name).join("rmqtt.toml")
}
