use std::{
    sync::mpsc::{Sender, channel},
    thread::JoinHandle,
};

use crate::{
    buffer::page::PageBytes,
    common::types::PageId,
    storage::disk::{
        config::DEFAULT_PAGE_SIZE, disk_manager::DiskManager, error::DiskSchedulerError,
    },
};

pub struct PageBuffer {
    data: PageBytes,
}

impl Default for PageBuffer {
    fn default() -> Self {
        Self {
            data: [0; DEFAULT_PAGE_SIZE],
        }
    }
}
impl PageBuffer {
    pub fn of_data(data: PageBytes) -> Self {
        Self { data }
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
        page_id: PageId,
        response: Sender<Result<PageBuffer, DiskSchedulerError>>,
    },
    Write {
        page_id: PageId,
        data: Box<PageBuffer>,
        response: Sender<Result<(), DiskSchedulerError>>,
    },
    Delete {
        page_id: PageId,
    },
}

pub struct DiskScheduler {
    worker: Option<JoinHandle<()>>,
    sender: Option<Sender<DiskRequest>>,
}

impl DiskScheduler {
    pub fn new(mut disk_manager: DiskManager) -> Self {
        let (sender, receiver) = channel::<DiskRequest>();
        let worker = std::thread::spawn(move || {
            // receiver returns error when all senders are closed, i.e. channel
            // is closed
            while let Ok(request) = receiver.recv() {
                match request {
                    DiskRequest::Read { page_id, response } => {
                        let mut buffer = PageBuffer::default();
                        let result = disk_manager
                            .read_page(page_id, buffer.data_mut())
                            .map(|_| buffer)
                            .map_err(DiskSchedulerError::Disk);

                        if let Err(e) = response.send(result) {
                            tracing::warn!(page_id, error = %e, "disk request response receiver was dropped")
                        }
                    }
                    DiskRequest::Write {
                        page_id,
                        data,
                        response,
                    } => {
                        let result = disk_manager
                            .write_page(page_id, data.data())
                            .map_err(DiskSchedulerError::Disk);
                        let _ = response.send(result);
                    }
                    DiskRequest::Delete { page_id } => {
                        if let Err(e) = disk_manager.delete_page(page_id) {
                            tracing::warn!(page_id, error = %e, "could not delete page")
                        };
                    }
                }
            }
        });

        Self {
            worker: Some(worker),
            sender: Some(sender),
        }
    }

    pub fn read_page(&self, page_id: PageId) -> Result<PageBuffer, DiskSchedulerError> {
        match &self.sender {
            None => Err(DiskSchedulerError::WorkerStopped),
            Some(sender) => {
                let (resp_sender, resp_receiver) = channel();
                if sender
                    .send(DiskRequest::Read {
                        page_id,
                        response: resp_sender,
                    })
                    .is_err()
                {
                    return Err(DiskSchedulerError::WorkerUnreachable);
                }

                match resp_receiver.recv() {
                    Err(_) => Err(DiskSchedulerError::WorkerStopped),
                    Ok(res) => res,
                }
            }
        }
    }

    pub fn write_page(&self, page_id: PageId, data: PageBuffer) -> Result<(), DiskSchedulerError> {
        match &self.sender {
            None => Err(DiskSchedulerError::WorkerStopped),
            Some(sender) => {
                let (resp_sender, resp_receiver) = channel();
                if sender
                    .send(DiskRequest::Write {
                        page_id,
                        data: Box::new(data),
                        response: resp_sender,
                    })
                    .is_err()
                {
                    return Err(DiskSchedulerError::WorkerUnreachable);
                }

                match resp_receiver.recv() {
                    Err(_) => Err(DiskSchedulerError::WorkerStopped),
                    Ok(res) => res,
                }
            }
        }
    }

    pub fn delete_page(&self, page_id: PageId) -> Result<(), DiskSchedulerError> {
        match &self.sender {
            None => Err(DiskSchedulerError::WorkerStopped),
            Some(sender) => sender
                .send(DiskRequest::Delete { page_id })
                .map_err(|_| DiskSchedulerError::WorkerUnreachable),
        }
    }
}

impl Drop for DiskScheduler {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
