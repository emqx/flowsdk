use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("mqttv5_descriptor.bin"))
        .compile_protos(&["proto/mqttv5.proto"], &["proto"])?;
    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("mqttv3_descriptor.bin"))
        .compile_protos(&["proto/mqttv3.proto"], &["proto"])?;
    Ok(())
}
