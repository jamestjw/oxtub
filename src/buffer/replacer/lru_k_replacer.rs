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
    frame_id: usize,
    is_evictable: bool,
}

impl LruKNode {
    fn new(frame_id: usize, k: usize) -> Self {
        Self {
            history: Vec::with_capacity(k),
            is_evictable: false,
            frame_id,
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
    fn record_access(&mut self, frame_id: usize, page_id: usize) {
        // TODO: this could overflow, but usize is huge, let's not worry about
        // it now. If we are about to overflow, maybe we should just divide
        // all timestamps by 2, including the current timestamp.
        self.current_timestamp += 1;

        let node = self
            .nodes
            .entry(frame_id)
            .or_insert_with(|| LruKNode::new(frame_id, self.k));

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
        todo!()
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
