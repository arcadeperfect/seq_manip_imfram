//! seq_manip's `commands.rs` as plain calls: the managed state Tauri
//! held, and one method per command. Async commands block the calling
//! thread (call them off the UI thread); sync ones are cheap.

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::commands::{self as c, CommitState, ExportState, SearchState};
use crate::state::{EntityId, IntentOp, Registry};
use crate::thumbs::{InputSpace, ViewTransform};

pub use crate::commands::{CommitOutcome, DeleteOutcome, DeletePreview, DirNode, DirView, EntityView, InfoField, MediaInfo, MetaDto, PermanentDeletes, ProgressDto, SearchProgress};

/// The Tauri app's identifier: the cache folder is shared with it.
pub const IDENTIFIER: &str = "com.alex.seq-manip";

/// Colour decisions every picture call carries.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub input: InputSpace,
    pub view: ViewTransform,
}

pub struct Backend {
    pub registry: Mutex<Registry>,
    pub commit: CommitState,
    pub search: SearchState,
    pub export: ExportState,
    pub app: AppHandle,
}

impl Default for Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend {
    pub fn new() -> Self {
        Self { registry: Mutex::new(Registry::new()), commit: CommitState::default(), search: SearchState::default(), export: ExportState::default(), app: AppHandle::new(IDENTIFIER) }
    }

