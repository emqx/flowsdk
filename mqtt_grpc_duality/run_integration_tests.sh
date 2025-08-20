#!/bin/bash

# Exit immediately if a command exits with a non-zero status.
set -e

# --- Configuration ---
S_PROXY_GRPC_PORT=50055
R_PROXY_MQTT_PORT=1883
BROKER_PORT=1884
BROKER_HOST="127.0.0.1"
S_PROXY_TARGET_BROKER="$BROKER_HOST:$BROKER_PORT"
R_PROXY_TARGET_S_PROXY="127.0.0.1:$S_PROXY_GRPC_PORT"
PAHO_TEST_DIR="paho.mqtt.testing"

# --- Cleanup Function ---
cleanup() {
    echo "--- Cleaning up ---"
    # Stop background jobs by PID
    if [ -n "$R_PROXY_PID" ]; then
        echo "Stopping r-proxy (PID: $R_PROXY_PID)..."
        kill "$R_PROXY_PID" 2>/dev/null || true
    fi
    if [ -n "$S_PROXY_PID" ]; then
        echo "Stopping s-proxy (PID: $S_PROXY_PID)..."
        kill "$S_PROXY_PID" 2>/dev/null || true
    fi
    if [ -n "$BROKER_PID" ]; then
        echo "Stopping Mosquitto broker (PID: $BROKER_PID)..."
        kill "$BROKER_PID" 2>/dev/null || true
    fi
    echo "Cleanup complete."
}

# Trap EXIT signal to ensure cleanup runs
trap cleanup EXIT

# --- Prerequisites Check ---
if ! command -v mosquitto &> /dev/null; then
    echo "Error: mosquitto could not be found. Please install it first."
    exit 1
fi
if ! command -v cargo &> /dev/null; then
    echo "Error: cargo could not be found. Please install the Rust toolchain."
    exit 1
fi
if ! command -v python3 &> /dev/null; then
    echo "Error: python3 could not be found. Please install Python 3."
    exit 1
fi

# --- Step 1: Start MQTT Broker ---
echo "--- Starting local Mosquitto MQTT Broker ---"
mosquitto -p "$BROKER_PORT" &
BROKER_PID=$!
echo "Broker started with PID $BROKER_PID on port $BROKER_PORT."
sleep 3 # Give broker time to initialize

# --- Step 2: Build Proxies ---
echo "--- Building proxy binaries ---"
cargo build --bin s-proxy
cargo build --bin r-proxy

# --- Step 3: Start Proxies ---
export RUST_LOG=debug
echo "--- Starting s-proxy ---"
./target/debug/s-proxy "$S_PROXY_GRPC_PORT" "$S_PROXY_TARGET_BROKER" &
S_PROXY_PID=$!
echo "s-proxy started with PID $S_PROXY_PID, listening on gRPC port $S_PROXY_GRPC_PORT."

sleep 1

echo "--- Starting r-proxy ---"
./target/debug/r-proxy "$R_PROXY_MQTT_PORT" "$R_PROXY_TARGET_S_PROXY" &
R_PROXY_PID=$!
echo "r-proxy started with PID $R_PROXY_PID, listening on MQTT port $R_PROXY_MQTT_PORT."

echo "Waiting for proxies to initialize..."
sleep 5

# --- Step 4: Run Paho Test Suite ---
echo "--- Preparing and running Paho MQTT Test Suite ---"
if [ ! -d "$PAHO_TEST_DIR" ]; then
    echo "Cloning Paho test suite..."
    git clone https://github.com/eclipse-paho/paho.mqtt.testing.git "$PAHO_TEST_DIR"
fi

cd "$PAHO_TEST_DIR"
VENV_DIR=".venv"

if [ ! -d "$VENV_DIR" ]; then
    echo "Creating Python virtual environment in $PAHO_TEST_DIR/$VENV_DIR..."
    python3 -m venv "$VENV_DIR"
fi

echo "Activating Python virtual environment..."
source "$VENV_DIR/bin/activate"

echo "Installing test dependencies into venv..."
#pip install -r requirements.txt > /dev/null
pip install pytest

echo "Running pytest suite against r-proxy..."
#pytest --host "$BROKER_HOST" --port "$R_PROXY_MQTT_PORT"
cd interoperability;
python3 client_test5.py
TEST_RESULT=$?

echo "Deactivating Python virtual environment..."
deactivate

cd ..

# --- Exit ---
if [ $TEST_RESULT -eq 0 ]; then
    echo "--- All tests passed successfully! ---"
else
    echo "--- Some tests failed. ---"
fi

exit $TEST_RESULT
