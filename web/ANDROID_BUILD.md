# Android APK Build Guide

This document explains how to build and deploy the Kiln web app as an Android APK using Tauri.

## Prerequisites

- Java JDK 17+ (configured with jenv)
- Android SDK with NDK installed
- Rust toolchain with Android targets
- Bun package manager

## Environment Setup

Ensure the following environment variables are set:

```bash
export ANDROID_HOME="$HOME/Library/Android/sdk"
export NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
```

You can add these to your `~/.zshrc` or `~/.bashrc` file.

## Build Commands

### Debug Build
```bash
cd web
bun run tauri:android:build:debug
```

**Output:**
- APK: `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`
- Size: ~389 MB (includes debug symbols)

### Release Build
```bash
cd web
bun run tauri:android:build
```

**Output:**
- APK (unsigned): `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
- AAB bundle: `src-tauri/gen/android/app/build/outputs/bundle/universalRelease/app-universal-release.aab`
- Size: ~29 MB (optimized for release)

## Signing the Release APK

The release APK is unsigned and needs to be signed before distribution.

### 1. Generate a Keystore (first time only)

```bash
keytool -genkey -v -keystore ~/kiln-release.keystore \
  -alias kiln \
  -keyalg RSA \
  -keysize 2048 \
  -validity 10000
```

### 2. Sign the APK

```bash
# Using jarsigner
jarsigner -verbose -sigalg SHA256withRSA -digestalg SHA-256 \
  -keystore ~/kiln-release.keystore \
  src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk \
  kiln

# Or using apksigner (recommended)
apksigner sign --ks ~/kiln-release.keystore \
  --out app-universal-release-signed.apk \
  src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
```

### 3. Verify the Signature

```bash
apksigner verify app-universal-release-signed.apk
```

## Installing on Android Device

### Via ADB (Android Debug Bridge)

```bash
# Install debug version
adb install src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk

# Install signed release version
adb install app-universal-release-signed.apk
```

### Via File Transfer

1. Copy the APK to your Android device
2. Enable "Install from Unknown Sources" in device settings
3. Open the APK file to install

## Google Play Store Distribution

To publish on Google Play Store:

1. Sign the AAB bundle instead of the APK
2. Upload the signed AAB to Google Play Console
3. Google Play will optimize and sign APKs for different device configurations

```bash
# Sign the AAB
jarsigner -verbose -sigalg SHA256withRSA -digestalg SHA-256 \
  -keystore ~/kiln-release.keystore \
  src-tauri/gen/android/app/build/outputs/bundle/universalRelease/app-universal-release.aab \
  kiln
```

## Development Workflow

### Running in Dev Mode

```bash
cd web
bun run tauri:android
```

This starts the app in development mode on a connected Android device or emulator.

### Open in Android Studio

```bash
cd web
bun run tauri android build --open
```

This opens the project in Android Studio for advanced debugging and configuration.

## Configuration

The Android app configuration is managed in:

- `src-tauri/tauri.conf.json` - Main Tauri configuration
- `src-tauri/gen/android/app/build.gradle.kts` - Android-specific build settings

### Key Settings

- **App ID:** `com.nivelais.kiln`
- **Min SDK:** 24 (Android 7.0)
- **Target SDK:** 36
- **Supported ABIs:** arm64-v8a, armeabi-v7a, x86, x86_64

## Troubleshooting

### NDK Not Found

If you see "NDK not found" errors, ensure `NDK_HOME` is set correctly:

```bash
export NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"
```

### Rust Targets Missing

Install required Android targets:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
```

### Build Fails with Gradle Errors

Clean the build cache:

```bash
cd src-tauri/gen/android
./gradlew clean
cd ../../..
```

## File Locations

- **Debug APK:** `389 MB` at `src-tauri/gen/android/app/build/outputs/apk/universal/debug/`
- **Release APK:** `29 MB` at `src-tauri/gen/android/app/build/outputs/apk/universal/release/`
- **AAB Bundle:** `14 MB` at `src-tauri/gen/android/app/build/outputs/bundle/universalRelease/`

## Additional Resources

- [Tauri Android Documentation](https://tauri.app/v1/guides/building/android)
- [Android App Signing](https://developer.android.com/studio/publish/app-signing)
- [Google Play Console](https://play.google.com/console)
