fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("failed to find vendored protoc");
    unsafe {
        std::env::set_var("PROTOC", protoc_path);
    }
    tonic_build::compile_protos("../../../proto/orionis.proto")?;
    Ok(())
}
