//! Chart snapshots, from the click to the effect.
//!
//! The chart is frozen where the click is handled: the card paints itself onto a scratch context
//! and the result is tessellated, all on the UI thread within one frame, from the very data the
//! chart on screen is drawn from. Everything slow (the save panel, rasterizing, the PNG, the
//! upload) then runs on a worker (`service::snapshot_job`) holding no workspace lock, and the
//! outcome comes back through a channel polled each frame. The clipboard and the browser are the
//! window's to drive, so they run here, when the outcome arrives, and a message only says what
//! has happened.

use super::*;
use crate::model::snapshot::{
    self, SnapshotAction, SnapshotDimensions, SnapshotHost as HostAddress, SnapshotRequest,
};
use crate::service::snapshot_job::{self, Outcome, Retry, SystemBackend};
use crate::service::{keychain_snapshot_token, timezone, work_queue};
use crate::snapshot_host::{Hosting, SnapshotHost};
use crate::ui::SnapshotInput;
use crate::ui::icons::Icon;
use crate::ui::kit;
use eframe::egui::{Context, Id};

/// A snapshot being made on a worker.
struct Running {
    action: SnapshotAction,
    receiver: Receiver<Outcome>,
    /// What the post says when the snapshot is shared on X.
    title: String,
    timeframe: String,
    host: Option<HostAddress>,
}

/// A hosted action that waits for the user to agree to the upload.
struct AwaitingConsent {
    request: SnapshotRequest,
    host: HostAddress,
}

/// An upload that failed in a way that may pass, waiting for the user's answer.
struct AwaitingRetry {
    retry: Retry,
    message: String,
    title: String,
    timeframe: String,
    host: HostAddress,
}

/// What a workspace keeps of its snapshots between frames.
#[derive(Default)]
pub(super) struct SnapshotState {
    running: Option<Running>,
    consent: Option<AwaitingConsent>,
    retry: Option<AwaitingRetry>,
}

impl ChartWorkspace {
    /// Gives the workspace the snapshot settings every window shares. Cheap to call every frame.
    pub fn set_snapshots(&mut self, host: &SnapshotHost) {
        if self.snapshots.is_none() {
            self.snapshots = Some(host.clone());
        }
    }

    fn snapshot_prefs(&self) -> crate::model::snapshot::SnapshotPrefs {
        self.snapshots
            .as_ref()
            .map(SnapshotHost::prefs)
            .unwrap_or_default()
    }

    /// Whether a snapshot is being made: the menu offers nothing meanwhile.
    pub(super) fn snapshot_busy(&self) -> bool {
        self.snapshot.running.is_some()
            || self.snapshot.consent.is_some()
            || self.snapshot.retry.is_some()
    }

    /// What the camera menu needs to know about hosting.
    pub(super) fn snapshot_hosting(&self) -> Hosting {
        self.snapshots
            .as_ref()
            .map_or(Hosting::NotConfigured, SnapshotHost::hosting)
    }

    /// Takes the snapshot `action` asks for of chart `chart_id`.
    pub(super) fn begin_snapshot(&mut self, ctx: &Context, chart_id: u64, action: SnapshotAction) {
        if self.snapshot_busy() {
            self.say("A snapshot is already being made.", true);
            return;
        }
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        if !self.charts[index].is_market() {
            self.say("This chart cannot be saved as an image.", true);
            return;
        }
        // What the user asked for is fixed now: a setting changed while the picture is made
        // (or while a question waits) does not change it.
        let prefs = self.snapshot_prefs();
        let request = SnapshotRequest {
            chart_id,
            action,
            options: prefs.options(),
        };
        if !action.is_hosted() {
            self.start_snapshot(ctx, request, None);
            return;
        }
        let Some(host) = prefs.host() else {
            self.say(
                "Set a snapshot host in Settings ▸ Snapshots to share.",
                true,
            );
            return;
        };
        if prefs.has_consented(&host) {
            self.start_snapshot(ctx, request, Some(host));
        } else {
            self.snapshot.consent = Some(AwaitingConsent { request, host });
        }
    }

    fn start_snapshot(
        &mut self,
        ctx: &Context,
        request: SnapshotRequest,
        host: Option<HostAddress>,
    ) {
        let SnapshotRequest {
            chart_id,
            action,
            options,
        } = request;
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        let chart = &self.charts[index];
        let size = chart.chart.snapshot_size();
        let Some(dimensions) = SnapshotDimensions::fit(size.x, size.y, options.scale) else {
            self.say("There is no chart to take a picture of yet.", true);
            return;
        };
        let input = SnapshotInput {
            symbol: &chart.symbol,
            source: chart.source,
            candles: displayed_candles(&chart.candles, replay_ms),
            log_scale: chart.log_scale,
            indicators: &chart.indicators,
            drawings: &chart.drawings,
            hide_all_drawings: self.hide_all_drawings,
            replay_active: replay_ms.is_some(),
            include_trading: options.include_trading,
        };
        let scene = match chart
            .chart
            .snapshot_scene(&input, dimensions, ui::theme::surface())
        {
            Ok(scene) => scene,
            Err(error) => {
                self.say(error.message(), true);
                return;
            }
        };
        let timeframe = self.timeframe.label().to_owned();
        let title = chart.display_title();
        let now = now_ms();
        let name = snapshot::file_name(
            &chart.symbol,
            &timeframe,
            now,
            timezone::local_utc_offset_secs(now),
        );
        let (sender, receiver) = mpsc::channel();
        let (worker_ctx, worker_host) = (ctx.clone(), host.clone());
        work_queue::SNAPSHOTS.submit(move || {
            let backend = system_backend(worker_host);
            let outcome = snapshot_job::run(action, &scene, &name, &backend);
            let _ = sender.send(outcome);
            worker_ctx.request_repaint();
        });
        self.snapshot.running = Some(Running {
            action,
            receiver,
            title,
            timeframe,
            host,
        });
    }

