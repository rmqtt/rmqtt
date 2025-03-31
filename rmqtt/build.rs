use std::fs::File;
use std::io::prelude::*;
use std::process::Command;

fn main() {
    version();
}

fn version() {
    let mut cargo_text = String::new();
    File::open("../Cargo.toml").and_then(|mut f| f.read_to_string(&mut cargo_text)).unwrap();
    let decoded: toml::Value = toml::from_str(&cargo_text).unwrap();
    let version =
        decoded.get("workspace").unwrap().get("package").unwrap().get("version").unwrap().as_str().unwrap();
    let build_time = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let server_version = format!("rmqtt/{}-{}", &version, &build_time);

    let out = std::env::var("OUT_DIR").unwrap();
    let mut version_file = File::create(format!("{}/{}", out, "version.rs")).unwrap();
    version_file.write_all(b"\n/// rmqtt version").unwrap();
    version_file
        .write_all(format!("\npub const VERSION: &str = \"{}\";", server_version).as_bytes())
        .unwrap();

    let rustc_version_out = Command::new("rustc").arg("--version").output().expect("Failed to execute rustc");
    let rustc_version = String::from_utf8_lossy(&rustc_version_out.stdout);
    version_file.write_all(b"\n/// rustc version").unwrap();
    version_file
        .write_all(format!("\npub const RUSTC_VERSION: &str = \"{}\";", rustc_version.trim()).as_bytes())
        .unwrap();

    println!("cargo:rerun-if-changed=build.rs");
}
