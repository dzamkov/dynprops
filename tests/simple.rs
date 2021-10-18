use dynprops::*;
use std::cell::Cell;
use std::sync::Arc;

#[derive(Extend)]
struct Dynamic {
    #[prop_data]
    prop_data: PropertyData,
}

#[test]
fn test_new_prop() {
    let dynamic = Dynamic {
        prop_data: PropertyData::new(),
    };
    for i in 0..100 {
        let prop = new_prop_const_init(i);
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
        let prop_a = new_prop_const_init(DropCounter::new(tracker.clone()));
        let dynamic_a = Dynamic {
            prop_data: PropertyData::new(),
        };
        let prop_b = new_prop_const_init(DropCounter::new(tracker.clone()));
        prop_a.get(&dynamic_a).touch();
        prop_b.get(&dynamic_a).touch();
        let dynamic_b = Dynamic {
            prop_data: PropertyData::new(),
        };
        prop_b.get(&dynamic_b).touch();
        drop(dynamic_a);
        prop_a.get(&dynamic_b).touch();
        drop(prop_b);
    }
    assert!(Arc::get_mut(&mut tracker).is_some());
}
