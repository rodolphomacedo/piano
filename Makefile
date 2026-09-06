.PHONY: help setup fmt fmt-fix lint test test-audio test-core-no-std bench \
        build release build-wasm build-core-no-std run-studio run-midi audio-check \
        docs pdf clean verify

# .piano.json file `make run-studio` loads. Override with: make run-studio PIANO=path/to/file.json
PIANO   ?= meu-piano.piano.json

# run-studio auto-detects a connected MIDI controller and plays from it when
# one is present. Force it off with MIDI=0; force the wait-for-a-port
# behaviour with MIDI=1. PORT=name picks a controller by a case-insensitive
# substring of its port name (default: the first one found).
MIDI    ?= auto
PORT    ?=

PORT_FLAG := $(if $(PORT),--port '$(PORT)',)

# Color definitions
BOLD    := \033[1m
DIM     := \033[2m
RESET   := \033[0m
BLUE    := \033[34m
GREEN   := \033[32m
YELLOW  := \033[33m
CYAN    := \033[36m

help:
	@clear
	@echo ""
	@echo "$(BOLD)$(BLUE)   ╭─╮ ╭─╮ ╭─╮ ╭─╮ ╭─╮ ╭─╮ ╭─╮ ╭─╮$(RESET)"
	@echo "$(BOLD)$(BLUE)   │ │ │ │ │ │ │ │ │ │ │ │ │ │ │ │   PIANO$(RESET)"
	@echo "$(BOLD)$(BLUE)   ╰─╯ ╰─╯ ╰─╯ ╰─╯ ╰─╯ ╰─╯ ╰─╯ ╰─╯$(RESET)"
	@echo "$(DIM)   Physically-modelled piano synthesiser, in Rust$(RESET)"
	@echo ""
	@echo "$(BOLD)🔧 DEVELOPMENT$(RESET)"
	@echo "  $(CYAN)make fmt$(RESET)              Check formatting (cargo fmt --check)"
	@echo "  $(CYAN)make fmt-fix$(RESET)          Apply formatting (cargo fmt)"
	@echo "  $(CYAN)make lint$(RESET)             Lint & deny warnings (cargo clippy)"
	@echo "  $(CYAN)make test$(RESET)             Run full workspace test suite"
	@echo "  $(CYAN)make test-audio$(RESET)       Run audio timbre diagnostics only"
	@echo "  $(CYAN)make test-core-no-std$(RESET) Test piano-core without std"
	@echo "  $(CYAN)make bench$(RESET)            Run benchmarks (criterion)"
	@echo ""
	@echo "$(BOLD)🏗️  BUILD$(RESET)"
	@echo "  $(CYAN)make build$(RESET)            Build debug binaries"
	@echo "  $(CYAN)make release$(RESET)          Build optimized release"
	@echo "  $(CYAN)make build-wasm$(RESET)       Build WASM module for browser"
	@echo "  $(CYAN)make build-core-no-std$(RESET) Build piano-core without std"
	@echo ""
	@echo "$(BOLD)🎵 RUN$(RESET)"
	@echo "  $(CYAN)make run-studio$(RESET)       Start the studio web UI (PIANO=file.json to pick a file; prompts to create it if missing)"
	@echo "  $(DIM)                     auto-detects a MIDI controller; MIDI=0 disables, PORT=name picks one$(RESET)"
	@echo "  $(CYAN)make run-midi$(RESET)         Play live from a MIDI controller, no web UI (PORT=name to pick one)"
	@echo "  $(CYAN)make audio-check$(RESET)      Validate audio thread safety rules"
	@echo ""
	@echo "$(BOLD)📚 DOCUMENTATION$(RESET)"
	@echo "  $(CYAN)make docs$(RESET)             Generate Rust API docs"
	@echo "  $(CYAN)make pdf$(RESET)              Build docs/pt-BR/COMO-FUNCIONA.pdf"
	@echo ""
	@echo "$(BOLD)🧹 MAINTENANCE$(RESET)"
	@echo "  $(CYAN)make clean$(RESET)            Clean all build artifacts"
	@echo "  $(CYAN)make verify$(RESET)           fmt + lint + test + build-core-no-std"
	@echo ""
	@echo "$(DIM)  Before claiming anything works, run: make verify$(RESET)"
	@echo ""

# ── Development ─────────────────────────────────────────────────────

fmt:
	@echo "$(BOLD)$(CYAN)Checking formatting...$(RESET)"
	cargo fmt --all --check

fmt-fix:
	@echo "$(BOLD)$(CYAN)Applying formatting...$(RESET)"
	cargo fmt --all

lint:
	@echo "$(BOLD)$(CYAN)Running clippy...$(RESET)"
	cargo clippy --workspace --all-targets -- -D warnings

test:
	@echo "$(BOLD)$(CYAN)Running workspace tests...$(RESET)"
	cargo test --workspace

test-audio:
	@echo "$(BOLD)$(CYAN)Running audio timbre diagnostics...$(RESET)"
	cargo test --release -p piano-audio --test timbre_diagnostic -- --nocapture

test-core-no-std:
	@echo "$(BOLD)$(CYAN)Testing piano-core without std...$(RESET)"
	cargo test -p piano-core --no-default-features

bench:
	@echo "$(BOLD)$(CYAN)Running benchmarks...$(RESET)"
	cargo bench --workspace

# ── Build ────────────────────────────────────────────────────────────

build:
	@echo "$(BOLD)$(CYAN)Building debug binaries...$(RESET)"
	cargo build --workspace

release:
	@echo "$(BOLD)$(CYAN)Building optimized release...$(RESET)"
	cargo build --release --workspace

build-wasm:
	@echo "$(BOLD)$(CYAN)Building WASM module...$(RESET)"
	wasm-pack build --target web crates/piano-wasm --release

build-core-no-std:
	@echo "$(BOLD)$(CYAN)Building piano-core (no_std)...$(RESET)"
	cargo build -p piano-core --no-default-features

# ── Run ──────────────────────────────────────────────────────────────

run-studio:
	@if [ ! -f "$(PIANO)" ]; then \
		echo "$(BOLD)$(YELLOW)No piano file at '$(PIANO)'.$(RESET)"; \
		piano_name=""; \
		if exec 3< /dev/tty; then \
			printf "$(CYAN)Name for the new piano [My Piano]: $(RESET)"; \
			read piano_name <&3; exec 3<&-; \
		fi 2>/dev/null; \
		[ -n "$$piano_name" ] || piano_name="My Piano"; \
		printf '{\n  "name": "%s",\n  "defaults": {},\n  "registers": {},\n  "groups": [],\n  "strings": [],\n  "instrument": {\n    "soundboard_modes": [],\n    "bridge": {\n      "local_coupling_gain": 0.15,\n      "global_coupling_gain": 0.08\n    }\n  }\n}\n' "$$piano_name" > "$(PIANO)"; \
		echo "$(GREEN)Wrote $(PIANO) — shape the strings live in the browser once the studio opens.$(RESET)"; \
	fi
	@midi_flag=""; \
	case "$(MIDI)" in \
		0|off|no|false) ;; \
		1|on|yes|true|force) midi_flag="--midi" ;; \
		*) if cargo run -q --release -p piano-cli -- midi --list 2>/dev/null \
		      | grep -q '^available MIDI input ports:'; then \
			midi_flag="--midi"; \
			echo "$(GREEN)MIDI controller detected — playing from it too.$(RESET)"; \
		   fi ;; \
	esac; \
	echo "$(BOLD)$(CYAN)Starting piano studio ($(PIANO))...$(RESET)"; \
	cargo run --release -p piano-cli -- studio --piano $(PIANO) $$midi_flag $(PORT_FLAG)

run-midi:
	@echo "$(BOLD)$(CYAN)Playing live from a MIDI controller...$(RESET)"
	cargo run --release -p piano-cli -- midi $(PORT_FLAG)

audio-check: lint
	@echo "$(BOLD)$(CYAN)Audio thread safety rules (docs/REALTIME-AUDIO-RULES.md):$(RESET)"
	@echo "  $(DIM)clippy denies unwrap/expect/panic/unimplemented outside tests$(RESET)"
	@echo "  $(DIM)no allocation, no locks, no unbounded loops on the audio thread$(RESET)"
	@echo "$(GREEN)Lint gate passed — see docs/REALTIME-AUDIO-RULES.md for the full rules$(RESET)"

# ── Documentation ────────────────────────────────────────────────────

docs:
	@echo "$(BOLD)$(CYAN)Generating Rust API docs...$(RESET)"
	cargo doc --no-deps --workspace

pdf:
	@echo "$(BOLD)$(CYAN)Building docs/pt-BR/COMO-FUNCIONA.pdf...$(RESET)"
	@if command -v pdflatex >/dev/null 2>&1; then \
		cd docs/pt-BR && \
		pdflatex -interaction=nonstopmode COMO-FUNCIONA.tex >/dev/null && \
		pdflatex -interaction=nonstopmode COMO-FUNCIONA.tex >/dev/null && \
		echo "$(GREEN)PDF generated: docs/pt-BR/COMO-FUNCIONA.pdf$(RESET)"; \
	else \
		echo "$(YELLOW)pdflatex not found. Install with:$(RESET)"; \
		echo "$(YELLOW)  brew install --cask basictex   (macOS)$(RESET)"; \
		echo "$(YELLOW)  apt-get install texlive-latex-base   (Linux)$(RESET)"; \
		exit 1; \
	fi

# ── Maintenance ──────────────────────────────────────────────────────

clean:
	@echo "$(BOLD)$(CYAN)Cleaning build artifacts...$(RESET)"
	cargo clean
	@find docs -name "*.aux" -o -name "*.log" -o -name "*.out" -o -name "*.toc" \
		-o -name "*.fls" -o -name "*.fdb_latexmk" | xargs rm -f
	@echo "$(GREEN)Clean complete$(RESET)"

verify: fmt lint test build-core-no-std
	@echo ""
	@echo "$(BOLD)$(GREEN)All verifications passed$(RESET)"
	@echo "  $(DIM)fmt · lint · test · build-core-no-std$(RESET)"

.DEFAULT_GOAL := help
