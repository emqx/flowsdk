"""Local broker integration tests for coverage (requires mosquitto and built binaries)."""

import asyncio
import contextlib
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import uuid

import flowsdk


BIN = Path(os.environ.get("FLOWSDK_COVERAGE_BIN", "target/llvm-cov-target/debug")).resolve()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@contextlib.contextmanager
def process(command):
    with tempfile.TemporaryFile(mode="w+") as output:
        child = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        try:
            yield child, output
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
            output.seek(0)
            if child.returncode not in (0, -15):
                print(output.read())


def wait_port(child, port):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError("Service exited before opening port {}".format(port))
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                return
        except OSError:
            time.sleep(0.05)
    raise TimeoutError("Service did not open port {}".format(port))


class NetworkTests(unittest.IsolatedAsyncioTestCase):
    @classmethod
    def setUpClass(cls):
        cls.services = contextlib.ExitStack()
        cls.addClassCleanup(cls.services.close)
        cls.port = free_port()
        broker, _ = cls.services.enter_context(process(["mosquitto", "-p", str(cls.port)]))
        wait_port(broker, cls.port)
        grpc_port = free_port()
        server, _ = cls.services.enter_context(process([
            str(BIN / "s-proxy"), str(grpc_port), "127.0.0.1:{}".format(cls.port)]))
        wait_port(server, grpc_port)
        cls.proxy_port = free_port()
        receiver, _ = cls.services.enter_context(process([
            str(BIN / "r-proxy"), str(cls.proxy_port), "127.0.0.1:{}".format(grpc_port)]))
        wait_port(receiver, cls.proxy_port)

    async def test_python_broker_workflows(self):
        env = dict(os.environ, FLOWSDK_BROKER_HOST="127.0.0.1",
            FLOWSDK_BROKER_TCP_PORT=str(self.port), FLOWSDK_BROKER_TRANSPORTS="tcp")
        result = await asyncio.to_thread(subprocess.run,
            [sys.executable, "-W", "error", "-m", "unittest", "discover", "-s",
             "python/tests", "-p", "test_broker.py", "-v"],
            env=env, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    async def test_proxy_publish_subscribe_all_versions_and_qos(self):
        for version in (3, 5):
            messages = asyncio.Queue()
            topic = "coverage/proxy/" + uuid.uuid4().hex
            client = flowsdk.FlowMqttClient(
                client_id="proxy-" + uuid.uuid4().hex, mqtt_version=version,
                on_message_full=messages.put_nowait)
            try:
                result = await client.connect("127.0.0.1", self.proxy_port, return_result=True)
                self.assertEqual(result.reason_code, 0)
                (await client.subscribe(topic, qos=2)).raise_for_status()
                for qos in (0, 1, 2):
                    payload = bytes([version, qos, 0, 255])
                    properties = (flowsdk.PublishProperties(
                        content_type="application/octet-stream",
                        correlation_data=b"\x00\xff",
                        user_properties=[("key", "one"), ("key", "two")])
                        if version == 5 else None)
                    result = await client.publish(topic, payload, qos=qos,
                        properties=properties, return_result=True)
                    result.raise_for_status()
                    received = await asyncio.wait_for(messages.get(), 5)
                    self.assertEqual(received.payload, payload)
                    self.assertEqual(received.qos, qos)
                    if properties:
                        for prop in properties.to_ffi():
                            self.assertIn(prop, received.properties)
                (await client.unsubscribe(topic)).raise_for_status()
            finally:
                await client.disconnect()

    def benchmark_command(self, *extra):
        return [str(BIN / "mqtt_ring_bench"), "--host", "127.0.0.1",
            "--port", str(self.port), "--clients", "2", "--messages", "4",
            "--workers", "1", "--interval", "1", "--ifaddr", "127.0.0.1",
            "--shutdown-mode", "graceful", *extra]

    def assert_stat(self, output, label, expected):
        found = re.search(r"^\s*" + re.escape(label) + r":\s*(\d+)\s*$", output, re.M)
        self.assertIsNotNone(found, output)
        self.assertEqual(int(found.group(1)), expected, output)

    async def test_benchmark_publishes_all_versions_and_qos(self):
        for version in (3, 5):
            for qos in (0, 1, 2):
                with self.subTest(version=version, qos=qos), process(self.benchmark_command(
                        "--mqtt-version", str(version), "--qos", str(qos))) as (child, output):
                    await asyncio.to_thread(child.wait, timeout=20)
                    output.seek(0)
                    text = output.read()
                    self.assertEqual(child.returncode, 0, text)
                    self.assert_stat(text, "Total Sent", 8)
                    if qos:
                        self.assert_stat(text, "Total Acked", 8)
                    self.assert_stat(text, "Errors", 0)

    async def test_benchmark_receives_subscribed_messages(self):
        topic = "coverage/bench/" + uuid.uuid4().hex
        with process(self.benchmark_command("--action", "sub", "--qos", "2",
                "--topic", topic)) as (child, output):
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                # Read without moving the file offset shared with the child.
                snapshot = os.pread(output.fileno(), os.fstat(output.fileno()).st_size, 0)
                if b"Subscribed: 2/2" in snapshot:
                    break
                self.assertIsNone(child.poll(), "Subscriber exited early")
                await asyncio.sleep(0.05)
            else:
                self.fail("Benchmark did not subscribe")
            client = flowsdk.FlowMqttClient(client_id="benchmark-publisher")
            try:
                await client.connect("127.0.0.1", self.port)
                for _ in range(4):
                    await client.publish(topic, b"delivery", qos=2)
                await asyncio.to_thread(child.wait, timeout=20)
            finally:
                await client.disconnect()
            output.seek(0)
            text = output.read()
            self.assertEqual(child.returncode, 0, text)
            self.assert_stat(text, "Total Received", 8)
            self.assert_stat(text, "Errors", 0)


if __name__ == "__main__":
    unittest.main()
