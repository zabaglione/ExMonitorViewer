# Motion Source

`motion-source.exe --display "\\.\DISPLAY2" --seconds 30 --report motion-report.txt` opens a GPU-rendered motion pattern across the named monitor. Press Esc to close it early. The optional `--report` file records elapsed time, the D3D adapter associated with that display, successful synchronized `Present(1, 0)` calls, and their average rate in ASCII. Run `motion-source.exe --list-monitors` to show display names and their adapters.

Build from this folder with a Rust MSVC toolchain, Visual C++ Build Tools, and the Windows SDK:

```powershell
cargo build --release
```

The report measures successful synchronized presentation calls. It is not an optical measurement of physical panel scanout. The source animation changes every frame, so the monitor is not kept on a static image.
