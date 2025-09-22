use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("mqtt_descriptor.bin"))
        .extern_path(".mqttv3", "crate::mqttv3pb")
        .extern_path(".mqttv5", "crate::mqttv5pb")
        .compile_protos(
            &[
                "proto/mqtt.proto",
                "proto/mqttv3.proto",
                "proto/mqttv5.proto",
            ],
            &["proto"],
        )?;

    Ok(())
}
