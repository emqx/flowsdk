// SPDX-License-Identifier: MPL-2.0

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // First, compile mqttv3 and mqttv5 proto files individually
    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("mqttv3_descriptor.bin"))
        .compile_protos(&["proto/mqttv3.proto"], &["proto"])?;

    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("mqttv5_descriptor.bin"))
        .compile_protos(&["proto/mqttv5.proto"], &["proto"])?;

    // Then compile the unified mqtt.proto with extern_path pointing to the generated modules
    tonic_prost_build::configure()
        .file_descriptor_set_path(out_dir.join("mqtt_descriptor.bin"))
        .extern_path(".mqttv3", "crate::mqttv3pb")
        .extern_path(".mqttv5", "crate::mqttv5pb")
        .compile_protos(&["proto/mqtt.proto"], &["proto"])?;

    Ok(())
}
