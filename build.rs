use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=assets/app-icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let resource_dir = manifest_dir.join("resources");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("exmonitor-viewer.res");
    let resource_compiler = find_resource_compiler();
    let status = Command::new(resource_compiler)
        .current_dir(&resource_dir)
        .arg("/nologo")
        .arg(format!("/fo{}", output.display()))
        .arg("app.rc")
        .status()
        .expect("rc.exe was not found; run build.cmd from a Visual Studio developer environment");
    assert!(status.success(), "Windows resource compilation failed");

    println!("cargo:rustc-link-arg={}", output.display());
}

fn find_resource_compiler() -> PathBuf {
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            let candidate = directory.join("rc.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    if let Some(sdk_root) = env::var_os("WindowsSdkDir") {
        let bin = PathBuf::from(sdk_root).join("bin");
        if let Ok(versions) = std::fs::read_dir(bin) {
            for version in versions.flatten() {
                let candidate = version.path().join("x64").join("rc.exe");
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }

    PathBuf::from("rc.exe")
}
