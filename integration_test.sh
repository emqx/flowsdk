#!/bin/bash

# Exit immediately if a command exits with a non-zero status.
set -e

# --- Configuration ---
LOG_FILE="/tmp/test_output.log"
PROXY_BIN="s-proxy"
CLIENT_BIN="simple-client"
TEST_TOPIC="test/topic"
TEST_DURATION=5 # How many seconds to run the client for

# --- Cleanup Function ---
# This function is called on script exit to ensure background processes are killed.
cleanup() {
    echo "--- Cleaning up ---"
    # The '|| true' prevents the script from exiting if the process is already gone
    [ ! -z "$PROXY_PID" ] && kill $PROXY_PID || true
    [ ! -z "$MOSQUITTO_PID" ] && kill $MOSQUITTO_PID || true
    [ ! -z "$SUB_PID" ] && kill $SUB_PID || true
    rm -f $LOG_FILE
    echo "Cleanup complete."
}

# Trap the EXIT signal to call the cleanup function.
trap cleanup EXIT

# --- Test Execution ---

# 1. Start Mosquitto Broker
echo "Starting mosquitto broker..."
mosquitto &
MOSQUITTO_PID=$!
sleep 2 # Give the broker a moment to start

# 2. Start the s-proxy
echo "Building and starting $PROXY_BIN..."
cargo build --bin $PROXY_BIN
./target/debug/$PROXY_BIN &
PROXY_PID=$!
sleep 2 # Give the proxy a moment to start

# 3. Start the Subscriber to listen for messages
echo "Starting subscriber to listen on topic '$TEST_TOPIC'..."
mosquitto_sub -h localhost -t "$TEST_TOPIC" -v > $LOG_FILE &
SUB_PID=$!
sleep 2 # Give the subscriber a moment to connect

# 4. Run the Client to send messages
echo "Starting $CLIENT_BIN to send messages for $TEST_DURATION seconds..."
cargo run --bin $CLIENT_BIN &
CLIENT_PID=$!
sleep $TEST_DURATION
kill $CLIENT_PID

# Give a moment for the last message to be delivered
sleep 1

# 5. Verify the results
echo "Verifying results in $LOG_FILE..."

if grep -q "$TEST_TOPIC" "$LOG_FILE"; then
    echo "✅ SUCCESS: Messages were successfully relayed to the broker."
    echo "--- Received Messages ---"
    cat $LOG_FILE
    echo "-------------------------"
else
    echo "❌ FAILURE: No messages were received by the subscriber."
    exit 1
fi
