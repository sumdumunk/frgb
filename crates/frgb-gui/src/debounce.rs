use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Debouncer<T: Clone + Send + 'static> {
    state: Arc<Mutex<DebouncerState<T>>>,
}

struct DebouncerState<T> {
    last_emitted: Instant,
    interval: Duration,
    pending: Option<T>,
    callback: Box<dyn Fn(T) + Send>,
}

impl<T: Clone + Send + 'static> Debouncer<T> {
    pub fn new(interval: Duration, callback: impl Fn(T) + Send + 'static) -> Self {
        Self {
            state: Arc::new(Mutex::new(DebouncerState {
                last_emitted: Instant::now() - interval,
                interval,
                pending: None,
                callback: Box::new(callback),
            })),
        }
    }

    pub fn update(&self, value: T) {
        let mut s = self.state.lock().unwrap();
        let now = Instant::now();
        if now.duration_since(s.last_emitted) >= s.interval {
            s.last_emitted = now;
            s.pending = None;
            (s.callback)(value);
        } else {
            s.pending = Some(value);
        }
    }

    pub fn flush(&self) {
        let mut s = self.state.lock().unwrap();
        if let Some(value) = s.pending.take() {
            s.last_emitted = Instant::now();
            (s.callback)(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    #[test]
    fn immediate_first_call() {
        let count = Arc::new(AtomicI32::new(0));
        let c = count.clone();
        let d = Debouncer::new(Duration::from_millis(100), move |v: i32| {
            c.store(v, Ordering::SeqCst);
        });
        d.update(42);
        assert_eq!(count.load(Ordering::SeqCst), 42);
    }

    #[test]
    fn suppresses_rapid_updates() {
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let e = emitted.clone();
        let d = Debouncer::new(Duration::from_secs(10), move |v: i32| {
            e.lock().unwrap().push(v);
        });
        d.update(1);
        d.update(2);
        d.update(3);
        assert_eq!(*emitted.lock().unwrap(), vec![1]);
        d.flush();
        assert_eq!(*emitted.lock().unwrap(), vec![1, 3]);
    }
}