    fn reg(&self) -> State<'_, Mutex<Registry>> {
        State::new(&self.registry)
    }

    pub fn cache_dir(&self) -> PathBuf {
        use tauri::Manager;
        self.app.path().app_cache_dir().unwrap_or_else(|_| std::env::temp_dir().join(IDENTIFIER))
    }

    // ---- the folder and its entities ----

    pub fn open_directory(&self, path: &str, min_frames: Option<usize>) -> Result<DirView, String> {
        c::open_directory(path.into(), min_frames, self.reg(), State::new(&self.commit))
    }
    pub fn view_entities(&self, ids: Vec<EntityId>) -> Result<DirView, String> {
        c::view_entities(ids, self.reg(), State::new(&self.commit))
    }
    pub fn stage_intent(&self, id: EntityId, intent: IntentOp) -> Result<EntityView, String> {
        c::stage_intent(id, intent, self.reg(), State::new(&self.commit))
    }
    pub fn stage_copy(&self, id: EntityId, dest: Option<String>, prefix: Option<String>, name: Option<String>) -> Result<EntityView, String> {
        c::stage_copy(id, dest, prefix, name, self.reg(), State::new(&self.commit))
    }
    pub fn revert_intent(&self, id: EntityId, index: usize) -> Result<Option<EntityView>, String> {
        c::revert_intent(id, index, self.reg(), State::new(&self.commit))
    }
    pub fn clear_intents(&self, id: EntityId) -> Result<Option<EntityView>, String> {
        c::clear_intents(id, self.reg(), State::new(&self.commit))
    }
    pub fn pending_entities(&self) -> Result<Vec<EntityView>, String> {
        c::pending_entities(self.reg())
    }
    pub fn permanent_deletes(&self, ids: Option<Vec<EntityId>>) -> Result<PermanentDeletes, String> {
        c::permanent_deletes(ids, self.reg())
    }

    // ---- long jobs (block; call off the UI thread) ----

    pub fn commit_pending(&self, ids: Option<Vec<EntityId>>, force: bool, progress: impl Fn(ProgressDto) + Send + Sync + 'static) -> Result<CommitOutcome, String> {
        tauri::block_on(c::commit_pending(ids, Some(force), Channel::new(progress), self.reg(), State::new(&self.commit)))
    }
    pub fn cancel_commit(&self) {
        c::cancel_commit(State::new(&self.commit))
    }
    pub fn search_open(&self, root: &str, query: &str, progress: impl Fn(SearchProgress) + Send + Sync + 'static) -> Result<DirView, String> {
        tauri::block_on(c::search_open(root.into(), query.into(), Channel::new(progress), self.reg(), State::new(&self.commit), State::new(&self.search)))
    }
    pub fn cancel_search(&self) {
        c::cancel_search(State::new(&self.search))
    }
    pub fn export_h264(&self, id: EntityId, fps: f64, crf: u32, dest: &str, overwrite: bool, progress: impl Fn(ProgressDto) + Send + Sync + 'static) -> Result<bool, String> {
        tauri::block_on(c::export_h264(id, fps, crf, dest.into(), overwrite, Channel::new(progress), self.reg(), State::new(&self.commit), State::new(&self.export)))
    }
    pub fn cancel_export(&self) {
        c::cancel_export(State::new(&self.export))
    }
    pub fn delete_preview(&self, ids: Vec<EntityId>) -> Result<DeletePreview, String> {
        c::delete_preview(ids, self.reg())
    }
    pub fn delete_entities(&self, ids: Vec<EntityId>, to_trash: bool, progress: impl Fn(ProgressDto) + Send + Sync + 'static) -> Result<DeleteOutcome, String> {
        tauri::block_on(c::delete_entities(ids, to_trash, Channel::new(progress), self.reg(), State::new(&self.commit)))
    }

    // ---- pictures and facts (block) ----

    /// A still's preview (a sequence's middle frame) as PNG bytes.
    pub fn thumbnail(&self, id: EntityId, color: Color) -> Result<Option<Vec<u8>>, String> {
        let url = tauri::block_on(c::thumbnail(id, Some(color.input), Some(color.view), self.app.clone(), self.reg()))?;
        Ok(url.and_then(|u| decode_data_url(&u)))
    }
    /// Evenly spaced preview frames (a movie's timestamps), PNG bytes each.
    pub fn scrub_thumbnails(&self, id: EntityId, count: usize, color: Color) -> Result<Vec<Vec<u8>>, String> {
        let urls = tauri::block_on(c::scrub_thumbnails(id, Some(count), Some(color.input), Some(color.view), self.app.clone(), self.reg()))?;
        Ok(urls.iter().map(|u| decode_data_url(u).unwrap_or_default()).collect())
    }
    /// Frame `index` (into the frames on disk) as JPEG bytes, fitted in `max_dim`.
    pub fn playback_frame(&self, id: EntityId, index: usize, max_dim: u32, color: Color, exposure: f32) -> Result<Vec<u8>, String> {
        tauri::block_on(c::playback_frame(id, index, Some(max_dim), Some(color.input), Some(color.view), Some(exposure), self.app.clone(), self.reg())).map(|r| r.into_bytes())
    }
    pub fn media_info(&self, id: EntityId) -> Result<MediaInfo, String> {
        tauri::block_on(c::media_info(id, self.reg()))
    }
    pub fn entity_metadata(&self, ids: Vec<EntityId>) -> Result<Vec<MetaDto>, String> {
        tauri::block_on(c::entity_metadata(ids, self.reg()))
    }

    // ---- the filesystem ----

    pub fn list_dirs(&self, path: &str) -> Vec<DirNode> {
        c::list_dirs(path.into())
    }
    pub fn filesystem_roots(&self) -> Vec<DirNode> {
        c::filesystem_roots()
    }
    pub fn complete_path(&self, partial: &str) -> Vec<String> {
        c::complete_path(partial.into())
    }
    pub fn home_directory(&self) -> Result<String, String> {
        c::home_directory()
    }
    pub fn send_to_wobble_edit(&self, samples: Vec<String>) -> Result<usize, String> {
        c::send_to_wobble_edit(samples)
    }

    /// The bundled ffmpeg (seq_manip's own; `binaries` links to it), else PATH's.
    pub fn ffmpeg(&self) -> Option<PathBuf> {
        crate::thumbs::ffmpeg_path()
    }

    /// Frame paths on disk for an entity (a loose file: itself).
    pub fn frame_paths(&self, id: EntityId) -> Vec<PathBuf> {
        use crate::state::Canonical;
        let r = self.registry.lock().unwrap();
        match r.get(id).map(|e| &e.canonical) {
            Some(Canonical::Sequence(s)) => s.items().iter().map(|i| i.path()).collect(),
            Some(Canonical::Loose { path, .. }) => vec![path.clone()],
            None => vec![],
        }
    }
}

fn decode_data_url(u: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let b64 = u.split_once(',')?.1;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_opens_stages_and_commits_through_the_apps_own_commands() {
        let d = std::env::temp_dir().join(format!("seqbackend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in 1..=5 {
            std::fs::write(d.join(format!("shot.{f:04}.exr")), b"x").unwrap();
        }
        let b = Backend::new();
        let v = b.open_directory(d.to_str().unwrap(), None).unwrap();
        let seq = v.entities.iter().find(|e| e.kind == "sequence").unwrap();
        b.stage_intent(seq.id, IntentOp::Offset { by: 1000, padding: None }).unwrap();
        assert_eq!(b.pending_entities().unwrap().len(), 1);
        let out = b.commit_pending(None, false, |_| {}).unwrap();
        assert!(out.committed);
        assert!(d.join("shot.1001.exr").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_scan_finds_sequences_in_subfolders() {
        let d = std::env::temp_dir().join(format!("seqsearch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        for f in 1..=3 {
            std::fs::write(d.join(format!("a/b/plate.{f:04}.exr")), b"x").unwrap();
        }
        let b = Backend::new();
        let v = b.search_open(d.to_str().unwrap(), "plate", |_| {}).unwrap();
        assert_eq!(v.entities.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
