CC ?= cc
CFLAGS ?= -std=c11 -O2 -Wall -Wextra -I. -Iinclude -Iplugins -Igenerated -Ithird_party/libfoo/include
LDFLAGS ?=
SRC := src/main.c src/graph.c src/platform.c src/util.c plugins/plugin.c third_party/libfoo/src/foo.c
OBJ := $(SRC:.c=.o)
BIN := build/orchard
GENHDR := generated/autogen.h

all: $(BIN)

# --- Generated header ---------------------------------------------------------
$(GENHDR):
	@mkdir -p $(dir $@)
	@echo '/* generated at build time */' > $@
	@echo '#pragma once' >> $@
	@echo '#define ORCHARD_VERSION "1.2.3"' >> $@

# All objects must wait for the generated header to exist
$(OBJ): $(GENHDR)

# --- Compile and link ---------------------------------------------------------
%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

$(BIN): $(OBJ)
	@mkdir -p $(dir $@)
	$(CC) $(OBJ) -o $@ $(LDFLAGS)

# --- Housekeeping -------------------------------------------------------------
clean:
	rm -rf build $(OBJ) $(GENHDR)

.PHONY: all clean
