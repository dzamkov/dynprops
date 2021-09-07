//! Creating and extending objects with typed dynamic properties.
//!
//! ## Example
//!
//! ```
//! use dynprops::{Subject, Dynamic};
//!
//! let subject = Subject::new();
//! let prop_a = subject.new_prop_const_init(5);
//! let prop_b = subject.new_prop_const_init("Foo");
//! let mut obj = Dynamic::new(&subject);
//! assert_eq!(obj[&prop_a], 5);
//! assert_eq!(obj[&prop_b], "Foo");
//!
//! // Properties can be changed on a mutable object
//! obj[&prop_b] = "Foobar";
//! assert_eq!(obj[&prop_b], "Foobar");
//!
//! // New properties can be introduced after an object is already created
//! let prop_c = subject.new_prop_default_init::<u32>();
//! assert_eq!(obj[&prop_c], 0u32);
//!
//! // Properties can be initialized based on a function of other properties on the object
//! let prop_d = subject.new_prop_fn_init(|obj| obj[&prop_b].len());
//! assert_eq!(obj[&prop_d], 6);
//! ```
//!
//! ## Property Lifetime
//! The lifetime of a reference to a property on an object is limited by both the object and
//! the property itself. This allows memory to be reclaimed/reused for properties that have been
//! dropped.
//!
//! ```compile_fail
//! use dynprops::{Subject, Dynamic};
//!
//! let subject = Subject::new();
//! let prop = subject.new_prop_const_init(5);
//! let mut obj = Dynamic::new(&subject);
//! let x = &mut obj[&prop];
//! drop(prop);
//! *x = 10; // ERROR: This reference requires prop to be alive
//! ```

use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::cmp::max;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::{mem, ptr, usize};

#[cfg(test)]
mod test;

pub struct Subject<T> {
    layout: Mutex<SubjectLayout>,
    _phantom: PhantomData<*const T>,
}

struct SubjectLayout {
    size: usize,
    align: usize,
    init_words: Vec<InitWord>,
    drop_props: Vec<DropProperty>,
}

struct InitWord {
    offset: usize,
    in_use: usize,
}

struct DropProperty {
    offset: usize,
    init_bit_offset: usize,
    drop: unsafe fn(NonNull<u8>),
}

struct PropertyInfo {
    offset: usize,
    init_bit_offset: usize,
}

impl<T> Subject<T> {
    pub fn new() -> Self {
        Subject {
            layout: Mutex::new(SubjectLayout {
                size: 0,
                align: 1,
                init_words: Vec::new(),
                drop_props: Vec::new(),
            }),
            _phantom: PhantomData,
        }
    }

    pub fn new_prop<'a, P, I: Init<T, P>>(&'a self, initer: I) -> Property<'a, T, P, I> {
        let info = self.alloc_prop::<P>();
        return Property {
            subject: self,
            offset: info.offset,
            init_bit_offset: info.init_bit_offset,
            initer,
            _phantom: PhantomData,
        };
    }

    pub fn new_prop_default_init<'a, P: Default>(&'a self) -> DefaultInitProperty<'a, T, P> {
        self.new_prop(DefaultInit)
    }

    pub fn new_prop_const_init<'a, P: Clone>(&'a self, value: P) -> ConstInitProperty<'a, T, P> {
        self.new_prop(ConstInit { value })
    }

    pub fn new_prop_fn_init<'a, P, F: Fn(&Extended<T>) -> P>(
        &'a self,
        init_fn: F,
    ) -> FnInitProperty<'a, T, P, F> {
        self.new_prop(FnInit { init_fn })
    }

    pub fn new_prop_dyn_default_init<'a, P: Default>(&'a self) -> DynInitProperty<'a, T, P> {
        self.new_prop(Box::new(DefaultInit)) // TODO: Reusable box
    }

    pub fn new_prop_dyn_const_init<'a, P: 'a + Sync + Clone>(
        &'a self,
        value: P,
    ) -> DynInitProperty<'a, T, P> {
        self.new_prop(Box::new(ConstInit { value }))
    }

    pub fn new_prop_dyn_fn_init<'a, P>(
        &'a self,
        init_fn: impl 'a + Sync + Fn(&Extended<T>) -> P,
    ) -> DynInitProperty<'a, T, P> {
        self.new_prop(Box::new(FnInit { init_fn }))
    }

    fn pin_layout(&self) -> Layout {
        let guard = self.layout.lock().unwrap();
        unsafe {
            return Layout::from_size_align_unchecked(guard.size, guard.align);
        }
    }

    fn alloc_prop<P>(&self) -> PropertyInfo {
        let mut layout = self.layout.lock().unwrap();
        return layout.alloc_prop::<P>();
    }

    fn free_prop<P>(&self, offset: usize) {
        if !mem::needs_drop::<P>() {
            let mut layout = self.layout.lock().unwrap();
            return layout.free_nodrop_prop(offset);
        }
    }
}

