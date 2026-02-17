#!/usr/bin/env python3
"""
Convert PNG to raw RGB565 data for embedding in Rust code.
Usage: python3 convert_png.py input.png output.bin
This file generated with GitHub Copilot
"""

import sys
from PIL import Image
import struct


def rgb888_to_rgb565(r, g, b):
    """Convert RGB888 to RGB565"""
    # Scale down to 5-6-5 bits
    r5 = (r * 31) // 255
    g6 = (g * 63) // 255
    b5 = (b * 31) // 255

    # Pack into 16-bit value (big endian)
    rgb565 = (r5 << 11) | (g6 << 5) | b5
    return rgb565


def convert_png_to_rgb565(input_path, output_path):
    """Convert PNG to raw RGB565 binary data"""
    try:
        # Open and convert to RGB
        img = Image.open(input_path).convert("RGB")
        width, height = img.size

        print(f"Converting {input_path}: {width}x{height}")

        # Convert each pixel to RGB565
        rgb565_data = []
        for y in range(height):
            for x in range(width):
                r, g, b = img.getpixel((x, y))
                rgb565 = rgb888_to_rgb565(r, g, b)
                rgb565_data.append(rgb565)

        # Write binary data (little endian for ESP32)
        with open(output_path, "wb") as f:
            f.write(struct.pack("<HH", width, height))  # Little endian 16-bit
            for pixel in rgb565_data:
                f.write(struct.pack("<H", pixel))  # Little endian 16-bit

        print(
            f"Wrote {len(rgb565_data)} pixels ({len(rgb565_data) * 2} bytes) to {output_path}"
        )
        print(f"Add this to your Rust code:")
        print(f'const EMBEDDED_IMAGE_DATA: &[u8] = include_bytes!("{output_path}");')
        print(
            f"const EMBEDDED_IMAGE: EmbeddedImage = EmbeddedImage::new(EMBEDDED_IMAGE_DATA);"
        )

    except Exception as e:
        print(f"Error: {e}")
        return False

    return True


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Usage: python3 convert_png.py input.png output.bin")
        sys.exit(1)

    input_file = sys.argv[1]
    output_file = sys.argv[2]

    if convert_png_to_rgb565(input_file, output_file):
        print("Conversion successful!")
    else:
        print("Conversion failed!")
        sys.exit(1)
