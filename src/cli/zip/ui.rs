//! Platform-specific scheduling for the shared incremental ZIP exporter.
use ag_iso_terminal_designer::{
    cli::zip::{ExportProgress, ZipExportJob},
    EditorProject,
};
use eframe::egui;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
    Arc, Mutex,
};

pub enum Outcome {
    Complete(Vec<u8>),
    Cancelled,
    Failed(String),
}

pub struct Export {
    cancelling: bool,
    #[cfg(not(target_arch = "wasm32"))]
    cancel: Arc<AtomicBool>,
    #[cfg(not(target_arch = "wasm32"))]
    progress: Arc<Mutex<ExportProgress>>,
    #[cfg(not(target_arch = "wasm32"))]
    receiver: Receiver<Outcome>,
    #[cfg(target_arch = "wasm32")]
    job: ZipExportJob,
}

impl Export {
    pub fn new(project: EditorProject, ctx: &egui::Context) -> Self {
        let job = ZipExportJob::new(project);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cancel = Arc::new(AtomicBool::new(false));
            let progress = Arc::new(Mutex::new(job.progress()));
            let (sender, receiver) = mpsc::channel();
            let worker_cancel = cancel.clone();
            let worker_progress = progress.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let mut job = job;
                let outcome = loop {
                    if worker_cancel.load(Ordering::Relaxed) {
                        break Outcome::Cancelled;
                    }
                    let result = job.advance();
                    *worker_progress.lock().unwrap() = job.progress();
                    ctx.request_repaint();
                    if worker_cancel.load(Ordering::Relaxed) {
                        break Outcome::Cancelled;
                    }
                    match result {
                        Ok(Some(bytes)) => break Outcome::Complete(bytes),
                        Ok(None) => (),
                        Err(error) => break Outcome::Failed(error),
                    }
                };
                let _ = sender.send(outcome);
                ctx.request_repaint();
            });
            Self {
                cancelling: false,
                cancel,
                progress,
                receiver,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            ctx.request_repaint();
            Self {
                cancelling: false,
                job,
            }
        }
    }

    pub fn progress(&self) -> ExportProgress {
        #[cfg(not(target_arch = "wasm32"))]
        {
            *self.progress.lock().unwrap()
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.job.progress()
        }
    }

    pub fn cancel(&mut self) {
        self.cancelling = true;
        #[cfg(not(target_arch = "wasm32"))]
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Render before polling so a Cancel click wins over queued completion.
    pub fn show(&mut self, ctx: &egui::Context) {
        let progress = self.progress();
        egui::Modal::new(egui::Id::new("cli_zip_export")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("Exporting CLI ZIP");
            ui.add_space(8.0);
            ui.label(if self.cancelling {
                "Cancelling…"
            } else {
                progress.status
            });
            ui.add(egui::ProgressBar::new(progress.fraction).text(percentage(progress.fraction)));
            ui.add_space(8.0);
            if ui
                .add_enabled(!self.cancelling, egui::Button::new("Cancel"))
                .clicked()
            {
                self.cancel();
            }
        });
    }

    pub fn poll(&mut self) -> Option<Outcome> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match self.receiver.try_recv() {
                Ok(_) if self.cancelling => Some(Outcome::Cancelled),
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Outcome::Failed(
                    "The CLI ZIP export worker stopped unexpectedly".into(),
                )),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if self.cancelling {
                return Some(Outcome::Cancelled);
            }
            let clock = web_sys::window().and_then(|window| window.performance());
            let start = clock.as_ref().map(|clock| clock.now());
            loop {
                match self.job.advance() {
                    Ok(Some(bytes)) => return Some(Outcome::Complete(bytes)),
                    Err(error) => return Some(Outcome::Failed(error)),
                    Ok(None) => (),
                }
                if !clock
                    .as_ref()
                    .zip(start)
                    .is_some_and(|(clock, start)| clock.now() - start < 8.0)
                {
                    return None;
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Export {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn percentage(fraction: f32) -> String {
    format!("{}%", (fraction.clamp(0.0, 1.0) * 100.0).floor() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentage_rounds_down_and_only_reaches_100_at_completion() {
        assert_eq!(percentage(0.0), "0%");
        assert_eq!(percentage(0.429), "42%");
        assert_eq!(percentage(0.999), "99%");
        assert_eq!(percentage(1.0), "100%");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cancellation_wins_over_queued_success() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Outcome::Complete(vec![1])).unwrap();
        let mut export = Export {
            cancelling: false,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(ExportProgress {
                fraction: 1.0,
                status: "Creating ZIP archive…",
            })),
            receiver,
        };
        export.cancel();
        assert!(matches!(export.poll(), Some(Outcome::Cancelled)));
    }
}