unsafe impl<T> Sync for Subject<T> {}

impl SubjectLayout {
    fn alloc_prop<P>(&mut self) -> PropertyInfo {
        let init_bit_offset = self.alloc_init_bit();
        let offset = self.alloc::<P>();
        if mem::needs_drop::<P>() {
            let drop = Self::drop_option_in_place::<P>;
            self.drop_props.push(DropProperty {
                offset,
                init_bit_offset,
                drop,
            });
        }
        return PropertyInfo {
            offset,
            init_bit_offset,
        };
    }

    unsafe fn drop_option_in_place<P>(ptr: NonNull<u8>) {
        ptr::drop_in_place(ptr.cast::<Option<P>>().as_ptr());
    }

    fn alloc_init_bit(&mut self) -> usize {
        // Search for an existing word that we can allocate a bit in
        for init_word in self.init_words.iter_mut() {
            if init_word.in_use != usize::MAX {
                let bit = init_word.in_use.trailing_ones() as usize;
                init_word.in_use |= 1 << bit;
                return init_word.offset * 8 + bit;
            }
        }

        // Allocate a new word
        let offset = self.alloc::<usize>();
        let mut in_use = 0;
        let bit = 0;
        in_use |= 1 << bit;
        self.init_words.push(InitWord { offset, in_use });
        return offset * 8 + bit;
    }

    fn alloc<P>(&mut self) -> usize {
        self.alloc_raw(mem::size_of::<P>(), mem::align_of::<P>())
    }

    fn alloc_raw(&mut self, size: usize, align: usize) -> usize {
        let offset = (self.size + align - 1) & !(align - 1);
        self.size = offset + size;
        self.align = max(self.align, align);
        println!("Alloc {} {}", offset, size);
        return offset;
    }

    fn free_nodrop_prop(&mut self, _offset: usize) {
        // TODO: remove from layout
    }
}

pub struct Property<'a, T, P, I: 'a + Init<T, P>> {
    subject: &'a Subject<T>,
    offset: usize,
    init_bit_offset: usize,
    initer: I,
    _phantom: PhantomData<*const P>,
}

pub type DefaultInitProperty<'a, T, P> = Property<'a, T, P, DefaultInit>;

pub type ConstInitProperty<'a, T, P> = Property<'a, T, P, ConstInit<P>>;

pub type FnInitProperty<'a, T, P, F> = Property<'a, T, P, FnInit<F>>;

pub type DynInitProperty<'a, T, P> = Property<'a, T, P, DynInit<'a, T, P>>;

unsafe impl<'a, T, P, I: Sync + Init<T, P>> Sync for Property<'a, T, P, I> {}

impl<'a, T, P, I: Init<T, P>> Drop for Property<'a, T, P, I> {
    fn drop(&mut self) {
        self.subject.free_prop::<P>(self.offset);
    }
}

/// Defines how a [Property] is initialized when first accessed.
pub trait Init<T, P> {
    /// Creates the initial value for the property on the given object.
    fn init(&self, obj: &Extended<T>) -> P;
}

pub struct DefaultInit;

pub struct ConstInit<P: Clone> {
    value: P,
}

pub struct FnInit<F> {
    init_fn: F,
}

pub type DynInit<'a, T, P> = Box<dyn 'a + Sync + Init<T, P>>;

impl<T, P, F: Fn(&Extended<T>) -> P> Init<T, P> for FnInit<F> {
    fn init(&self, obj: &Extended<T>) -> P {
        (self.init_fn)(obj)
    }
}

impl<T, P: Clone> Init<T, P> for ConstInit<P> {
    fn init(&self, _obj: &Extended<T>) -> P {
        self.value.clone()
    }
}

impl<T, P: Default> Init<T, P> for DefaultInit {
    fn init(&self, _obj: &Extended<T>) -> P {
        Default::default()
    }
}

impl<'a, T, P> Init<T, P> for DynInit<'a, T, P> {
    fn init(&self, obj: &Extended<T>) -> P {
        self.as_ref().init(obj)
    }
}

pub struct Extended<'a, T> {
    pub value: T,
    subject: &'a Subject<T>,
    data: Data,
}

pub type Dynamic<'a> = Extended<'a, ()>;

impl<'a, T> Extended<'a, T> {
    pub fn new_extend(value: T, subject: &'a Subject<T>) -> Self {
        Extended {
            value,
            subject,
            data: Data::new(subject.pin_layout()),
        }
    }

