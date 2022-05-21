use std::fs::File;
use std::io::prelude::*;

fn main() {
    let mut cargo_text = String::new();
    File::open("Cargo.toml").and_then(|mut f| f.read_to_string(&mut cargo_text)).unwrap();
    let decoded: toml::Value = toml::from_str(&cargo_text).unwrap();

    version(&decoded);
    plugins(&decoded);
}

fn plugins(decoded: &toml::Value) {
    let mut inits = Vec::new();
    if let Some(plugins) = decoded
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("plugins"))
        .and_then(|plugins| plugins.as_table())
    {
        for (id, cfg) in plugins {
            let plugin_id = id.replace('-', "_");
            let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let descr = cfg.get("description").and_then(|v| v.as_str()).unwrap_or_default();
            let default_startup = cfg.get("default_startup").and_then(|v| v.as_bool()).unwrap_or(false);

            println!(
                "plugin_id: {}, default_startup: {}, name: {}, descr: {}",
                plugin_id, default_startup, name, descr
            );

            inits.push(format!(
                "    {}::register(rmqtt::Runtime::instance(), r#\"{}\"#, r#\"{}\"#, {}).await?;",
                plugin_id, name, descr, default_startup
            ));
        }
    }

    let out = std::env::var("OUT_DIR").unwrap();
    let mut plugin_rs = File::create(format!("{}/{}", out, "plugin.rs")).unwrap();
    plugin_rs.write_all(b"pub(crate) async fn init() -> anyhow::Result<()>{\n").unwrap();
    plugin_rs.write_all(inits.join("\n").as_bytes()).unwrap();
    plugin_rs.write_all(b"\n    Ok(())\n}").unwrap();
}

fn version(decoded: &toml::Value) {
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
