use io_uring::{IoUring, squeue, cqueue};

pub struct RingDriver {
    ring: IoUring,
}

impl RingDriver {
    pub fn new(entries: u32) -> std::io::Result<Self> {
        let ring = IoUring::new(entries)?;
        Ok(Self { ring })
    }

    pub fn submit_and_wait(&mut self, want: usize) -> std::io::Result<usize> {
        self.ring.submit_and_wait(want)
    }

    pub fn completion_queue(&mut self) -> cqueue::CompletionQueue<'_> {
        self.ring.completion()
    }

    pub fn submission_queue(&mut self) -> squeue::SubmissionQueue<'_> {
        self.ring.submission()
    }
}
