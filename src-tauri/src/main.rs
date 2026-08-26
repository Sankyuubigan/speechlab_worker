// Prevents additional console window on Windows in ALL builds (incl. dev).
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    speechlab_lib::run()
}
