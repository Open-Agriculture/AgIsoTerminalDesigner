use super::{comparison::Comparison, validate, CliProjectArchive, GenerationState, ReplayState};
use crate::EditorProject;
use std::io::{Cursor, Write};

/// Stable, coarse progress for an incremental CLI ZIP export.
#[derive(Clone, Copy, Debug)]
pub struct ExportProgress {
    /// Approximate fraction complete, from 0.0 through 1.0. Only successful
    /// completion reports 1.0; this is work completed, not elapsed time.
    pub fraction: f32,
    /// Broad export stage. Text changes only when the stage changes.
    pub status: &'static str,
}

/// Incremental export driver owning a stable project snapshot.
///
/// Dropping this job between calls cancels without changing source project
/// content or writing any files. Individual validation, image conversion,
/// serialization and replay operations cannot be interrupted mid-call.
pub struct ZipExportJob {
    project: EditorProject,
    phase: Phase,
    progress: ExportProgress,
}

enum Phase {
    Prepare,
    Generate(GenerationState),
    Replay(ReplayState),
    Verify {
        replay: ReplayState,
        comparison: Comparison,
    },
    Package {
        archive: CliProjectArchive,
        writer: ArchiveWriter,
    },
    Done,
}

impl ZipExportJob {
    /// Take ownership of a project snapshot. Work starts on the first advance.
    pub fn new(project: EditorProject) -> Self {
        Self {
            project,
            phase: Phase::Prepare,
            progress: ExportProgress {
                fraction: 0.0,
                status: "Preparing project…",
            },
        }
    }

    /// Return the latest progress, without performing export work.
    pub fn progress(&self) -> ExportProgress {
        self.progress
    }

    /// Perform one cooperative work unit, returning ZIP bytes on completion.
    ///
    /// `Ok(None)` means more work remains. Stop calling after `Ok(Some(_))` or
    /// `Err(_)`; a completed or failed job cannot be resumed.
    pub fn advance(&mut self) -> Result<Option<Vec<u8>>, String> {
        // Move owned state between stages; errors leave the job terminal.
        match std::mem::replace(&mut self.phase, Phase::Done) {
            Phase::Prepare => {
                validate(&self.project)?;
                self.phase = Phase::Generate(GenerationState::new(&self.project)?);
                self.progress = ExportProgress {
                    fraction: 0.05,
                    status: "Generating commands and assets…",
                };
            }
            Phase::Generate(mut state) => {
                if state.step(&self.project)? {
                    self.phase = Phase::Replay(ReplayState::new(state.workflow));
                    self.progress = ExportProgress {
                        fraction: 0.30,
                        status: "Checking generated commands…",
                    };
                } else {
                    self.progress.fraction = 0.05 + 0.25 * state.fraction();
                    self.phase = Phase::Generate(state);
                }
            }
            Phase::Replay(mut replay) => {
                if replay.step()? {
                    self.phase = Phase::Verify {
                        replay,
                        comparison: Comparison::default(),
                    };
                    self.progress = ExportProgress {
                        fraction: 0.85,
                        status: "Verifying rebuilt project…",
                    };
                } else {
                    self.progress.fraction = 0.30 + 0.55 * replay.fraction();
                    self.phase = Phase::Replay(replay);
                }
            }
            Phase::Verify {
                replay,
                mut comparison,
            } => {
                if comparison.advance(&self.project, &replay.candidate)? {
                    let archive = replay.finish_archive();
                    let writer = ArchiveWriter::new(&archive);
                    self.phase = Phase::Package { archive, writer };
                    self.progress = ExportProgress {
                        fraction: 0.95,
                        status: "Creating ZIP archive…",
                    };
                } else {
                    self.progress.fraction = 0.85 + 0.10 * comparison.fraction();
                    self.phase = Phase::Verify { replay, comparison };
                }
            }
            Phase::Package {
                archive,
                mut writer,
            } => {
                if let Some(bytes) = writer.advance(&archive)? {
                    self.progress.fraction = 1.0;
                    return Ok(Some(bytes));
                }
                self.progress.fraction = (0.95 + 0.05 * writer.fraction()).min(0.999);
                self.phase = Phase::Package { archive, writer };
            }
            Phase::Done => return Err("CLI ZIP export job is already complete or failed".into()),
        }
        Ok(None)
    }
}

/// Shared by synchronous ZIP serialization and the cooperative job.
pub(super) struct ArchiveWriter {
    writer: Option<zip::ZipWriter<Cursor<Vec<u8>>>>,
    paths: std::vec::IntoIter<String>,
    current: Option<String>,
    offset: usize,
    completed: u64,
    total: u64,
}

impl ArchiveWriter {
    pub(super) fn new(archive: &CliProjectArchive) -> Self {
        Self {
            writer: Some(zip::ZipWriter::new(Cursor::new(Vec::new()))),
            paths: archive
                .files
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .into_iter(),
            current: None,
            offset: 0,
            completed: 0,
            total: archive
                .files
                .values()
                .map(|bytes| bytes.len() as u64 + 1)
                .sum(),
        }
    }

    fn fraction(&self) -> f32 {
        self.completed as f32 / self.total.max(1) as f32
    }

    pub(super) fn advance(
        &mut self,
        archive: &CliProjectArchive,
    ) -> Result<Option<Vec<u8>>, String> {
        if self.current.is_none() {
            let Some(path) = self.paths.next() else {
                return Ok(Some(
                    self.writer
                        .take()
                        .unwrap()
                        .finish()
                        .map_err(|e| e.to_string())?
                        .into_inner(),
                ));
            };
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .last_modified_time(zip::DateTime::default());
            self.writer
                .as_mut()
                .unwrap()
                .start_file(&path, options)
                .map_err(|e| e.to_string())?;
            self.current = Some(path);
        }
        let bytes = &archive.files[self.current.as_ref().unwrap()];
        let end = (self.offset + 64 * 1024).min(bytes.len());
        self.writer
            .as_mut()
            .unwrap()
            .write_all(&bytes[self.offset..end])
            .map_err(|e| e.to_string())?;
        self.completed += (end - self.offset) as u64;
        self.offset = end;
        if end == bytes.len() {
            self.completed += 1;
            self.current = None;
            self.offset = 0;
        }
        Ok(None)
    }
}
