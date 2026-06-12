CARGO_TARGET ?= ../../target/debug
INCLUDE      := ../include

CC      := cc
CFLAGS  := -I$(INCLUDE) -Wall -Wextra
LDFLAGS := -L$(CARGO_TARGET) -lflowsdk_paho -lpthread -ldl -lm

.PHONY: all clean run run_async run_v5props run_tls run_qos check_symbols

all: paho_smoke paho_async_smoke paho_v5_props_smoke paho_tls_smoke paho_qos_smoke

paho_smoke: paho_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

paho_async_smoke: paho_async_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

paho_v5_props_smoke: paho_v5_props_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

paho_tls_smoke: paho_tls_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

paho_qos_smoke: paho_qos_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

run: paho_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_smoke

run_async: paho_async_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_async_smoke

run_v5props: paho_v5_props_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_v5_props_smoke

run_tls: paho_tls_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_tls_smoke

run_qos: paho_qos_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_qos_smoke

# Audit that the library exports the full Paho symbol surface.
check_symbols:
	sh ../scripts/check_symbols.sh

clean:
	rm -f paho_smoke paho_async_smoke paho_v5_props_smoke paho_tls_smoke paho_qos_smoke
