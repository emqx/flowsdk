import flowsdk_ffi

def test_mqtt_engine():
    print("--- Testing Standard MqttEngineFfi ---")
    opts = flowsdk_ffi.MqttOptionsFfi(
        client_id="python_client",
        mqtt_version=5,
        clean_start=True,
        keep_alive=60,
        username=None,
        password=None,
        reconnect_base_delay_ms=1000,
        reconnect_max_delay_ms=30000,
        max_reconnect_attempts=0
    )
    engine = flowsdk_ffi.MqttEngineFfi.new_with_opts(opts)
    
    print(f"MQTT Version: {engine.get_version()}")
    
    # Simulate connection
    engine.connect()
    
    # Subscribe
    pid = engine.subscribe("test/topic", 1)
    print(f"Subscribe packet_id: {pid}")
    
    # Publish
    pid = engine.publish("test/topic", b"hello world", 1, None)
    print(f"Publish packet_id: {pid}")
    
    # Simulate receiving a message (loopback)
    print("Simulating message reception...")
    msg = flowsdk_ffi.MqttMessageFfi(topic="test/topic", payload=b"hello loopback", qos=1, retain=False)
    event = flowsdk_ffi.MqttEventFfi.MESSAGE_RECEIVED(msg)
    engine.push_event_ffi(event)
    
    # Check events
    events = engine.take_events()
    for ev in events:
        if ev.is_message_received():
            m = ev[0] # The MqttMessageFfi is the first element in the variant tuple
            print(f"RECEIVED PUBLISH: topic={m.topic}, payload={m.payload.decode()}, qos={m.qos}")
        else:
            print(f"Other Event: {ev}")

    # Disconnect
    engine.disconnect()
    print("Disconnected.")

def test_tls_engine():
    print("\n--- Testing TlsMqttEngineFfi ---")
    opts = flowsdk_ffi.MqttOptionsFfi(
        client_id="tls_client",
        mqtt_version=5,
        clean_start=True,
        keep_alive=60,
        username=None,
        password=None,
        reconnect_base_delay_ms=1000,
        reconnect_max_delay_ms=30000,
        max_reconnect_attempts=0
    )
    tls_opts = flowsdk_ffi.MqttTlsOptionsFfi(
        ca_cert_file=None,
        client_cert_file=None,
        client_key_file=None,
        insecure_skip_verify=True,
        alpn_protocols=["mqtt"]
    )
    
    engine = flowsdk_ffi.TlsMqttEngineFfi(opts, tls_opts, "localhost")
    engine.connect()
    
    # Check for outgoing socket data
    data = engine.take_socket_data()
    print(f"Outgoing TLS data: {len(data)} bytes")
    
    engine.disconnect()
    print("Disconnected.")

def test_quic_engine():
    print("\n--- Testing QuicMqttEngineFfi ---")
    opts = flowsdk_ffi.MqttOptionsFfi(
        client_id="quic_client",
        mqtt_version=5,
        clean_start=True,
        keep_alive=60,
        username=None,
        password=None,
        reconnect_base_delay_ms=1000,
        reconnect_max_delay_ms=30000,
        max_reconnect_attempts=0
    )
    
    engine = flowsdk_ffi.QuicMqttEngineFfi(opts)
    
    tls_opts = flowsdk_ffi.MqttTlsOptionsFfi(
        ca_cert_file=None,
        client_cert_file=None,
        client_key_file=None,
        insecure_skip_verify=True,
        alpn_protocols=["mqtt"]
    )
    
    engine.connect("127.0.0.1:1883", "localhost", tls_opts, 0)
    
    # Check for outgoing datagrams
    datagrams = engine.take_outgoing_datagrams()
    print(f"Outgoing QUIC datagrams: {len(datagrams)}")
    
    # Simulate a tick
    events = engine.handle_tick(100)
    print(f"Tick events: {len(events)}")
    
    engine.disconnect()
    print("Disconnected.")

if __name__ == "__main__":
    try:
        test_mqtt_engine()
        test_tls_engine()
        test_quic_engine()
        print("\nAll verification tests passed!")
    except Exception as e:
        print(f"\nVerification Failed: {e}")
        import traceback
        traceback.print_exc()
        exit(1)
