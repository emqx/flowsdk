"""Regression tests for Cargo artifact lookup in the feature-contract checker."""

import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import check_feature_contracts as contracts


class ArtifactLookupTests(unittest.TestCase):
    def check_layout(self, layout):
        with tempfile.TemporaryDirectory(prefix="feature-contract-layout-") as temporary:
            root = Path(temporary)
            if layout == "conventional":
                export = root / "target/debug"
                dependency = leaf = export / "deps"
            elif layout == "flat":
                export = dependency = leaf = root / "artifacts"
            else:
                export = root / "exports/debug"
                dependency = root / "separate build/dependency"
                leaf = root / "separate build/leaf"

            def compile_library(name, source, directory, dependencies=()):
                directory.mkdir(parents=True, exist_ok=True)
                library = directory / f"lib{name}.rlib"
                command = ["rustc", "--edition=2021", "--crate-type=rlib",
                           "--crate-name", name, "-", "-o", str(library)]
                for name, path in dependencies:
                    command += ["--extern", f"{name}={path}",
                                "-L", f"dependency={path.parent}"]
                result = subprocess.run(command, input=source, text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                return library

            leaf_library = compile_library("leaf", "pub struct Token;", leaf)
            dependency_library = compile_library(
                "dependency", "pub struct Token(pub leaf::Token);", dependency,
                [("leaf", leaf_library)])
            library = compile_library(
                "flowsdk",
                "pub mod mqtt_client {\n"
                "pub use dependency::Token as MqttEngine;\n"
                "pub use dependency::Token as PortableMqttEngine;\n"
                "pub use dependency::Token as PortableNoIoMqttClient;\n"
                "}\n",
                export, [("dependency", dependency_library), ("leaf", leaf_library)])

            def artifact(name, library, features, fresh):
                return {"reason": "compiler-artifact", "target": {"name": name},
                        "filenames": [str(library)], "features": features, "fresh": fresh}

            # Only the requested library's features determine API availability.
            # Cargo reports dependencies even when their cached artifacts are fresh.
            events = [
                artifact("leaf", leaf_library, [], True),
                artifact("dependency", dependency_library, ["std"], True),
                artifact("flowsdk", library, ["alloc"], False),
                {"reason": "build-finished", "success": True},
            ]
            real_run = contracts.run

            def run(command, **kwargs):
                if command[:2] == ["cargo", "build"]:
                    # Reuse the small fixture for each API-gate configuration;
                    # the external consumers are still compiled by real rustc.
                    return subprocess.CompletedProcess(
                        command, 0, "\n".join(map(json.dumps, events)), "")
                return real_run(command, **kwargs)

            consumer = root / "consumer"
            consumer.mkdir()
            with patch.object(contracts, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
                contracts.check_rust_apis(consumer, offline=True)

    def test_conventional_dependency_directory(self):
        self.check_layout("conventional")

    def test_flat_artifact_directory(self):
        self.check_layout("flat")

    def test_separate_build_directories_with_cached_dependencies(self):
        self.check_layout("separate")


if __name__ == "__main__":
    unittest.main()
