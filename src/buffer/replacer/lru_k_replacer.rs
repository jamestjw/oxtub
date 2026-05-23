use std::collections::HashMap;

use super::replacer::Replacer;

// LRUKReplacer implements the LRU-k replacement policy.
//
// The LRU-k algorithm evicts a frame whose backward k-distance is maximum
// of all frames. Backward k-distance is computed as the difference in time between
// current timestamp and the timestamp of kth previous access.
//
// A frame with less than k historical references is given
// +inf as its backward k-distance. When multiple frames have +inf backward k-distance,
// classical LRU algorithm is used to choose victim.

struct LruKNode {
    // History of last seen K timestamps of this page.
    // Least recent timestamp stored in front.
    history: Vec<usize>,
    is_evictable: bool,
}

impl LruKNode {
    fn new(k: usize) -> Self {
        Self {
            history: Vec::with_capacity(k),
            is_evictable: false,
        }
    }
}

pub struct LruKReplacer {
    nodes: HashMap<usize, LruKNode>,
    current_timestamp: usize,
    // alive, evictable entries count
    curr_size: usize,
    // num frames handled by replacer
    replacer_size: usize,
    k: usize,
}

impl LruKReplacer {
    pub fn new(num_frames: usize, k: usize) -> LruKReplacer {
        if k == 0 {
            panic!("k == 0 is not allowed");
        }

        LruKReplacer {
            nodes: HashMap::new(),
            current_timestamp: 0,
            curr_size: 0,
            replacer_size: num_frames,
            k,
        }
    }
}

impl Replacer for LruKReplacer {
    fn record_access(&mut self, frame_id: usize, _page_id: usize) {
        if !(frame_id < self.replacer_size) {
            panic!("invalid frame id {frame_id}");
        }

        // TODO: this could overflow, but usize is huge, let's not worry about
        // it now. If we are about to overflow, maybe we should just divide
        // all timestamps by 2, including the current timestamp.
        self.current_timestamp += 1;

        let node = self
            .nodes
            .entry(frame_id)
            .or_insert_with(|| LruKNode::new(self.k));

        // The node has just enough space to store k entries
        if node.history.len() + 1 > self.k {
            node.history.remove(0);
        }
        node.history.push(self.current_timestamp);
    }

    fn set_evictable(&mut self, frame_id: usize, evictable: bool) {
        match self.nodes.get_mut(&frame_id) {
            Some(node) => {
                if node.is_evictable != evictable {
                    node.is_evictable = evictable;
                    if evictable {
                        self.curr_size += 1;
                    } else {
                        self.curr_size -= 1;
                    }
                }
            }
            None => panic!("frame {frame_id} does not exist in the replacer"),
        }
    }

    fn evict(&mut self) -> Option<usize> {
        let victim = self
            .nodes
            .iter()
            .filter(|(_, node)| node.is_evictable)
            .min_by_key(|(_, node)| {
                // Sorting by the boolean allows us to prioritse nodes
                // that have been access less than k times, and we break ties
                // based on the last access timestamp like classical LRU
                if node.history.len() < self.k {
                    (false, *node.history.last().unwrap())
                } else {
                    (true, node.history[0])
                }
            })
            .map(|(&frame_id, _)| frame_id);
        if let Some(frame_id) = victim {
            self.nodes.remove(&frame_id);
            self.curr_size -= 1;
            Some(frame_id)
        } else {
            None
        }
    }

    fn remove(&mut self, frame_id: usize) {
        match self.nodes.get(&frame_id) {
            Some(node) => {
                if !node.is_evictable {
                    panic!("frame {frame_id} is not evictable")
                }

                self.nodes.remove(&frame_id);
                self.curr_size -= 1;
            }
            None => panic!("cannot remove inexistent frame {frame_id}"),
        }
    }