    /// Uploads a kept image again.
    fn retry_snapshot(&mut self, ctx: &Context, waiting: AwaitingRetry) {
        let (sender, receiver) = mpsc::channel();
        let (worker_ctx, host, retry) = (ctx.clone(), waiting.host.clone(), waiting.retry);
        let action = retry.action;
        work_queue::SNAPSHOTS.submit(move || {
            let backend = system_backend(Some(host));
            let _ = sender.send(snapshot_job::retry(retry, &backend));
            worker_ctx.request_repaint();
        });
        self.snapshot.running = Some(Running {
            action,
            receiver,
            title: waiting.title,
            timeframe: waiting.timeframe,
            host: Some(waiting.host),
        });
    }

    /// Carries out what a finished snapshot asks of the window.
    pub(super) fn poll_snapshot(&mut self, ctx: &Context) {
        let Some(running) = &self.snapshot.running else {
            return;
        };
        let outcome = match running.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.snapshot.running = None;
                self.say("The snapshot stopped before it finished.", true);
                return;
            }
        };
        let Some(running) = self.snapshot.running.take() else {
            return;
        };
        match outcome {
            Outcome::Cancelled => {}
            Outcome::Saved(path) => {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                self.say(format!("Saved {name}"), false);
                // Without a save panel the file went to the downloads folder: show where.
                if !cfg!(target_os = "macos") {
                    crate::platform::reveal_in_file_manager(&path);
                }
            }
            Outcome::CopyImage(image) => {
                ctx.copy_image(image);
                self.say("Image copied", false);
            }
            Outcome::Hosted { action, shared } => match action {
                SnapshotAction::CopyLink => {
                    ctx.copy_text(shared.url);
                    self.say("Link copied", false);
                }
                SnapshotAction::OpenInTab => {
                    ctx.open_url(egui::OpenUrl::new_tab(shared.url));
                    self.say("Snapshot opened in your browser", false);
                }
                SnapshotAction::ShareOnX => {
                    let text = snapshot::share_text(&running.title, &running.timeframe);
                    ctx.open_url(egui::OpenUrl::new_tab(snapshot::x_compose_url(
                        &text,
                        &shared.url,
                    )));
                    self.say("Opened X with a link to the snapshot", false);
                }
                SnapshotAction::Download | SnapshotAction::CopyImage => {}
            },
            Outcome::Failed {
                message,
                retry: Some(retry),
            } if running.host.is_some() => {
                self.snapshot.retry = running.host.map(|host| AwaitingRetry {
                    retry,
                    message,
                    title: running.title,
                    timeframe: running.timeframe,
                    host,
                });
            }
            Outcome::Failed { message, .. } => self.say(message, true),
        }
    }

    /// The upload's progress, and the questions it may stop to ask.
    pub(super) fn show_snapshot_sheets(&mut self, ctx: &Context) {
        if let Some(running) = &self.snapshot.running
            && running.action.is_hosted()
        {
            ui::chart_notice::show(
                ctx,
                Id::new(("snapshot-progress", self.id)),
                "Uploading snapshot…",
                false,
            );
        }
        if let Some(waiting) = &self.snapshot.consent {
            let message = format!(
                "To share this chart, DegenView uploads its image to {}. Anyone who has the link \
                 can open it. Saving or copying an image never uploads anything.",
                waiting.host.authority()
            );
            let outcome = kit::alert(
                ctx,
                Id::new(("snapshot-consent", self.id)),
                &kit::Alert {
                    icon: Icon::ShareUp,
                    tint: kit::ACCENT,
                    title: "Upload this chart image?",
                    message: &message,
                    accept: "Upload",
                    dismiss: "Cancel",
                },
            );
            match outcome {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    if let Some(waiting) = self.snapshot.consent.take() {
                        if let Some(host) = &self.snapshots
                            && let Err(error) = host.consent(&waiting.host)
                        {
                            self.say(error, true);
                        }
                        self.start_snapshot(ctx, waiting.request, Some(waiting.host));
                    }
                }
                Some(kit::ConfirmOutcome::Cancelled) => self.snapshot.consent = None,
                None => {}
            }
        }
        if let Some(waiting) = &self.snapshot.retry {
            let outcome = kit::alert(
                ctx,
                Id::new(("snapshot-retry", self.id)),
                &kit::Alert {
                    icon: Icon::TriangleFill,
                    tint: kit::ORANGE,
                    title: "Could not upload the snapshot",
                    message: &waiting.message,
                    accept: "Try again",
                    dismiss: "Cancel",
                },
            );
            match outcome {
                Some(kit::ConfirmOutcome::Confirmed) => {
                    if let Some(waiting) = self.snapshot.retry.take() {
                        self.retry_snapshot(ctx, waiting);
                    }
                }
                Some(kit::ConfirmOutcome::Cancelled) => self.snapshot.retry = None,
                None => {}
            }
        }
    }
}

/// The real backend for a job, with the token read here, on the worker: a Keychain prompt must
/// never stop the window.
fn system_backend(host: Option<HostAddress>) -> SystemBackend {
    SystemBackend::new(host, keychain_snapshot_token::token())
}
