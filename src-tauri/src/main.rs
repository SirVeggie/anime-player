// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|arg| arg == "--apply-update") {
        std::process::exit(anime_player_lib::apply_pending_update());
    }
    anime_player_lib::run()
}
