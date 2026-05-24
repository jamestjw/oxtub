use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use crate::buffer::{bpm::BufferPoolInner, page::Page};

pub struct ReadPageGuard<'a> {
    bpm: &'a BufferPoolInner,
    page_id: usize,
    frame_id: usize,
    guard: RwLockReadGuard<'a, Page>,
}

pub struct WritePageGuard<'a> {
    bpm: &'a BufferPoolInner,
    page_id: usize,
    frame_id: usize,
    guard: RwLockWriteGuard<'a, Page>,
}

impl Drop for ReadPageGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.bpm.state.lock().unwrap();
        let frame_meta = &mut state.frame_metas[self.frame_id];

        assert!(frame_meta.pin_count > 0);
        frame_meta.pin_count -= 1;

        if frame_meta.pin_count == 0 {
            state.replacer.set_evictable(self.frame_id, true);
        }
    }
}

impl std::ops::Deref for ReadPageGuard<'_> {
    type Target = Page;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl Drop for WritePageGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.bpm.state.lock().unwrap();
        let frame_meta = &mut state.frame_metas[self.frame_id];

        frame_meta.is_dirty = true;

        assert!(frame_meta.pin_count > 0);
        frame_meta.pin_count -= 1;

        if frame_meta.pin_count == 0 {
            state.replacer.set_evictable(self.frame_id, true);
        }
    }
}

impl std::ops::Deref for WritePageGuard<'_> {
    type Target = Page;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl std::ops::DerefMut for WritePageGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}
