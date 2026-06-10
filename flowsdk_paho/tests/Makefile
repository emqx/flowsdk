CARGO_TARGET ?= ../../target/debug
INCLUDE      := ../include

CC      := cc
CFLAGS  := -I$(INCLUDE) -Wall -Wextra
LDFLAGS := -L$(CARGO_TARGET) -lflowsdk_paho -lpthread -ldl -lm

.PHONY: all clean run run_async run_v5props

all: paho_smoke paho_async_smoke paho_v5_props_smoke

paho_smoke: paho_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

paho_async_smoke: paho_async_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

paho_v5_props_smoke: paho_v5_props_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

run: paho_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_smoke

run_async: paho_async_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_async_smoke

run_v5props: paho_v5_props_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_v5_props_smoke

clean:
	rm -f paho_smoke paho_async_smoke paho_v5_props_smoke
