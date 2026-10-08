"""Reject coverage exports that omit workspace or portable API execution data."""

import argparse
from pathlib import Path


# Keep an exercised production file from each workspace crate in the upload.
# A root-package-only report can pass 80% while silently dropping all FFI tests.
REQUIRED_SOURCES = (
    "src/mqtt_client/engine.rs",
    "src/mqtt_client/no_io_client.rs",
    "flowsdk_ffi/src/engine.rs",
    "mqtt_grpc_duality/src/lib.rs",
    "mqtt_ring_bench/src/main.rs",
)

# These explicit-time wrappers are exercised by the separate portable suite.
# File-level hits alone can come from host tests even when portable binaries
# or their generic function instrumentation are missing from the report.
PORTABLE_SOURCE = "src/mqtt_client/no_io_client.rs"
REQUIRED_PORTABLE_FUNCTIONS = ("publish_at", "subscribe_at", "handle_tick_at", "puback_at")


def check_report(path):
    hits = {}
    portable_hits = {name: 0 for name in REQUIRED_PORTABLE_FUNCTIONS}
    for record in path.read_text().split("end_of_record"):
        fields = dict(line.split(":", 1) for line in record.splitlines() if ":" in line)
        source = fields.get("SF", "").replace("\\", "/")
        for required in REQUIRED_SOURCES:
            if source == required or source.endswith("/" + required):
                hits[required] = hits.get(required, 0) + int(fields.get("LH", "0"))
        if source == PORTABLE_SOURCE or source.endswith("/" + PORTABLE_SOURCE):
            for line in record.splitlines():
                if line.startswith("FNDA:"):
                    count, symbol = line[5:].split(",", 1)
                    for name in portable_hits:
                        if name in symbol:
                            portable_hits[name] += int(count)
    missing = [source for source in REQUIRED_SOURCES if hits.get(source, 0) == 0]
    if missing:
        raise SystemExit("Coverage report is missing execution data for: " + ", ".join(missing))
    missing_portable = [name for name, count in portable_hits.items() if count == 0]
    if missing_portable:
        raise SystemExit(
            "Coverage report is missing portable API execution data for: "
            + ", ".join(missing_portable)
        )
    for source in REQUIRED_SOURCES:
        print("{}: {} covered lines".format(source, hits[source]))
    for name, count in portable_hits.items():
        print("NoIoMqttClient.{}: {} calls".format(name, count))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    check_report(parser.parse_args().report)
