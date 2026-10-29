#!/usr/bin/env python3
"""Check feature contracts against independently built production libraries."""

import argparse
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def run(command, *, expect_success=True):
    print("+ " + shlex.join(map(str, command)), flush=True)
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if expect_success and result.returncode:
        print(result.stdout)
        print(result.stderr, file=sys.stderr)
        raise RuntimeError(f"Command failed with status {result.returncode}")
    return result


def cargo_args(package, features, offline):
    args = ["--locked", "-p", package, "--no-default-features"]
    if features:
        args += ["--features", features]
    if offline:
        args += ["--offline"]
    return args


def build(package, features, offline):
    result = run(["cargo", "build", *cargo_args(package, features, offline),
                  "--lib", "--message-format=json"])
    artifacts = []
    enabled = set()
    for line in result.stdout.splitlines():
        event = json.loads(line)
        if event.get("reason") == "compiler-artifact" and event["target"]["name"] == package:
            artifacts.extend(map(Path, event["filenames"]))
            enabled.update(event["features"])
    return artifacts, enabled


def check_dependencies(package, features, forbidden, offline):
    result = run(["cargo", "tree", *cargo_args(package, features, offline),
                  "-e", "normal", "--prefix", "none", "--format", "{p} {f}"])
    print(result.stdout)
    names = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
    leaked = names & forbidden
    if leaked:
        raise RuntimeError(f"{package} unexpectedly includes {sorted(leaked)}")


def check_rust_apis(directory, offline):
    apis = {
        "std": "MqttClient",
        "async-client": "TokioAsyncMqttClient",
        "rustls-tls": "TlsMqttEngine",
        "quic-proto": "QuicMqttEngine",
        "protocol-testing": "raw_packet::RawPacketBuilder",
        "durable-session": "ClientSessionState",
    }
    for features in ["alloc", "arbitrary", *(f"alloc,{feature}" for feature in apis)]:
        artifacts, enabled_features = build("flowsdk", features, offline)
        library = next(path for path in artifacts if path.suffix == ".rlib")
        dependencies = library.parent / "deps"
        if not dependencies.is_dir():
            dependencies = library.parent
        snapshot = directory / "libflowsdk.rlib"
        shutil.copyfile(library, snapshot)
        # rustc compiles an external consumer using the exact production artifact;
        # no dev dependencies or workspace feature unification can enable its API.
        for feature, api in [("", "MqttEngine"), ("", "PortableMqttEngine"),
                             ("", "PortableNoIoMqttClient"), *apis.items()]:
            source = directory / "consumer.rs"
            source.write_text(f"use flowsdk::mqtt_client::{api};\n"
                              f"pub fn require_api(_: Option<{api.split('::')[-1]}>) {{}}\n")
            result = run(["rustc", "--edition=2021", "--crate-type=lib", source,
                          "--emit=metadata", "-o", directory / "consumer.rmeta",
                          "--extern", f"flowsdk={snapshot}", "-L", f"dependency={dependencies}"],
                         expect_success=False)
            enabled = not feature or feature in enabled_features
            if enabled and result.returncode:
                raise RuntimeError(result.stderr)
            if not enabled:
                if not result.returncode or "E0432" not in result.stderr or api.split('::')[-1] not in result.stderr:
                    raise RuntimeError(f"Expected unavailable API {api}:\n{result.stderr}")
            print(f"  {features or 'minimal'}: {api} {'available' if enabled else 'absent'}")


def exported_symbols(library):
    flags = ["-gU"] if sys.platform == "darwin" else ["-g", "--defined-only"]
    result = run(["nm", *flags, library])
    return {line.split()[-1].removeprefix("_") for line in result.stdout.splitlines() if line.split()}


def check_c_exports(offline):
    gates = {
        "json": "mqtt_engine_new_v1",
        "durable-session": "mqtt_engine_snapshot_session",
        "quic": "mqtt_quic_engine_new",
    }
    for features in ["", "json", "durable-session", "quic", "tls", "uniffi-bindings", "tls,quic-openssl,durable-session"]:
        artifacts, enabled = build("flowsdk_ffi", features, offline)
        library = next(path for path in artifacts if path.suffix in {".so", ".dylib"})
        symbols = exported_symbols(library)
        for feature, symbol in gates.items():
            if (symbol in symbols) != (feature in enabled):
                raise RuntimeError(f"{features or 'minimal'}: incorrect export gate for {symbol}")
        for symbol in ["mqtt_engine_new", "mqtt_tls_engine_new"]:
            if symbol not in symbols:
                raise RuntimeError(f"Missing stable C export {symbol}")
        has_uniffi = any(symbol.startswith("uniffi_flowsdk_ffi") for symbol in symbols)
        if has_uniffi != ("uniffi-bindings" in enabled):
            raise RuntimeError(f"{features or 'minimal'}: incorrect UniFFI export gate")
        print(f"  {features or 'minimal'}: C export gates passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="Use cached Cargo dependencies only")
    args = parser.parse_args()
    # The workspace lockfile is intentionally untracked. Resolve it before
    # checking each configuration with --locked.
    run(["cargo", "fetch", *(["--offline"] if args.offline else [])])
    transports = {"quinn", "quinn-proto", "rustls", "tokio-rustls", "native-tls", "tokio-native-tls"}
    missing_alloc = run(["cargo", "check", *cargo_args("flowsdk", "", args.offline), "--lib"],
                        expect_success=False)
    if not missing_alloc.returncode or "FlowSDK requires the alloc or std feature" not in missing_alloc.stderr:
        raise RuntimeError(f"Expected the missing-alloc diagnostic:\n{missing_alloc.stderr}")
    core_only = transports | {"tonic", "prost", "tonic-prost", "tokio", "async-trait", "thiserror"}
    check_dependencies("flowsdk", "alloc", core_only | {"serde_json"}, args.offline)
    check_dependencies("flowsdk", "std", core_only, args.offline)
    check_dependencies("flowsdk", "default", {"quinn", "quinn-proto", "rustls", "tokio-rustls"}, args.offline)
    check_dependencies("flowsdk_ffi", "", transports | {"uniffi", "uniffi_macros"}, args.offline)
    with tempfile.TemporaryDirectory(prefix="flowsdk-feature-contracts-") as temporary:
        check_rust_apis(Path(temporary), args.offline)
    check_c_exports(args.offline)
    print("All feature contracts passed.")


if __name__ == "__main__":
    main()