    fn index_raw<P, I: Init<T, P>>(&self, index: &Property<'a, T, P, I>) -> NonNull<P> {
        // Verify subject
        if (self.subject as *const Subject<T>) != (index.subject as *const Subject<T>) {
            panic!("Subject mismatch");
        }

        // Check for initialization
        let get_data_layout = || self.subject.pin_layout();
        let init_word_offset = (index.init_bit_offset / 8) & !(mem::align_of::<usize>() - 1);
        let init_word_bit = index.init_bit_offset - (init_word_offset * 8);
        unsafe {
            let init_word = self
                .data
                .get_ptr(get_data_layout, init_word_offset, mem::size_of::<usize>())
                .cast::<usize>()
                .as_mut();
            let value_ptr = self
                .data
                .get_ptr(get_data_layout, index.offset, mem::size_of::<P>())
                .cast::<P>();
            let init_bit = (*init_word & (1 << init_word_bit)) != 0;
            if !init_bit {
                // Do initialization
                let init_value = index.initer.init(&self);
                ptr::write(value_ptr.as_ptr(), init_value);
                *init_word |= 1 << init_word_bit;
            }
            return value_ptr;
        }
    }
}

impl<'a> Dynamic<'a> {
    pub fn new(subject: &'a Subject<()>) -> Self {
        Self::new_extend((), subject)
    }
}

impl<'a, 'b, T, P, I: Init<T, P>> Index<&'b Property<'a, T, P, I>> for Extended<'b, T> {
    type Output = P;

    fn index(&self, index: &Property<'a, T, P, I>) -> &Self::Output {
        unsafe {
            return &(*self.index_raw(index).as_ref());
        }
    }
}

impl<'a, 'b, T, P, I: Init<T, P>> IndexMut<&'b Property<'a, T, P, I>> for Extended<'b, T> {
    fn index_mut(&mut self, index: &Property<'a, T, P, I>) -> &mut Self::Output {
        unsafe {
            return &mut (*self.index_raw(index).as_mut());
        }
    }
}

struct Data {
    head_ptr: NonNull<u8>,
}

struct ChunkHeader {
    overflow_ptr: *mut u8,
    chunk_layout: Layout,
    data_ptr: *mut u8,
    data_end: usize,
}

impl Data {
    fn new(data_layout: Layout) -> Self {
        unsafe {
            Data {
                head_ptr: Self::alloc_chunk(0, data_layout),
            }
        }
    }

    unsafe fn alloc_chunk(data_start: usize, data_layout: Layout) -> NonNull<u8> {
        let data_end = data_layout.size();
        let chunk_size = data_end - data_start;
        let chunk_layout = Layout::from_size_align_unchecked(chunk_size, data_layout.align());
        let header_layout = Layout::new::<ChunkHeader>();
        let (chunk_layout, offset) = header_layout.extend(chunk_layout).unwrap();
        let ptr = alloc_zeroed(chunk_layout);
        let ptr = match NonNull::new(ptr) {
            Some(ptr) => ptr,
            None => handle_alloc_error(chunk_layout),
        };
        let data_ptr = ptr.as_ptr().add(offset).sub(data_start);
        ptr::write(
            ptr.cast::<ChunkHeader>().as_ptr(),
            ChunkHeader {
                overflow_ptr: ptr::null_mut(),
                chunk_layout,
                data_ptr,
                data_end,
            },
        );
        return ptr;
    }

    unsafe fn get_ptr(
        &self,
        get_data_layout: impl FnOnce() -> Layout,
        offset: usize,
        size: usize,
    ) -> NonNull<u8> {
        let header = self.head_ptr.cast::<ChunkHeader>().as_mut();
        return Self::get_ptr_in_chunk(header, get_data_layout, offset, size);
    }

    unsafe fn get_ptr_in_chunk(
        header: &mut ChunkHeader,
        get_data_layout: impl FnOnce() -> Layout,
        offset: usize,
        size: usize,
    ) -> NonNull<u8> {
        if offset + size <= header.data_end {
            return NonNull::new_unchecked(header.data_ptr.add(offset));
        } else {
            match NonNull::new(header.overflow_ptr) {
                Some(overflow_ptr) => {
                    let overflow_header = overflow_ptr.cast::<ChunkHeader>().as_mut();
                    return Self::get_ptr_in_chunk(overflow_header, get_data_layout, offset, size);
                }
                None => {
                    let overflow_ptr = Self::alloc_chunk(header.data_end, get_data_layout());
                    let overflow_header = overflow_ptr.cast::<ChunkHeader>().as_mut();
                    header.overflow_ptr = overflow_ptr.as_ptr();
                    assert!(offset + size <= overflow_header.data_end);
                    return NonNull::new_unchecked(overflow_header.data_ptr.add(offset));
                }
            }
        }
    }
}
