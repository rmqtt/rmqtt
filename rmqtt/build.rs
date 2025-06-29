fn main() {
    let version = rustc_version::version().unwrap();
    let build_time = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    println!("cargo:rustc-env=RUSTC_VERSION={version}");
    println!("cargo:rustc-env=RUSTC_BUILD_TIME={build_time}");
    println!("cargo:rerun-if-changed=build.rs");
}
