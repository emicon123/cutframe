//! Application state shared by commands, the media protocol and the preview.

use std::sync::{Arc, Mutex, MutexGuard};

use app_core::command::CommandError;
use app_core::{Document, Project};
use arc_swap::ArcSwap;
use tauri::AppHandle;
use tauri_specta::Event;

use crate::ipc::IpcError;
use crate::ipc::events::ProjectChanged;
use crate::ipc::types::ProjectState;
use crate::preview::Preview;

#[derive(Debug)]
pub struct AppState {
    document: Mutex<Document>,
    /// Latest project snapshot for engine threads. Lock-free reads.
    snapshot: ArcSwap<Project>,
    pub preview: Option<Preview>,
}

impl AppState {
    pub fn new(project: Project, preview: Option<Preview>) -> Self {
        let document = Document::new(project);
        let snapshot = ArcSwap::new(document.snapshot());
        if let Some(preview) = &preview {
            preview.set_canvas_aspect(&document.project().settings);
        }
        Self {
            document: Mutex::new(document),
            snapshot,
            preview,
        }
    }

    pub fn project_state(&self) -> ProjectState {
        ProjectState::from(&*self.lock())
    }

    #[allow(dead_code)] // Read by the render graph from stage 1 on.
    pub fn snapshot(&self) -> Arc<Project> {
        self.snapshot.load_full()
    }

    /// Runs `edit` on the document, publishes the new snapshot and notifies the UI.
    ///
    /// `edit` returns `Ok(None)` when nothing changed (e.g. undo with an empty history); then no
    /// event is emitted.
    pub fn edit<T>(
        &self,
        app: &AppHandle,
        edit: impl FnOnce(&mut Document) -> Result<Option<T>, CommandError>,
    ) -> Result<Option<T>, IpcError> {
        let (result, state) = {
            let mut doc = self.lock();
            let Some(result) = edit(&mut doc)? else {
                return Ok(None);
            };
            self.snapshot.store(doc.snapshot());
            if let Some(preview) = &self.preview {
                preview.set_canvas_aspect(&doc.project().settings);
            }
            (result, ProjectState::from(&*doc))
        };
        // Emit outside the lock. Events may arrive out of order; the UI orders them by revision.
        ProjectChanged(state).emit(app)?;
        Ok(Some(result))
    }

    fn lock(&self) -> MutexGuard<'_, Document> {
        // A panic while holding the lock leaves a consistent document: commands mutate a draft
        // and swap it in only on success.
        self.document
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
