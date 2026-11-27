//! Test suites

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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

/// Prepared copy of a self-contained fixture, returned as a `rmqtt.toml` path.
///
/// Unlike [`config_path`] this does **not** hand the repository file to the
/// broker: it copies `rmqtt-test/configs/<name>/` into
/// `<workspace>/target/test-fixtures/<name>/` and rewrites the two placeholders
/// the fixture may declare,
///
/// * `@@PLUGINS_DIR@@` -> absolute path of the copied `plugins/` directory
///   (the broker resolves a relative `plugins.dir` against *its* working
///   directory, which is not necessarily the repository root), and
/// * `@@LOG_DIR@@` -> absolute path of a fresh `logs/` directory, so a case
///   that needs broker-side evidence can read a known file
///   (`log.to = "file"`).
///
/// The result is memoised per name: [`TestCase::broker_config`] is called
/// while the suites are built, i.e. before the scheduler starts the broker at
/// the sub-suite boundary, and the copy must already exist at that point.
///
/// [`TestCase::broker_config`]: crate::framework::testcase::TestCase::broker_config
pub fn fixture_config(name: &'static str) -> PathBuf {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = cache.get(name) {
        return path.clone();
    }
    let path = prepare_fixture(name)
        .unwrap_or_else(|e| panic!("failed to prepare the `{name}` fixture config: {e:#}"));
    cache.insert(name, path.clone());
    path
}

/// Log directory of a prepared fixture (created by [`fixture_config`]).
pub fn fixture_logs_dir(name: &str) -> PathBuf {
    fixture_root(name).join("logs")
}

fn fixture_root(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rmqtt-test must live inside the workspace")
        .join("target")
        .join("test-fixtures")
        .join(name)
}

fn prepare_fixture(name: &str) -> anyhow::Result<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join(name);
    if !src.is_dir() {
        anyhow::bail!("fixture directory {} does not exist", src.display());
    }
    let dst = fixture_root(name);
    copy_dir_recursive(&src, &dst)?;

    let logs = dst.join("logs");
    // A fresh log per harness process: the directory lives in `target/` and is
    // reused across runs, so keeping the previous run's lines would let a
    // log-based assertion pass on stale evidence (and let an "absence" case
    // fail on it). `prepare_fixture` runs at suite-build time, before the
    // scheduler starts the broker, so nothing is writing here yet.
    if logs.exists() {
        std::fs::remove_dir_all(&logs)?;
    }
    std::fs::create_dir_all(&logs)?;

    let abs_plugins = std::fs::canonicalize(dst.join("plugins"))?.to_string_lossy().replace('\\', "/");
    let abs_logs = std::fs::canonicalize(&logs)?.to_string_lossy().replace('\\', "/");

    // `-f` must point at the FILE: rmqtt-conf silently ignores a directory and
    // would fall back to the built-in defaults.
    let cfg = dst.join("rmqtt.toml");
    let content = std::fs::read_to_string(&cfg)?;
    std::fs::write(&cfg, content.replace("@@PLUGINS_DIR@@", &abs_plugins).replace("@@LOG_DIR@@", &abs_logs))?;
    Ok(cfg)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
