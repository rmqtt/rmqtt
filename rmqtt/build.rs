use std::fs::File;
use std::io::prelude::*;

fn main() {
    proto();
    version();
}

fn proto() {
    let out = std::env::var("OUT_DIR").unwrap();
    println!("out: {}", out);
    let build_res = tonic_build::configure().out_dir(out).compile(&["pb.proto"], &["src/grpc/proto"]);
    println!("compile proto result! {:?}", build_res);
    build_res.unwrap();
}

fn version() {
    let mut cargo_text = String::new();
    File::open("Cargo.toml").and_then(|mut f| f.read_to_string(&mut cargo_text)).unwrap();
    let decoded: toml::Value = toml::from_str(&cargo_text).unwrap();

    let version = decoded.get("package").unwrap().get("version").unwrap().as_str().unwrap();
    let build_time = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let server_version = format!("rmqtt/{}-{}", &version, &build_time);

    let out = std::env::var("OUT_DIR").unwrap();
    let mut version_file = File::create(format!("{}/{}", out, "version.rs")).unwrap();
    version_file.write_all(b"\n/// rmqtt version").unwrap();
    version_file
        .write_all(format!("\npub const VERSION: &str = \"{}\";", server_version).as_bytes())
        .unwrap();
}
