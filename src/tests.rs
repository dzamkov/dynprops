use std::cell::Cell;
use crate::*;

#[test]
fn test_dyn_add() {
    let subject = Subject::new();
    let dynamic = Dynamic::new(&subject);
    for i in 0..100 {
        let prop = subject.new_prop_const_init(i);
        assert_eq!(dynamic[&prop], i);
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
    let _ = dynamic_a[&prop_b];
}

struct DropCounter<'a> {
    num_alive: &'a Cell<u32>,
    is_alive: Cell<bool>
}

impl<'a> DropCounter<'a> {
    fn new(num_alive: &'a Cell<u32>) -> Self {
        num_alive.set(num_alive.get() + 1);
        let is_alive = Cell::new(true);
        DropCounter { num_alive, is_alive }
    }

    fn touch(&self) {
        assert!(self.is_alive.get());
    }
}

impl<'a> Clone for DropCounter<'a> {
    fn clone(&self) -> Self {
        assert!(self.is_alive.get());
        DropCounter::new(self.num_alive)
    }
}

impl<'a> Drop for DropCounter<'a> {
    fn drop(&mut self) {
        assert!(self.is_alive.get());
        self.num_alive.set(self.num_alive.get() - 1);
    }
}

#[test]
fn test_drop() {
    let num_alive = Cell::new(0);
    {
        let subject = Subject::new();
        let prop_a = subject.new_prop_const_init(DropCounter::new(&num_alive));
        let dynamic_a = Dynamic::new(&subject);
        let prop_b = subject.new_prop_const_init(DropCounter::new(&num_alive));
        dynamic_a[&prop_a].touch();
        dynamic_a[&prop_b].touch();
        let dynamic_b = Dynamic::new(&subject);
        dynamic_b[&prop_b].touch();
        drop(dynamic_a);
        dynamic_b[&prop_a].touch();
        drop(prop_b);
    }
    assert_eq!(num_alive.get(), 0);
}