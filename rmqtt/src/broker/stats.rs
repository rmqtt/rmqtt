use std::sync::atomic::{AtomicIsize, Ordering};

use once_cell::sync::OnceCell;

pub trait Stats: Sync + Send {
    fn handshakings(&self) -> isize;
    fn handshakings_add(&self, v: isize) -> isize;
}


pub struct DefaultStats {
    handshakings: AtomicIsize,
}

impl DefaultStats {
    #[inline]
    pub fn instance() -> &'static DefaultStats {
        static INSTANCE: OnceCell<DefaultStats> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            handshakings: AtomicIsize::new(0),
        })
    }
}

impl Stats for &'static DefaultStats {
    #[inline]
    fn handshakings(&self) -> isize {
        self.handshakings.load(Ordering::SeqCst)
    }

    #[inline]
    fn handshakings_add(&self, v: isize) -> isize {
        self.handshakings.fetch_add(v, Ordering::SeqCst)
    }
}

