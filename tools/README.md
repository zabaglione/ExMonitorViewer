# Diagnostic helpers

These standalone Windows tools support development and validation. They are not included in the user release ZIP.

## Capture metadata

`capture-diagnostics` lists limited metadata for visible (or optionally hidden) top-level windows from selected executable basenames. It reports window handles, owner handles, z-order, PID, process basename, class name, screen rectangle, monitor overlap, topmost state, and display-affinity status. It does not read window titles or pixels, change window settings, or send input.

Pass any executable basename with `--process` (the default list is retained for the original Webex investigation):

    cargo run --release -- --process example-app.exe

Add `--include-hidden` to include hidden top-level windows. Keep generated reports local and remove any sensitive metadata before sharing them.

## DWM thumbnail probe

`dwm-thumbnail-probe` is a short-lived API experiment. Its self-test draws its own synthetic colored windows and samples only a few points inside its own destination rectangle. `GetPixel` results can be inconclusive under desktop composition and do not prove that a thumbnail is physically visible to a person. A successful DWM registration/update likewise proves API acceptance, not visible output. Physical visibility must be checked separately; the probe does not capture or save a screenshot.

The `webex` mode is a historical probe that can temporarily show a selected Webex window through DWM on another monitor. The application's current window selection does not use process names. The probe does not change a source window's settings or position, and the destination is marked capture-excluded before the foreign thumbnail is attached. It closes after 15 seconds.

    cargo run --release -- --mode self-test --display \\.\DISPLAY2

Diagnostic output is local metadata. Do not publish it with release assets.

## Composition source fixture

`composition-fixture` runs as a separate executable named `composition-fixture.exe` and creates a synthetic, clearly labeled window for testing the app's generic eligibility rules. The default `protected` mode is topmost, marked capture-excluded, and placed on the requested display. It closes automatically after 30 seconds.

    cargo run --release --manifest-path tools/composition-fixture/Cargo.toml -- --mode protected --display \\.\DISPLAY2

Negative modes exercise an unprotected window, a non-topmost window, a window on another display, an explicitly marked overlay window, and the legacy v0.1.4 overlay class. No mode creates a fake Webex process, changes Webex, captures screenshots, or moves the OS pointer.
