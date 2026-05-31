PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
CARGO ?= cargo +stable
CODEX_RS := codex-rs
BIN := bitter-codex

.PHONY: install-debug clean-target

install-debug:
	cd $(CODEX_RS) && $(CARGO) build -p codex-cli --bin $(BIN)
	install -d "$(BINDIR)"
	install -m 0755 "$(CODEX_RS)/target/debug/$(BIN)" "$(BINDIR)/$(BIN)"
	rm -rf "$(CODEX_RS)/target"

clean-target:
	rm -rf "$(CODEX_RS)/target"
