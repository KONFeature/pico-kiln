# pico-kiln patches

Vendored from crates.io `tauri-plugin-background-service` **1.0.1**. Upstream has no
way to update the foreground-service notification content in place, so these
patches are re-applied on every upgrade (search for `PICO-KILN PATCH`):

1. `updateNotification` command: Kotlin `BackgroundServicePlugin.kt`
   (`UpdateNotificationArgs`, `@Command updateNotification`) and
   `LifecycleService.kt` (`ACTION_UPDATE`, `EXTRA_TITLE`, `EXTRA_BODY`, handler in
   `onStartCommand`, `buildLiveNotification`) re-post the running service's own
   notification (same id) with a title and a big-text body.
2. Rust: `MobileLifecycle::update_notification` (`src/mobile.rs`) and the public
   `update_notification(app, title, body)` (`src/lib.rs`; no-op off Android).
3. `permissions/default.toml`: desktop-service OS-service permissions removed
   (their permission files are only generated with the `desktop-service` feature).
4. `Cargo.toml`: taken from upstream `Cargo.toml.orig` minus the `headless_daemon`
   example; `tests/`, `guest-js/`, `CODEBASE_AUDIT.md` not vendored.
5. 1.0's call stack disabled (unused by the kiln monitor). The library manifest
   merges into the app, so upstream's `AndroidManifest.xml` would make the APK
   request CAMERA, RECORD_AUDIO, MANAGE_OWN_CALLS, USE_FULL_SCREEN_INTENT and the
   phoneCall/microphone FGS types. `AndroidManifest.xml` drops those permissions,
   the `phoneCall|microphone` service types, and the
   `BackgroundCallConnectionService` declaration. `BackgroundServicePlugin.load()`
   no longer registers the self-managed Telecom phone account. Upstream's
   call-related unit tests (e.g. `MergedManifestForegroundServiceTest`) are not
   expected to pass against this manifest. Re-check the merged manifest
   (`gen/android/app/build/intermediates/merged_manifests`) on every upgrade.
