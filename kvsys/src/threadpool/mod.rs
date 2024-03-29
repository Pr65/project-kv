//! A thread pool directly shipped from TRPL
//!
//! Read TRPL for more information, no documentation will be added since the interface is already
//! intuitive enough

use std::thread;
use std::sync::{mpsc, Arc, Mutex};
use log::info;

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

type Job = Box<dyn FnBox + Send + 'static>;

enum Message {
    NewJob(Job),
    Terminate
}

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Message>
}

impl ThreadPool {
    pub fn new(size: usize) -> ThreadPool {
        assert!(size > 0);

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size);
        for id in 0..size {
            workers.push(Worker::new(id, receiver.clone()));
        }

        ThreadPool { workers, sender }
    }

    pub fn execute<F>(&self, f: F) where F: FnOnce() + Send + 'static {
        info!("sending a job");
        let job = Box::new(f);
        self.sender.send(Message::NewJob(job)).unwrap();
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        info!("sending terminate message to all workers");
        for _ in &mut self.workers {
            self.sender.send(Message::Terminate).unwrap();
        }

        info!("shutting down all workers");
        for worker in &mut self.workers {
            info!("shutting down worker {}", worker.id);
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        }
    }
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let thread = thread::spawn(move || {
            loop {
                let message = receiver.lock().unwrap().recv().unwrap();
                match message {
                    Message::NewJob(job) => {
                        info!("worker {} got a job, executing", id);
                        job.call_box();
                        info!("worker {} finished its job", id);
                    },
                    Message::Terminate => break
                }
            }
        });
        Worker { id, thread: Some(thread) }
    }
}
