#!/bin/bash
# Build all examples and convert ELFs to flashable BINs

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
FLASH_STATION="$PROJECT_ROOT/flash-station"
CONVERT_SCRIPT="$FLASH_STATION/scripts/cargo_elf_to_esptool.sh"
TARGET_DIR="$SCRIPT_DIR/target/xtensa-esp32s3-none-elf/release/examples"
OUTPUT_DIR="$SCRIPT_DIR/target/binaries"

if [ ! -f "$CONVERT_SCRIPT" ]; then
    echo "Error: Conversion script not found at $CONVERT_SCRIPT"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

echo "Building all examples..."
cargo build --release --examples

echo ""
echo "Converting ELFs to BINs..."

# Convert only the clean binary names (without hash suffixes)
# The hash pattern is -[16 hex chars], so we exclude those
for elf in "$TARGET_DIR"/*; do
    if [ -f "$elf" ] && [ -x "$elf" ]; then
        name=$(basename "$elf")
        # Skip files with hash suffix pattern (e.g., "backlight-0bddedc183aec442")
        if [[ ! "$name" =~ -[a-f0-9]{16}$ ]]; then
            output="$OUTPUT_DIR/$name.bin"
            echo "Converting $name..."
            "$CONVERT_SCRIPT" "$elf" "$output"
        fi
    fi
done

echo ""
echo "✓ All examples built and converted!"
echo "Binaries available in: $OUTPUT_DIR"
