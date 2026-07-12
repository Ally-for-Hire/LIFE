//! LIFE — native, watchable life/civ simulator with evolving neural leaders.
//!
//! Single process: the simulation and the egui renderer share memory, so the
//! view never serializes the world to draw it. Run with `cargo run --release`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod brain;
mod clan;
#[cfg(test)]
mod diag;
mod entity;
mod grid;
mod rng;
mod trainer;
mod world;

use app::LifeApp;
use eframe::egui;

const MAX_TRAINING_THREADS: usize = 18;

fn main() -> eframe::Result<()> {
    configure_rayon_threads();
    raise_process_priority();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("LIFE — native"),
        ..Default::default()
    };
    eframe::run_native(
        "LIFE",
        native_options,
        Box::new(|cc| Ok(Box::new(LifeApp::new(cc)))),
    )
}

fn configure_rayon_threads() {
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(MAX_TRAINING_THREADS);
    let threads = available.min(MAX_TRAINING_THREADS).max(1);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global();
}

#[cfg(target_os = "windows")]
fn raise_process_priority() {
    const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn SetPriorityClass(hProcess: *mut core::ffi::c_void, dwPriorityClass: u32) -> i32;
    }

    unsafe {
        let handle = GetCurrentProcess();
        let _ = SetPriorityClass(handle, HIGH_PRIORITY_CLASS);
    }
}

#[cfg(not(target_os = "windows"))]
fn raise_process_priority() {}
