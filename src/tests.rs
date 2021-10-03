use std::cell::Cell;
use crate::*;

#[test]
fn test_new_prop() {
    let subject = Subject::new();
    let dynamic = Dynamic::new(&subject);
    for i in 0..100 {
        let prop = subject.new_prop_const_init(i);
        assert_eq!(*dynamic.get(&prop), i);
    }
}

#[test]
#[should_panic]
fn test_wrong_subject() {
    let subject_a = Subject::new();
    let subject_b = Subject::new();
    let _prop_a = subject_a.new_prop_const_init(1);
    let prop_b = subject_b.new_prop_const_init(2);
    let dynamic_a = Dynamic::new(&subject_a);
    let _ = dynamic_a.get(&prop_b);
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
fn test_drop() {
    let mut tracker = Arc::new(());
    {
        let subject = Subject::new();
        let prop_a = subject.new_prop_const_init(DropCounter::new(tracker.clone()));
        let dynamic_a = Dynamic::new(&subject);
        let prop_b = subject.new_prop_const_init(DropCounter::new(tracker.clone()));
        dynamic_a.get(&prop_a).touch();
        dynamic_a.get(&prop_b).touch();
        let dynamic_b = Dynamic::new(&subject);
        dynamic_b.get(&prop_b).touch();
        drop(dynamic_a);
        dynamic_b.get(&prop_a).touch();
        drop(prop_b);
    }
    assert!(Arc::get_mut(&mut tracker).is_some());
}