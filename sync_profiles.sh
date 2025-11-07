#!/bin/bash

# Sync Profiles Script
# Cleans and uploads all firing profiles to the Pico

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo ""
echo "=========================================="
echo "  Profile Sync for Pico Kiln Controller"
echo "=========================================="
echo ""

# Check if mpremote is available
if ! command -v mpremote &> /dev/null; then
    echo -e "${RED}Error: mpremote not found${NC}"
    echo "Please install it with: pip install mpremote"
    exit 1
fi

# Check if profiles directory exists locally
if [ ! -d "profiles" ]; then
    echo -e "${RED}Error: profiles/ directory not found${NC}"
    echo "Please create a profiles/ directory with .json profile files"
    exit 1
fi

# Check if there are any profile files
if [ -z "$(ls -A profiles/*.json 2>/dev/null)" ]; then
    echo -e "${YELLOW}Warning: No .json files found in profiles/ directory${NC}"
    echo "Nothing to upload"
    exit 0
fi

echo -e "${YELLOW}Step 1: Cleaning profiles directory on Pico...${NC}"

# Try to remove all .json files from the profiles directory
# We use || true to continue even if the directory doesn't exist yet
mpremote fs rm :profiles/*.json 2>/dev/null || true

echo -e "${GREEN}✓ Profiles directory cleaned${NC}"
echo ""

echo -e "${YELLOW}Step 2: Creating profiles directory...${NC}"

# Ensure the profiles directory exists
mpremote mkdir :profiles 2>/dev/null || true

echo -e "${GREEN}✓ Profiles directory ready${NC}"
echo ""

echo -e "${YELLOW}Step 3: Uploading profiles...${NC}"
echo ""

# Counter for uploaded files
count=0

# Copy all profile JSON files
for file in profiles/*.json; do
    if [ -f "$file" ]; then
        filename=$(basename "$file")
        echo "  → Uploading $filename"
        mpremote cp "$file" :profiles/
        count=$((count + 1))
    fi
done

echo ""
echo -e "${GREEN}✓ Successfully uploaded $count profile(s)${NC}"
echo ""

echo "=========================================="
echo "  Sync Complete!"
echo "=========================================="
echo ""
