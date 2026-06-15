#!/bin/bash
#
# debug.sh - Pico Kiln Debug Tool Manager
#
# Manage debug modes for diagnosing standalone boot issues
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Directories
DEBUG_DIR="debug"
BACKUP_DIR=".debug_backups"

# Files
MAIN_PY="main.py"
DEBUG_BOOT="${DEBUG_DIR}/debug_boot.py"
MAIN_SAFE="${DEBUG_DIR}/main_safe.py"
READ_LOGS="${DEBUG_DIR}/read_boot_logs.py"

# Device log files
BOOT_DEBUG_LOG="/boot_debug.log"
BOOT_STAGES_LOG="/boot_stages.log"
BOOT_ERROR_LOG="/boot_error.log"
ERRORS_LOG="/errors.log"

# Functions

print_header() {
    echo -e "${BLUE}=================================${NC}"
    echo -e "${BLUE}  Pico Kiln Debug Tool${NC}"
    echo -e "${BLUE}=================================${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

print_info() {
    echo -e "${BLUE}ℹ $1${NC}"
}

check_mpremote() {
    if ! command -v mpremote &> /dev/null; then
        print_error "mpremote not found. Please install it:"
        echo "  pip install mpremote"
        exit 1
    fi
}

check_pico_connected() {
    if ! mpremote version &> /dev/null; then
        print_error "Pico not connected or not responding"
        echo "  Check USB connection and try again"
        exit 1
    fi
}

backup_main() {
    if [ ! -d "${BACKUP_DIR}" ]; then
        mkdir -p "${BACKUP_DIR}"
        print_info "Created backup directory: ${BACKUP_DIR}"
    fi

    local timestamp=$(date +%Y%m%d_%H%M%S)
    local backup_file="${BACKUP_DIR}/main_${timestamp}.py"

    if mpremote fs cat ":${MAIN_PY}" > "${backup_file}" 2>/dev/null; then
        print_success "Backed up current main.py to ${backup_file}"

        # Also keep a 'latest' backup
        cp "${backup_file}" "${BACKUP_DIR}/main_latest.py"
    else
        print_warning "Could not backup main.py (might not exist on device)"
    fi
}

install_minimal() {
    print_header
    echo "Installing minimal debug boot (LED patterns + logging)"
    echo ""

    check_mpremote
    check_pico_connected

    if [ ! -f "${DEBUG_BOOT}" ]; then
        print_error "Debug boot file not found: ${DEBUG_BOOT}"
        exit 1
    fi

    # Backup current main.py
    backup_main

    # Install debug_boot.py as main.py
    print_info "Deploying ${DEBUG_BOOT} as ${MAIN_PY}..."
    mpremote fs cp "${DEBUG_BOOT}" ":${MAIN_PY}"

    print_success "Minimal debug mode installed!"
    echo ""
    echo "Next steps:"
    echo "  1. Power cycle your Pico (unplug/replug)"
    echo "  2. Watch LED blink patterns:"
    echo "     - 1 blink:  Boot started"
    echo "     - 2 blinks: Basic imports OK"
    echo "     - 3 blinks: Config loaded"
    echo "     - 4 blinks: Project imports OK"
    echo "     - 5 blinks: Success!"
    echo "     - Fast blink (10Hz): Error!"
    echo "  3. Read logs: ./debug.sh --logs"
}

install_safe() {
    print_header
    echo "Installing safe boot mode (full protection + logging)"
    echo ""

    check_mpremote
    check_pico_connected

    if [ ! -f "${MAIN_SAFE}" ]; then
        print_error "Safe boot file not found: ${MAIN_SAFE}"
        exit 1
    fi

    # Backup current main.py
    backup_main

    # Install main_safe.py as main.py
    print_info "Deploying ${MAIN_SAFE} as ${MAIN_PY}..."
    mpremote fs cp "${MAIN_SAFE}" ":${MAIN_PY}"

    print_success "Safe mode installed!"
    echo ""
    echo "Next steps:"
    echo "  1. Power cycle your Pico"
    echo "  2. Watch LED blink patterns (1-9 for boot stages)"
    echo "  3. Solid LED = running, Fast blink = error"
    echo "  4. Monitor logs: ./debug.sh --watch"
}

restore_original() {
    print_header
    echo "Restoring original main.py"
    echo ""

    check_mpremote
    check_pico_connected

    local latest_backup="${BACKUP_DIR}/main_latest.py"

    if [ ! -f "${latest_backup}" ]; then
        print_error "No backup found at ${latest_backup}"
        echo "Available backups:"
        ls -1 "${BACKUP_DIR}"/main_*.py 2>/dev/null || echo "  (none)"
        exit 1
    fi

    print_info "Restoring from ${latest_backup}..."
    mpremote fs cp "${latest_backup}" ":${MAIN_PY}"

    print_success "Original main.py restored!"
    echo ""
    echo "Power cycle your Pico to boot with restored main.py"
}

read_logs() {
    print_header
    echo "Reading boot logs from Pico"
    echo ""

    check_mpremote
    check_pico_connected

    if [ -f "${READ_LOGS}" ]; then
        python3 "${READ_LOGS}"
    else
        print_error "Log reader not found: ${READ_LOGS}"
        exit 1
    fi
}

watch_logs() {
    print_header
    echo "Watching boot logs (Ctrl+C to stop)"
    echo ""

    check_mpremote
    check_pico_connected

    if [ -f "${READ_LOGS}" ]; then
        python3 "${READ_LOGS}" --watch
    else
        print_error "Log reader not found: ${READ_LOGS}"
        exit 1
    fi
}

clean_logs() {
    print_header
    echo "Cleaning debug logs on Pico"
    echo ""

    check_mpremote
    check_pico_connected

    local cleaned=0

    for log in "${BOOT_DEBUG_LOG}" "${BOOT_STAGES_LOG}" "${BOOT_ERROR_LOG}" "${ERRORS_LOG}"; do
        if mpremote fs rm ":${log}" 2>/dev/null; then
            print_success "Removed ${log}"
            cleaned=$((cleaned + 1))
        fi
    done

    if [ ${cleaned} -eq 0 ]; then
        print_info "No logs found to clean"
    else
        print_success "Cleaned ${cleaned} log file(s)"
    fi
}

list_backups() {
    print_header
    echo "Available backups in ${BACKUP_DIR}:"
    echo ""

    if [ -d "${BACKUP_DIR}" ]; then
        ls -lh "${BACKUP_DIR}"/main_*.py 2>/dev/null || print_info "No backups found"
    else
        print_info "No backup directory found"
    fi
}

show_status() {
    print_header
    echo "Debug Tool Status"
    echo ""

    check_mpremote

    # Check Pico connection
    if mpremote version &> /dev/null; then
        print_success "Pico connected"

        # Check which main.py is installed
        if mpremote fs cat ":${MAIN_PY}" | head -n 5 | grep -q "debug_boot.py"; then
            print_info "Mode: Minimal Debug (debug_boot.py)"
        elif mpremote fs cat ":${MAIN_PY}" | head -n 5 | grep -q "main_safe.py"; then
            print_info "Mode: Safe Boot (main_safe.py)"
        else
            print_info "Mode: Original/Unknown"
        fi
    else
        print_warning "Pico not connected"
    fi

    echo ""

    # List available tools
    echo "Available debug tools:"
    [ -f "${DEBUG_BOOT}" ] && print_success "Minimal debug boot" || print_error "Minimal debug boot (missing)"
    [ -f "${MAIN_SAFE}" ] && print_success "Safe boot mode" || print_error "Safe boot mode (missing)"
    [ -f "${READ_LOGS}" ] && print_success "Log reader" || print_error "Log reader (missing)"

    echo ""

    # Show backups
    if [ -d "${BACKUP_DIR}" ]; then
        local backup_count=$(ls -1 "${BACKUP_DIR}"/main_*.py 2>/dev/null | wc -l)
        print_info "Backups available: ${backup_count}"
    else
        print_info "Backups available: 0"
    fi
}

show_help() {
    print_header
    echo ""
    echo "Usage: ./debug.sh [OPTION]"
    echo ""
    echo "Boot Debug Modes:"
    echo "  --install-minimal    Install minimal debug boot (LED patterns + basic logging)"
    echo "  --install-safe       Install safe boot mode (full protection + detailed logging)"
    echo "  --restore            Restore original main.py from backup"
    echo ""
    echo "Log Management:"
    echo "  --logs               Read all debug logs from Pico (one-time)"
    echo "  --watch              Watch logs in real-time (updates every 2s)"
    echo "  --clean              Remove all debug logs from Pico"
    echo ""
    echo "Backup Management:"
    echo "  --list-backups       List all available backups"
    echo "  --status             Show current debug tool status"
    echo ""
    echo "Help:"
    echo "  --help, -h           Show this help message"
    echo ""
    echo "Examples:"
    echo "  ./debug.sh --install-minimal     # Install minimal debug mode"
    echo "  ./debug.sh --logs                # Read boot logs"
    echo "  ./debug.sh --watch               # Monitor logs in real-time"
    echo "  ./debug.sh --restore             # Go back to original"
    echo ""
    echo "For more information, see debug/BOOT_DEBUG_GUIDE.md"
}

# Main logic

case "${1}" in
    --install-minimal)
        install_minimal
        ;;
    --install-safe)
        install_safe
        ;;
    --restore)
        restore_original
        ;;
    --logs)
        read_logs
        ;;
    --watch)
        watch_logs
        ;;
    --clean)
        clean_logs
        ;;
    --list-backups)
        list_backups
        ;;
    --status)
        show_status
        ;;
    --help|-h|"")
        show_help
        ;;
    *)
        print_error "Unknown option: ${1}"
        echo ""
        show_help
        exit 1
        ;;
esac
