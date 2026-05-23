pub trait Replacer {
    fn record_access(&mut self, frame_id: usize, page_id: usize);
    fn set_evictable(&mut self, frame_id: usize, evictable: bool);
    fn remove(&mut self, frame_id: usize);
    fn size(&self) -> usize;

    // Evicts a frame and returns the frame_id if it was successful
    fn evict(&mut self) -> Option<usize>;
}
