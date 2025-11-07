#!/bin/bash
# compile.sh
# Compile Python files to .mpy bytecode for faster execution on Pico 2

set -e  # Exit on error

echo "======================================"
echo "Pico Kiln Bytecode Compiler"
echo "======================================"

# Check if mpy-cross is installed
if ! command -v mpy-cross &> /dev/null; then
    echo "Error: mpy-cross is not installed"
    echo "Install with: pip install mpy-cross"
    exit 1
fi

# Architecture flags from mpy-detect.py output
# Pico 2 uses armv7emsp (ARMv7E-M with single precision FPU)
ARCH_FLAGS="-march=armv7emsp"

echo "Using architecture: armv7emsp"
echo ""

# Create build directory
BUILD_DIR="build"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

echo "Compiling Python modules to bytecode..."
echo ""

# Function to compile a Python file
compile_file() {
    local src_file="$1"
    local dest_dir="$2"
    local filename=$(basename "$src_file")
    local base_name="${filename%.py}"

    mkdir -p "$dest_dir"

    echo "  -> Compiling $src_file"
    mpy-cross $ARCH_FLAGS -o "$dest_dir/${base_name}.mpy" "$src_file"
}

# Compile lib/ directory
if [ -d "lib" ]; then
    echo "Compiling lib/ modules..."
    mkdir -p "$BUILD_DIR/lib"

    for file in lib/*.py; do
        if [ -f "$file" ]; then
            compile_file "$file" "$BUILD_DIR/lib"
        fi
    done

    # Compile lib subdirectories (e.g., adafruit_bus_device)
    for dir in lib/*/; do
        if [ -d "$dir" ]; then
            dirname=$(basename "$dir")
            mkdir -p "$BUILD_DIR/lib/$dirname"

            for file in "$dir"*.py; do
                if [ -f "$file" ]; then
                    compile_file "$file" "$BUILD_DIR/lib/$dirname"
                fi
            done
        fi
    done
    echo ""
fi

# Compile kiln/ directory
if [ -d "kiln" ]; then
    echo "Compiling kiln/ modules..."
    mkdir -p "$BUILD_DIR/kiln"

    for file in kiln/*.py; do
        if [ -f "$file" ]; then
            compile_file "$file" "$BUILD_DIR/kiln"
        fi
    done
    echo ""
fi

# Compile server/ directory
if [ -d "server" ]; then
    echo "Compiling server/ modules..."
    mkdir -p "$BUILD_DIR/server"

    for file in server/*.py; do
        if [ -f "$file" ]; then
            compile_file "$file" "$BUILD_DIR/server"
        fi
    done
    echo ""
fi

# Copy files that should NOT be compiled
echo "Copying non-compiled files..."

# main.py and boot.py should stay as .py (MicroPython needs these as source)
for file in main.py boot.py config.py; do
    if [ -f "$file" ]; then
        echo "  -> $file (keeping as .py)"
        cp "$file" "$BUILD_DIR/"
    fi
done

# Copy static directory if it exists
if [ -d "static" ]; then
    echo "  -> static/ (non-Python assets)"
    cp -r static "$BUILD_DIR/"
fi

echo ""
echo "======================================"
echo "Compilation complete!"
echo "======================================"
echo ""
echo "Compiled files are in: $BUILD_DIR/"
echo ""
echo "Next steps:"
echo "  1. Review compiled files: ls -lR $BUILD_DIR/"
echo "  2. Deploy to Pico: ./deploy.sh"
echo ""

# Show size comparison
echo "Size comparison:"
ORIGINAL_SIZE=$(du -sh . 2>/dev/null | cut -f1)
COMPILED_SIZE=$(du -sh "$BUILD_DIR" 2>/dev/null | cut -f1)
echo "  Original:  $ORIGINAL_SIZE"
echo "  Compiled:  $COMPILED_SIZE"
echo ""
