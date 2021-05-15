fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    println!("out: {}", out);
    let build_res = tonic_build::configure()
        .out_dir(&format!("{}", out))
        .compile(&["pb.proto"], &["src/grpc/proto"]);
    println!("compile proto result! {:?}", build_res);
    build_res.unwrap();
}
