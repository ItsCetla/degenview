//! Small shared pool for blocking, replaceable background requests.

use std::sync::{Mutex, OnceLock, mpsc};

type Job = Box<dyn FnOnce() + Send + 'static>;
static QUEUE: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

pub fn submit(job: impl FnOnce() + Send + 'static) {
    let sender = QUEUE.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = std::sync::Arc::new(Mutex::new(receiver));
        for _ in 0..4 {
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
