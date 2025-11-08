#!/bin/bash
# compile.sh
# Compile Python files to .mpy bytecode for faster execution on Pico 2

set -e  # Exit on error

# Parse arguments
MINIFY=false
MODE="development"
OPTIMIZE_LEVEL=""  # Will be set based on mode if not specified

while [[ $# -gt 0 ]]; do
    case $1 in
        --production|--minify)
            MINIFY=true
            MODE="production"
            shift
            ;;
        --optimize|-O)
            OPTIMIZE_LEVEL="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: ./compile.sh [--production] [--optimize N]"
            echo "  --production    : Full optimization (minify + remove prints)"
            echo "  --optimize N    : Set bytecode optimization level (0-3)"
            exit 1
            ;;
    esac
done

# Set default optimization levels if not specified
if [ -z "$OPTIMIZE_LEVEL" ]; then
    if [ "$MODE" = "production" ]; then
        OPTIMIZE_LEVEL=3  # Maximum optimization for production
    else
        OPTIMIZE_LEVEL=0  # No optimization for development (keeps assertions)
    fi
fi

echo "======================================"
echo "Pico Kiln Bytecode Compiler"
echo "======================================"

# Check if mpy-cross is installed
if ! command -v mpy-cross &> /dev/null; then
    echo "Error: mpy-cross is not installed"
    echo "Install with: pip install mpy-cross"
    exit 1
fi

# Check if python-minifier is installed (if needed)
if [ "$MINIFY" = true ]; then
    if ! python3 -c "import python_minifier" 2>/dev/null; then
        echo "Error: python-minifier is not installed"
        echo "Install with: pip install python-minifier"
        exit 1
    fi
fi

# Architecture flags from mpy-detect.py output
# Pico 2 uses armv7emsp (ARMv7E-M with single precision FPU)
ARCH_FLAGS="-march=armv7emsp -O${OPTIMIZE_LEVEL}"

echo "Using architecture: armv7emsp"
echo "Build mode: $MODE"
echo "Bytecode optimization: -O${OPTIMIZE_LEVEL}"
if [ "$MINIFY" = true ]; then
    echo "  - Removing print statements"
    echo "  - Removing docstrings and comments"
    echo "  - Minifying code"
    echo "  - Preserving type annotations"
fi
echo ""

# Create build directory
BUILD_DIR="build"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

# Create temp directory for minification if needed
if [ "$MINIFY" = true ]; then
    TEMP_DIR="build/.temp"
    mkdir -p "$TEMP_DIR"
fi

echo "Compiling Python modules to bytecode..."
echo ""

# Function to compile a Python file
compile_file() {
    local src_file="$1"
    local dest_dir="$2"
    local filename=$(basename "$src_file")
    local base_name="${filename%.py}"

    mkdir -p "$dest_dir"

    local processed_file="$src_file"

    # Production: remove prints + minify
    if [ "$MINIFY" = true ]; then
        # Step 1: Remove print statements using AST
        local temp_no_prints="$TEMP_DIR/no_prints_${filename}"
        python3 scripts/remove_prints.py "$src_file" "$temp_no_prints"

        # Step 2: Minify the result
        local temp_minified="$TEMP_DIR/minified_${filename}"
        python3 -m python_minifier \
            --no-remove-annotations \
            --remove-literal-statements \
            --output "$temp_minified" \
            "$temp_no_prints"

        processed_file="$temp_minified"
        echo "  -> Compiling $src_file (prints removed + minified)"
    else
        echo "  -> Compiling $src_file"
    fi

    mpy-cross $ARCH_FLAGS -o "$dest_dir/${base_name}.mpy" "$processed_file"
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

# Clean up temp directory if it exists
if [ -d "$TEMP_DIR" ]; then
    rm -rf "$TEMP_DIR"
fi

echo ""
echo "======================================"
echo "Compilation complete!"
echo "======================================"
echo ""
echo "Compiled files are in: $BUILD_DIR/"
echo ""

# Build mode summary
case $MODE in
    "production")
        echo "Production build complete!"
        echo "  ✓ Print statements removed"
        echo "  ✓ Docstrings removed"
        echo "  ✓ Comments removed"
        echo "  ✓ Code minified"
        echo "  ✓ Type annotations preserved"
        echo "  ✓ Bytecode optimization: -O${OPTIMIZE_LEVEL} (maximum)"
        ;;
    "development")
        echo "Development build complete!"
        echo "  • Print statements included"
        echo "  • Docstrings and comments included"
        echo "  • Bytecode optimization: -O${OPTIMIZE_LEVEL} (debug friendly)"
        echo "Tip: Use './compile.sh --production' for deployment (smaller, faster, no logs)"
        ;;
esac

echo ""
echo "Next steps:"
echo "  1. Review compiled files: ls -lR $BUILD_DIR/"
echo "  2. Deploy to Pico: ./deploy.sh"
echo ""

# Show size comparison
echo "Size comparison:"
ORIGINAL_SIZE=$(du -sh . 2>/dev/null | cut -f1)
COMPILED_SIZE=$(du -sh "$BUILD_DIR" 2>/dev/null | cut -f1)
echo "  Original source:  $ORIGINAL_SIZE"
echo "  Compiled build:   $COMPILED_SIZE"
echo ""

echo "Build modes:"
echo "  ./compile.sh                    - Development (prints + docstrings, -O0)"
echo "  ./compile.sh --production       - Production (optimized, minified, -O3)"
echo "  ./compile.sh --optimize N       - Development with custom optimization (0-3)"
echo "  ./compile.sh --production -O 2  - Production with custom optimization"
echo ""
echo "Optimization levels:"
echo "  -O0: No optimization (keeps assertions, best for debugging)"
echo "  -O1: Basic optimization (removes assertions)"
echo "  -O2: Standard optimization"
echo "  -O3: Maximum optimization (smallest, fastest)"
echo ""
