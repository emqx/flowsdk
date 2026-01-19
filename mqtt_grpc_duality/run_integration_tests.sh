#!/bin/bash

# Integration test script for MQTT-gRPC duality proxy system
# Tests MQTT v3.1.1 client and MQTT v5 compatibility
#
# Usage: ./run_integration_tests.sh [--mqtt-ver=v3|v5|both] [TEST_ARGUMENTS]
# Examples:
#   ./run_integration_tests.sh                              # Run all V5 tests (default)
#   ./run_integration_tests.sh --mqtt-ver=v3                 # Run V3 tests only
#   ./run_integration_tests.sh --mqtt-ver=v5                 # Run V5 tests only
#   ./run_integration_tests.sh --mqtt-ver=both               # Run both V3 and V5 tests
#   ./run_integration_tests.sh --mqtt-ver=v5 Test.testBasic  # Run specific V5 test
#   ./run_integration_tests.sh --mqtt-ver=v3 Test.test_keepalive # Run specific V3 test
#
# Exit immediately if a command exits with a non-zero status.
set -e

if [[ $CARGO_LLVM_COV==1 ]]; then
    echo "running with coverage test"
fi

# --- Configuration ---
S_PROXY_GRPC_PORT=50055
R_PROXY_MQTT_PORT=1884

# Real MQTT broker settings
BROKER_PORT=1883
BROKER_HOST="127.0.0.1"
S_PROXY_TARGET_BROKER="$BROKER_HOST:$BROKER_PORT"
R_PROXY_TARGET_S_PROXY="127.0.0.1:$S_PROXY_GRPC_PORT"
PAHO_TEST_DIR="paho.mqtt.testing"

# --- Parse Command Line Arguments ---
MQTT_VERSION="v5"  # Default to v5
TEST_ARGS=()

for arg in "$@"; do
    case $arg in
        --mqtt-ver=*)
            MQTT_VERSION="${arg#*=}"
            shift
            ;;
        *)
            TEST_ARGS+=("$arg")
            shift
            ;;
    esac
done

# Validate version argument
case $MQTT_VERSION in
    v3|v5|both)
        echo "Running tests for MQTT version: $MQTT_VERSION"
        ;;
    *)
        echo "Error: Invalid version '$MQTT_VERSION'. Use v3, v5, or both."
        exit 1
        ;;
esac

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
        kill -9 "$BROKER_PID" 2>/dev/null || true
    fi
    echo "Cleanup complete."
}

# Trap EXIT signal to ensure cleanup runs
trap cleanup EXIT

# --- Prerequisites Check ---
if ! command -v cargo &> /dev/null; then
    echo "Error: cargo could not be found. Please install the Rust toolchain."
    exit 1
fi
if ! command -v python3 &> /dev/null; then
    echo "Error: python3 could not be found. Please install Python 3."
    exit 1
fi

# --- Step 1: Build Proxies ---
echo "--- Building proxy binaries ---"

# only build if not running with coverage
if [[ "${CARGO_LLVM_COV}" != "1" ]]; then
    cargo build --bin s-proxy
    cargo build --bin r-proxy
fi

# --- Step 2: Start Proxies ---
if [ -n "$DEBUG" ]; then
    export RUST_LOG=debug
fi
echo "--- Starting s-proxy ---"
../target/debug/s-proxy "$S_PROXY_GRPC_PORT" "$S_PROXY_TARGET_BROKER" &
S_PROXY_PID=$!
echo "s-proxy started with PID $S_PROXY_PID, listening on gRPC port $S_PROXY_GRPC_PORT."

sleep 1

echo "--- Starting r-proxy ---"
../target/debug/r-proxy "$R_PROXY_MQTT_PORT" "$R_PROXY_TARGET_S_PROXY" &
R_PROXY_PID=$!
echo "r-proxy started with PID $R_PROXY_PID, listening on MQTT port $R_PROXY_MQTT_PORT."

echo "Waiting for proxies to initialize..."
sleep 3

# --- Step 4: Run Paho Test Suite ---
echo "--- Preparing and running Paho MQTT Test Suite ---"
if [ ! -d "$PAHO_TEST_DIR" ]; then
    echo "Cloning Paho test suite..."
    git clone -b main https://github.com/qzhuyan/paho.mqtt.testing.git "$PAHO_TEST_DIR"
fi

cd "$PAHO_TEST_DIR"
VENV_DIR="../.venv"

if [ ! -d "$VENV_DIR" ]; then
    echo "Creating Python virtual environment in $PAHO_TEST_DIR/$VENV_DIR..."
    python3 -m venv "$VENV_DIR"
fi

echo "Activating Python virtual environment..."
source "$VENV_DIR/bin/activate"

#echo "Installing test dependencies into venv..."
#pip install -r requirements.txt > /dev/null
#pip install pytest

#echo "Running pytest suite against r-proxy..."
#pytest --host "$BROKER_HOST" --port "$R_PROXY_MQTT_PORT"

cd interoperability;
echo "--- Starting local Mosquitto MQTT Broker ---"
python3 startbroker.py --port "$BROKER_PORT" &
BROKER_PID=$!
echo "Broker started with PID $BROKER_PID on port $BROKER_PORT."
sleep 3 # Give broker time to initialize

# Run tests based on version selection
TEST_RESULT=0

case $MQTT_VERSION in
    v5)
        echo "Running MQTT v5 client test against r-proxy on port $R_PROXY_MQTT_PORT..."
        python3 client_test5.py -p "$R_PROXY_MQTT_PORT" -v "${TEST_ARGS[@]}"
        TEST_RESULT=$?
        ;;
    v3)
        echo "Running MQTT v3.1.1 client test against r-proxy on port $R_PROXY_MQTT_PORT..."
        python3 client_test.py --hostname=localhost --port="$R_PROXY_MQTT_PORT" "${TEST_ARGS[@]}"
        TEST_RESULT=$?
        ;;
    both)
        echo "Running MQTT v5 client test against r-proxy on port $R_PROXY_MQTT_PORT..."
        python3 client_test5.py -p "$R_PROXY_MQTT_PORT" -v "${TEST_ARGS[@]}"
        V5_RESULT=$?
        
        echo "Running MQTT v3.1.1 client test against r-proxy on port $R_PROXY_MQTT_PORT..."
        python3 client_test.py --hostname=localhost --port="$R_PROXY_MQTT_PORT" "${TEST_ARGS[@]}"
        V3_RESULT=$?
        
        # Return non-zero if either test failed
        if [ $V5_RESULT -ne 0 ] || [ $V3_RESULT -ne 0 ]; then
            TEST_RESULT=1
            echo "V5 test result: $V5_RESULT, V3 test result: $V3_RESULT"
        fi
        ;;
esac

echo "Deactivating Python virtual environment..."
deactivate

cd ..

# --- Exit ---
if [ $TEST_RESULT -eq 0 ]; then
    echo "--- All MQTT $MQTT_VERSION tests passed successfully! ---"
else
    echo "--- Some MQTT $MQTT_VERSION tests failed. ---"
fi

exit $TEST_RESULT
