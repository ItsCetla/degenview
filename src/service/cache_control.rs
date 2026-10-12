//! The cache's controls: what launch sets up, and what Settings ▸ Storage reads and does.

use super::disk_cache::{self, CacheStats, ClearOutcome, Limits};
use super::{alpaca, binance, coinbase, icon, image_loader, lookup_cache, work_queue};
use eframe::egui;
use std::time::{Duration, SystemTime};

/// How long after launch the folder is pruned, so the sweep never competes with first paint.
const PRUNE_DELAY: Duration = Duration::from_secs(10);

/// Sets the cache up for a launch: images served through it, lookups the first frame needs read
/// from disk, and the folder kept within its age and size limits. Call after
/// `install_image_loaders`.
pub fn launch(ctx: &egui::Context) {
    image_loader::install(ctx);
    lookup_cache::shared().warm(&icon::NAMESPACES);
    if disk_cache::global().is_enabled() {
        std::thread::spawn(|| {
            std::thread::sleep(PRUNE_DELAY);
            disk_cache::global().prune(SystemTime::now(), Limits::default());
        });
    }
}

/// What is cached and where. Walks the folder: call it off the UI thread.
pub fn stats() -> CacheStats {
    disk_cache::global().stats()
}

/// Deletes everything cached and forgets what is held in memory, so the app starts over from the
/// network. Blocking; user data is not touched. Logos on screen show monograms until they load
/// again.
pub fn clear(ctx: &egui::Context) -> ClearOutcome {
    let outcome = disk_cache::global().clear();
    clear_memory();
    // egui's own copies: decoded textures and the bytes behind them.
    ctx.forget_all_images();
    ctx.request_repaint();
    outcome
}

/// Empties every in-memory layer in front of the disk cache.
fn clear_memory() {
    icon::clear_memory();
    lookup_cache::shared().clear_memory();
    binance::clear_catalog();
    coinbase::clear_catalog();
    alpaca::clear_catalog();
    image_loader::clear_memory();
}

/// Starts the work on the shared queue and hands back the answer, for callers that must not
/// block the UI.
pub fn clear_in_background(ctx: &egui::Context, done: impl FnOnce(ClearOutcome) + Send + 'static) {
    let ctx = ctx.clone();
    work_queue::submit(move || done(clear(&ctx)));
}

/// `stats`, off the UI thread.
pub fn stats_in_background(done: impl FnOnce(CacheStats) + Send + 'static) {
    work_queue::submit(move || done(stats()));
}
