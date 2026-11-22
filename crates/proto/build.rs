fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/plugin.proto");
    // the contract is compiled in Rust (protox): building the crate needs no protoc
    let fds = protox::compile(["plugin.proto"], ["proto"])?;
    // no `XClient::connect(dst)` constructors: clients are built from a `Channel`
    tonic_prost_build::configure()
        .build_transport(false)
        .bytes(".")
        .compile_fds(fds)?;
    Ok(())
}
