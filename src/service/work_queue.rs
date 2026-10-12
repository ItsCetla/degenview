//! Small shared pools for blocking, replaceable background requests.

use std::sync::{Arc, Mutex, OnceLock, mpsc};

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed set of worker threads, started on first use, fed from one queue.
pub struct WorkQueue {
    workers: usize,
    sender: OnceLock<mpsc::Sender<Job>>,
}

impl WorkQueue {
    pub const fn new(workers: usize) -> Self {
        Self {
            workers,
            sender: OnceLock::new(),
        }
    }

    pub fn submit(&self, job: impl FnOnce() + Send + 'static) {
        let sender = self.sender.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Job>();
            let receiver = Arc::new(Mutex::new(receiver));
            for _ in 0..self.workers {
                let receiver = receiver.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = receiver.lock().expect("work queue poisoned").recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                });
            }
            sender
        });
        let _ = sender.send(Box::new(job));
    }
}

static QUEUE: WorkQueue = WorkQueue::new(4);
/// Image downloads get their own workers, so a screenful of new logos never queues in front of a
/// chart's candles.
pub static IMAGES: WorkQueue = WorkQueue::new(4);

pub fn submit(job: impl FnOnce() + Send + 'static) {
    QUEUE.submit(job);
}
