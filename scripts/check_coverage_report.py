"""Reject coverage exports that omit a workspace crate or contain no execution data."""

import argparse
from pathlib import Path


# Keep an exercised production file from each workspace crate in the upload.
# A root-package-only report can pass 80% while silently dropping all FFI tests.
REQUIRED_SOURCES = (
    "src/mqtt_client/engine.rs",
    "flowsdk_ffi/src/engine.rs",
    "mqtt_grpc_duality/src/lib.rs",
    "mqtt_ring_bench/src/main.rs",
)


def check_report(path):
    hits = {}
    for record in path.read_text().split("end_of_record"):
        fields = dict(line.split(":", 1) for line in record.splitlines() if ":" in line)
        source = fields.get("SF", "").replace("\\", "/")
        for required in REQUIRED_SOURCES:
            if source == required or source.endswith("/" + required):
                hits[required] = hits.get(required, 0) + int(fields.get("LH", "0"))
    missing = [source for source in REQUIRED_SOURCES if hits.get(source, 0) == 0]
    if missing:
        raise SystemExit("Coverage report is missing execution data for: " + ", ".join(missing))
    for source in REQUIRED_SOURCES:
        print("{}: {} covered lines".format(source, hits[source]))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    check_report(parser.parse_args().report)
