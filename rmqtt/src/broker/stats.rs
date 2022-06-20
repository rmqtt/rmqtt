use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::OnceCell;

pub trait Stats: Sync + Send {
    fn handshakings(&self) -> isize;

    fn publishs(&self) -> usize;
    fn delivers(&self) -> usize;
    fn ackeds(&self) -> usize;
}


pub struct DefaultStats {
    publishs: AtomicUsize,
    delivers: AtomicUsize,
    ackeds: AtomicUsize,
}

impl DefaultStats {
    #[inline]
    pub fn instance() -> &'static DefaultStats {
        static INSTANCE: OnceCell<DefaultStats> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            publishs: AtomicUsize::new(0),
            delivers: AtomicUsize::new(0),
            ackeds: AtomicUsize::new(0),
        })
    }


    #[inline]
    pub fn publishs_inc(&self) -> usize {
        self.publishs.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    pub fn delivers_inc(&self) -> usize {
        self.delivers.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    pub fn ackeds_inc(&self) -> usize {
        self.ackeds.fetch_add(1, Ordering::SeqCst)
    }
}

impl Stats for &'static DefaultStats {
    #[inline]
    fn handshakings(&self) -> isize {
        ntex_mqtt::handshakings()
    }

    #[inline]
    fn publishs(&self) -> usize{
        self.publishs.load(Ordering::SeqCst)
    }

    #[inline]
    fn delivers(&self) -> usize{
        self.delivers.load(Ordering::SeqCst)
    }

    #[inline]
    fn ackeds(&self) -> usize{
        self.ackeds.load(Ordering::SeqCst)
    }

}

