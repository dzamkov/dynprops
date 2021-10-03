use loom::thread;
use loom::sync::atomic::AtomicUsize;
use loom::sync::Arc;
use loom::sync::atomic::Ordering;
use std::cell::Cell;
use crate::*;

#[test]
fn test_concurrent_init() {
    loom::model(|| {
        let subject = Arc::new(Subject::new());
        let dynamic = Arc::new(Dynamic::new(&subject));
        let counter = AtomicUsize::new(0);
        let prop = Arc::new(subject.new_prop_fn_init(move |_| {
            counter.fetch_add(1, Ordering::SeqCst)
        }));
        let handle_0 = {
            let dynamic = dynamic.clone();
            let prop = prop.clone();
            thread::spawn(move || {
                *dynamic.get(&prop)
            })
        };
        let handle_1 = {
            let dynamic = dynamic.clone();
            let prop = prop.clone();
            thread::spawn(move || {
                *dynamic.get(&prop)
            })
        };
        let counter_0 = handle_0.join().unwrap();
        let counter_1 = handle_1.join().unwrap();
        assert_eq!(counter_0, counter_1);
        // TODO: Additionally require counter to be 1 (no excess initializations)
    });
}

pub struct DropCounter {
    tracker: Arc<()>,
    is_alive: Cell<bool>
}

impl DropCounter {
    pub fn new(tracker: Arc<()>) -> Self {
        let is_alive = Cell::new(true);
        DropCounter { tracker, is_alive }
    }

    pub fn touch(&self) {
        assert!(self.is_alive.get());
    }
}

impl Clone for DropCounter {
    fn clone(&self) -> Self {
        assert!(self.is_alive.get());
        DropCounter::new(self.tracker.clone())
    }
}

impl Drop for DropCounter {
    fn drop(&mut self) {
        assert!(self.is_alive.get());
        self.is_alive.set(false);
    }
}

#[test]
fn test_concurrent_drop() {
    loom::model(|| {
        let mut tracker = Arc::new(());
        let subject = Arc::new(Subject::new());
        let prop = Arc::new(subject.new_prop_const_init(DropCounter::new(tracker.clone())));
        let dynamic = Arc::new(Dynamic::new(&subject));
        let handle_0 = {
            let subject = subject.clone();
            let tracker = tracker.clone();
            let prop = prop.clone();
            let dynamic = dynamic.clone();
            thread::spawn(move || {
                let prop_local = subject.new_prop_const_init(DropCounter::new(tracker.clone()));
                dynamic.get(&prop_local).touch();
                dynamic.get(&prop).touch();
            })
        };
        let handle_1 = {
            let subject = subject.clone();
            let prop = prop.clone();
            let dynamic = dynamic.clone();
            thread::spawn(move || {
                let dynamic_local = Dynamic::new(&subject);
                dynamic.get(&prop).touch();
                dynamic_local.get(&prop).touch();
            })
        };
        handle_0.join().unwrap();
        handle_1.join().unwrap();
        drop(dynamic);
        drop(prop);
        assert!(Arc::get_mut(&mut tracker).is_some());
    });
}