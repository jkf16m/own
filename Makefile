.PHONY: badge update test install clean

# Get ownership percentage
PERCENTAGE := $(shell own extract --format json 2>/dev/null | python3 -c "import sys,json; print(f'{json.load(sys.stdin)[\"percentage\"]:.1f}')" 2>/dev/null || echo "0.0")

# Badge color based on percentage
BADGE_COLOR := $(shell if [ $(shell echo "$(PERCENTAGE) >= 80" | bc -l 2>/dev/null || echo 0) -eq 1 ]; then echo "brightgreen"; elif [ $(shell echo "$(PERCENTAGE) >= 50" | bc -l 2>/dev/null || echo 0) -eq 1 ]; then echo "yellow"; else echo "red"; fi)

# Shield.io badge URL
BADGE_URL := https://img.shields.io/badge/human%20reviewed-$(PERCENTAGE)%25-$(BADGE_COLOR)

## Update badge in README.md
badge:
	@echo "Updating badge: $(PERCENTAGE)%"
	@sed -i 's|https://img.shields.io/badge/human%20reviewed-[^)]*|$(BADGE_URL)|g' README.md
	@echo "Badge updated in README.md"

## Update badge and show result
update: badge
	@echo "README.md badge: $(BADGE_URL)"
	@grep -o 'https://img.shields.io/badge/human%20reviewed-[^)]*' README.md

## Run tests
test:
	cargo test

## Install
install:
	cargo install --path .

## Clean
clean:
	cargo clean

## Show current status
status:
	@own status

## Show help
help:
	@echo "Usage:"
	@echo "  make badge    - Update badge in README.md"
	@echo "  make update   - Update badge and show result"
	@echo "  make test     - Run unit tests"
	@echo "  make install  - Install own binary"
	@echo "  make clean    - Clean build artifacts"
	@echo "  make status   - Show ownership status"
	@echo "  make help     - Show this help"
