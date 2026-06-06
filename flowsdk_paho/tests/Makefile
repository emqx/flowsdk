CARGO_TARGET ?= ../../target/debug
INCLUDE      := ../include

CC      := cc
CFLAGS  := -I$(INCLUDE) -Wall -Wextra
LDFLAGS := -L$(CARGO_TARGET) -lflowsdk_paho -lpthread -ldl -lm

.PHONY: all clean run

all: paho_smoke

paho_smoke: paho_smoke.c
	$(CC) $(CFLAGS) $< $(LDFLAGS) -o $@

run: paho_smoke
	DYLD_LIBRARY_PATH=$(CARGO_TARGET) LD_LIBRARY_PATH=$(CARGO_TARGET) ./paho_smoke

clean:
	rm -f paho_smoke
