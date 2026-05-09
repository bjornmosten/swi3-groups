PREFIX ?= /usr/local
BINDIR  = $(PREFIX)/bin

CARGO   = cargo
RELEASE_BIN = swi3-groups-client/target/release/swi3-groups

.PHONY: all build install uninstall clean

all: build

build:
	$(CARGO) build --release --manifest-path swi3-groups-client/Cargo.toml

install: build
	install -Dm755 $(RELEASE_BIN) $(DESTDIR)$(BINDIR)/swi3-groups-client
	install -Dm755 bin/swi3-groups $(DESTDIR)$(BINDIR)/swi3-groups

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/swi3-groups-client
	rm -f $(DESTDIR)$(BINDIR)/swi3-groups

clean:
	$(CARGO) clean --manifest-path swi3-groups-client/Cargo.toml
