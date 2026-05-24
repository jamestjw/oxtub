use std::{
    sync::mpsc::{Sender, channel},
    thread::JoinHandle,
};

use crate::storage::disk::{
    config::DEFAULT_PAGE_SIZE, disk_manager::DiskManager, error::DiskSchedulerError,
};

struct PageBuffer {
    data: [u8; DEFAULT_PAGE_SIZE],
}

impl PageBuffer {
    pub fn new() -> Self {
        Self {
            data: [0; DEFAULT_PAGE_SIZE],
        }
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

enum DiskRequest {
    Read {
        page_id: usize,
        response: Sender<Result<PageBuffer, DiskSchedulerError>>,
    },
    Write {
        page_id: usize,
        data: PageBuffer,
        response: Sender<Result<(), DiskSchedulerError>>,
    },
}

struct DiskScheduler {
    worker: Option<JoinHandle<()>>,
    sender: Option<Sender<DiskRequest>>,
}

impl DiskScheduler {
    pub fn new(mut disk_manager: DiskManager) -> Self {
        let (sender, receiver) = channel::<DiskRequest>();
        let worker = std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                match request {
                    DiskRequest::Read { page_id, response } => {}
                    DiskRequest::Write {
                        page_id,
                        data,
                        response,
                    } => {}
                }
            }
        });

        Self {
            worker: Some(worker),
            sender: Some(sender),
        }
    }
}

impl Drop for DiskScheduler {
    fn drop(&mut self) {
        {
            self.sender.take();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
