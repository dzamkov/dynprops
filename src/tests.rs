use crate::*;
use std::cell::Cell;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

#[test]
fn test_new_prop() {
    let dynamic = Dynamic::new();
    for i in 0..100 {
        let mut prop = Property::new();
        assert_eq!(*prop.get(&dynamic), 0);
        prop.set(&dynamic, i);
        assert_eq!(*prop.get(&dynamic), i);
    }
}

pub struct DropCounter {
    tracker: Arc<()>,
    is_alive: Cell<bool>,
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
fn test_drop() {
    let mut tracker = Arc::new(());
    {
        let prop_a = Property::new();
        let dynamic_a = Dynamic::new();
        let prop_b = Property::new();
        let init = || DropCounter::new(tracker.clone());
        prop_a.get_with_init(&dynamic_a, init).touch();
        prop_b.get_with_init(&dynamic_a, init).touch();
        let dynamic_b = Dynamic::new();
        prop_b.get_with_init(&dynamic_b, init).touch();
        drop(dynamic_a);
        prop_a.get_with_init(&dynamic_b, init).touch();
        drop(prop_b);
    }
    assert!(Arc::get_mut(&mut tracker).is_some());
}

// Generics should have different subjects for each generic parameter, since this will prevent
// inapplicable properties from taking up space in the PropertyData.
#[test]
#[ignore]
fn test_generic_subject() {
    // TODO
    let subject_a = Extended::<u32>::subject();
    let subject_b = Extended::<f32>::subject();
    assert_ne!(subject_a as *const Subject, subject_b as *const Subject);
}

#[derive(Extend)]
struct MemoizeThing {
    num_reads: AtomicUsize,
    #[prop_data]
    prop_data: PropertyData<MemoizeThing>,
}

#[memoize]
fn const_123(obj: &MemoizeThing) -> i32 {
    obj.num_reads
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    123
}

#[memoize]
fn const_123_sqr(obj: &MemoizeThing) -> i32 {
    obj.num_reads
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    const_123(obj) * const_123(obj)
}

#[memoize(share)]
fn const_mutex_hello(obj: &MemoizeThing) -> &Mutex<&'static str> {
    Mutex::new("Hello")
}

#[test]
fn test_memoize() {
    let obj = MemoizeThing {
        num_reads: AtomicUsize::new(0),
        prop_data: PropertyData::new(),
    };
    assert_eq!(const_123(&obj), 123);
    assert_eq!(obj.num_reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(const_123(&obj), 123);
    assert_eq!(obj.num_reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(const_123_sqr(&obj), 15129);
    assert_eq!(obj.num_reads.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(const_123_sqr(&obj), 15129);
    assert_eq!(obj.num_reads.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(*const_mutex_hello(&obj).lock().unwrap(), "Hello");
    *const_mutex_hello(&obj).lock().unwrap() = "World";
    assert_eq!(*const_mutex_hello(&obj).lock().unwrap(), "World");
}
