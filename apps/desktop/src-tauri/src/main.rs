//! Windows release builds must not open a console window; everything else lives in the
//! library so it stays testable.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    evara_desktop_lib::run();
}
