PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
CARGO ?= cargo +stable
CODEX_RS := codex-rs
BIN := bitter-codex

.PHONY: install-debug install-release clean-target

install-debug:
	cd $(CODEX_RS) && $(CARGO) build -p codex-cli --bin $(BIN)
	install -d "$(BINDIR)"
	install -m 0755 "$(CODEX_RS)/target/debug/$(BIN)" "$(BINDIR)/$(BIN)"
	cd $(CODEX_RS) && $(CARGO) clean

install-release:
	cd $(CODEX_RS) && $(CARGO) build --release -p codex-cli --bin $(BIN)
	install -d "$(BINDIR)"
	install -m 0755 "$(CODEX_RS)/target/release/$(BIN)" "$(BINDIR)/$(BIN)"
	cd $(CODEX_RS) && $(CARGO) clean

clean-target:
	cd $(CODEX_RS) && $(CARGO) clean
