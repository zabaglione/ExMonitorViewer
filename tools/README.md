# Diagnostic helpers

These standalone Windows tools support development and validation. They are not included in the user release ZIP.

## Capture metadata

`capture-diagnostics` lists limited metadata for visible (or optionally hidden) top-level windows from selected executable basenames. It reports window handles, owner handles, z-order, PID, process basename, class name, screen rectangle, monitor overlap, topmost state, and display-affinity status. It does not read window titles or pixels, change window settings, or send input.

Run from the helper directory:

    cargo run --release -- --process WebexHost.exe --process CiscoCollabHost.exe

Add `--include-hidden` to include hidden top-level windows. Keep generated reports local and remove any sensitive metadata before sharing them.

## DWM thumbnail probe

`dwm-thumbnail-probe` is a short-lived API experiment. Its self-test draws its own synthetic colored windows and samples only a few points inside its own destination rectangle. `GetPixel` results can be inconclusive under desktop composition and do not prove that a thumbnail is physically visible to a person. A successful DWM registration/update likewise proves API acceptance, not visible output. Physical visibility must be checked separately; the probe does not capture or save a screenshot.

The `webex` mode can temporarily show a selected Webex window through DWM on another monitor. It does not change the source window's settings or position, and the destination window is marked capture-excluded before the foreign thumbnail is attached. Use it only during a controlled local validation, then allow its 15-second lifetime to close it.

    cargo run --release -- --mode self-test --display \\.\DISPLAY2

Diagnostic output is local metadata. Do not publish it with release assets.
