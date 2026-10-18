mod app;
mod check;
mod cli;
mod config;
mod disks;
mod draft;
mod editor;
mod error;
mod exec;
mod hardware;
mod i18n;
mod input;
mod layout;
mod manifest;
mod network;
mod options;
mod plan;
mod repos;
mod repository;
mod resolve;
mod secrets;
mod sources;
mod storage;
#[cfg(test)]
mod testing;
mod tui;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    match cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