    // Return replacer's size, which tracks the number of evictable frames.
    fn size(&self) -> usize {
        self.curr_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_replacer_evicts_nothing() {
        let mut replacer = LruKReplacer::new(10, 2);
        assert_eq!(replacer.evict(), None);
        assert_eq!(replacer.size(), 0);
    }

    #[test]
    fn evict_single_frame() {
        let mut replacer = LruKReplacer::new(10, 2);

        // Doesn't evict pinned frame
        replacer.record_access(1, 0);
        assert_eq!(replacer.size(), 0);
        assert_eq!(replacer.evict(), None);

        // Evicts after unpin
        replacer.set_evictable(1, true);
        assert_eq!(replacer.size(), 1);
        assert_eq!(replacer.evict(), Some(1));
    }

    #[test]
    fn eviction_prefer_frames_fewer_than_k_accesses() {
        // With k = 2:
        // frame 1: accessed twice, finite backward k-distance
        // frame 2: accessed once, +inf backward k-distance
        // Expected victim: frame 2.
        let mut replacer = LruKReplacer::new(10, 2);

        replacer.record_access(1, 0);
        replacer.record_access(1, 0);
        replacer.record_access(2, 0);
        replacer.set_evictable(1, true);
        replacer.set_evictable(2, true);

        assert_eq!(replacer.size(), 2);
        assert_eq!(replacer.evict(), Some(2));
        assert_eq!(replacer.evict(), Some(1));
    }

    #[test]
    fn eviction_tie_breaking_between_frames_fewer_than_k_accesses() {
        // With k = 2:
        // frame 1 accessed at t=1
        // frame 2 accessed at t=2
        // both have < 2 accesses
        // Expected victim: frame 1.
        let mut replacer = LruKReplacer::new(10, 2);
        replacer.record_access(1, 0);
        replacer.record_access(2, 0);
        replacer.set_evictable(1, true);
        replacer.set_evictable(2, true);
        assert_eq!(replacer.evict(), Some(1));
    }

    #[test]
    fn eviction_largest_backward_k_distance_amongst_full_history_frames() {
        // With k = 2:
        // t=1: access frame 1
        // t=2: access frame 2
        // t=3: access frame 1
        // t=4: access frame 2
        // Histories:
        // frame 1: [1, 3]
        // frame 2: [2, 4]
        // Current timestamp is 4.
        // Backward 2-distance:
        // frame 1: 4 - 1 = 3
        // frame 2: 4 - 2 = 2
        // Expected victim: frame 1.

        let mut replacer = LruKReplacer::new(10, 2);
        replacer.record_access(1, 0);
        replacer.record_access(2, 0);
        replacer.record_access(1, 0);
        replacer.record_access(2, 0);
        replacer.set_evictable(1, true);
        replacer.set_evictable(2, true);
        assert_eq!(replacer.evict(), Some(1));
    }

    #[test]
    fn only_last_k_timestamps_retained() {
        let mut replacer = LruKReplacer::new(10, 2);
        replacer.record_access(1, 0); // t=1
        replacer.record_access(2, 0); // t=2
        replacer.record_access(1, 0); // t=3
        replacer.record_access(2, 0); // t=4

        // If we evict here, frame 1 would be the victim

        replacer.record_access(1, 0); // t=5, drops frame 1's t=1
        //
        replacer.set_evictable(1, true);
        replacer.set_evictable(2, true);
        assert_eq!(replacer.evict(), Some(2));
    }

    #[test]
    fn cant_evict_twice() {
        let mut replacer = LruKReplacer::new(10, 2);
        replacer.record_access(1, 0);
        replacer.set_evictable(1, true);
        assert_eq!(replacer.evict(), Some(1));
        assert_eq!(replacer.evict(), None);
    }

    #[test]
    fn size_tracks_evictable_frames() {
        let mut replacer = LruKReplacer::new(10, 2);

        replacer.record_access(1, 0);
        replacer.record_access(2, 0);
        assert_eq!(replacer.size(), 0);

        replacer.set_evictable(1, true);
        assert_eq!(replacer.size(), 1);

        replacer.set_evictable(2, true);
        assert_eq!(replacer.size(), 2);

        replacer.set_evictable(1, false);
        assert_eq!(replacer.size(), 1);
        assert_eq!(replacer.evict(), Some(2));
        assert_eq!(replacer.size(), 0);
    }

    #[test]
    fn remove_evictable_node() {
        let mut replacer = LruKReplacer::new(10, 2);
        replacer.record_access(1, 0);
        replacer.set_evictable(1, true);
        replacer.remove(1);
        assert_eq!(replacer.size(), 0);
        assert_eq!(replacer.evict(), None);
    }

    #[test]
    #[should_panic]
    fn remove_non_evictable_frame_panics() {
        let mut replacer = LruKReplacer::new(10, 2);
        replacer.record_access(1, 0);
        replacer.remove(1);
    }

    #[test]
    #[should_panic]
    fn set_evictable_unknown_frame_panics() {
        let mut replacer = LruKReplacer::new(10, 2);
        replacer.set_evictable(1, true);
    }
}
