PREFIX ?= /usr/local
BINDIR  = $(PREFIX)/bin

CARGO   = cargo
RELEASE_BIN = i3sets-client/target/release/swi3-sets-client

.PHONY: all build install uninstall clean

all: build

build:
	$(CARGO) build --release --manifest-path i3sets-client/Cargo.toml

install: build
	install -Dm755 $(RELEASE_BIN) $(DESTDIR)$(BINDIR)/swi3-sets-client
	install -Dm755 bin/swi3-sets $(DESTDIR)$(BINDIR)/swi3-sets
	install -Dm644 bin/_swi3-sets-common.sh $(DESTDIR)$(BINDIR)/_swi3-sets-common.sh

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/swi3-sets-client
	rm -f $(DESTDIR)$(BINDIR)/swi3-sets
	rm -f $(DESTDIR)$(BINDIR)/_swi3-sets-common.sh

clean:
	$(CARGO) clean --manifest-path i3sets-client/Cargo.toml
