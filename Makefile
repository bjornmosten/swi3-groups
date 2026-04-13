PREFIX ?= /usr/local
BINDIR  = $(PREFIX)/bin

CARGO   = cargo
RELEASE_BIN = i3sets-client/target/release/swi3-groups-client

.PHONY: all build install uninstall clean

all: build

build:
	$(CARGO) build --release --manifest-path i3sets-client/Cargo.toml

install: build
	install -Dm755 $(RELEASE_BIN) $(DESTDIR)$(BINDIR)/swi3-groups-client
	install -Dm755 bin/swi3-groups $(DESTDIR)$(BINDIR)/swi3-groups
	install -Dm644 bin/_swi3-groups-common.sh $(DESTDIR)$(BINDIR)/_swi3-groups-common.sh

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/swi3-groups-client
	rm -f $(DESTDIR)$(BINDIR)/swi3-groups
	rm -f $(DESTDIR)$(BINDIR)/_swi3-groups-common.sh

clean:
	$(CARGO) clean --manifest-path i3sets-client/Cargo.toml
