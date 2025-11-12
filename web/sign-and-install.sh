#!/bin/bash
# Script to sign and install the Kiln Android APK

set -e

# Configuration
KEYSTORE="$HOME/kiln-release.keystore"
ALIAS="kiln"
UNSIGNED_APK="src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk"
SIGNED_APK="app-universal-release-signed.apk"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Kiln Android APK Signing and Installation${NC}"
echo "================================================"

# Check if unsigned APK exists
if [ ! -f "$UNSIGNED_APK" ]; then
	echo -e "${RED}Error: Unsigned APK not found at $UNSIGNED_APK${NC}"
	echo -e "${YELLOW}Run 'bun run tauri:android:build' first${NC}"
	exit 1
fi

# Check if keystore exists
if [ ! -f "$KEYSTORE" ]; then
	echo -e "${YELLOW}Keystore not found. Creating new keystore...${NC}"
	keytool -genkey -v -keystore "$KEYSTORE" \
		-alias "$ALIAS" \
		-keyalg RSA \
		-keysize 2048 \
		-validity 10000
	echo -e "${GREEN}Keystore created successfully${NC}"
fi

# Sign the APK
echo -e "${YELLOW}Signing APK...${NC}"
if command -v apksigner &> /dev/null; then
	# Use apksigner if available (recommended)
	apksigner sign --ks "$KEYSTORE" \
		--out "$SIGNED_APK" \
		"$UNSIGNED_APK"
else
	# Fall back to jarsigner
	echo -e "${YELLOW}apksigner not found, using jarsigner${NC}"
	cp "$UNSIGNED_APK" "$SIGNED_APK"
	jarsigner -verbose -sigalg SHA256withRSA -digestalg SHA-256 \
		-keystore "$KEYSTORE" \
		"$SIGNED_APK" \
		"$ALIAS"
fi

echo -e "${GREEN}APK signed successfully: $SIGNED_APK${NC}"

# Verify the signature
echo -e "${YELLOW}Verifying signature...${NC}"
if command -v apksigner &> /dev/null; then
	apksigner verify "$SIGNED_APK"
	echo -e "${GREEN}Signature verified${NC}"
else
	jarsigner -verify "$SIGNED_APK"
fi

# Ask if user wants to install
echo ""
read -p "Do you want to install the APK on a connected device? (y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
	if command -v adb &> /dev/null; then
		echo -e "${YELLOW}Installing on device...${NC}"
		adb install -r "$SIGNED_APK"
		echo -e "${GREEN}Installation complete!${NC}"
	else
		echo -e "${RED}adb not found. Please install Android SDK platform-tools${NC}"
		exit 1
	fi
fi

echo -e "${GREEN}Done! Signed APK available at: $SIGNED_APK${NC}"
