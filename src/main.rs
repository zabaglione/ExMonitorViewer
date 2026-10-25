#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cursor;
mod math;
mod settings;
mod winapp;

fn main() {
    if let Err(error) = winapp::run() {
        let message = format!(
            "Application failed with code 0x{:08X}.",
            error.code().0 as u32
        );
        winapp::show_error(&message);
        std::process::exit(1);
    }
}
