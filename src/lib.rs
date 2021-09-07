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

use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::cmp::max;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::{mem, ptr, usize};

/// Identifies a category of objects and a dynamic set of [`Property`]s that apply to those objects.
/// New properties can be introduced into the subject at any time using [`Subject::new_prop`]
/// and its derivatives. When accessing a property of an object, the subject of the property
/// must match the subject of the object.
///
/// ## Lifetime
/// The [`Subject`] must be alive for the entire duration that any [`Property`], [`Extended`] or
/// [`Dynamic`] are associated with it. This is enforced by a lifetime parameter on those types. In
/// practice, subjects will usually be bound to the `'static` lifetime.
pub struct Subject<T> {
    layout: Mutex<SubjectLayout>,
    _phantom: PhantomData<fn(T)>,
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
    /// Creates a new subject.
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

    /// Creates a new property within this subject. An [`Init`] must be supplied to specify how the
    /// initial value of the property is determined.
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

    /// Creates a new property within this subject. Upon initialization, the property will have the
    /// default value (as defined by [`Default::default()`]) for type `P`.
    pub fn new_prop_default_init<'a, P: Default>(&'a self) -> DefaultInitProperty<'a, T, P> {
        self.new_prop(DefaultInit)
    }

    /// Creates a new property within this subject. Upon initialization, the property will have the
    /// given value.
    pub fn new_prop_const_init<'a, P: Clone>(&'a self, value: P) -> ConstInitProperty<'a, T, P> {
        self.new_prop(ConstInit { value })
    }

    /// Creates a new property within this subject. Upon initialization, the value of the property
    /// will be determined by executing the given closure.
    ///
    /// Since the closure takes the object itself, the initializer may reference the base value or
    /// any other property that has been defined on [`Subject`]. For example:
    /// ```
    /// use dynprops::{Subject, Extended};
    ///
    /// let subject = Subject::new();
    /// let prop_value = subject.new_prop_fn_init(|obj| obj.value);
    /// let prop_double_value = subject.new_prop_fn_init(|obj| obj[&prop_value] * 2);
    /// let prop_square_value = subject.new_prop_fn_init(|obj| obj[&prop_value] * obj[&prop_value]);
    /// let obj = Extended::new_extend(20, &subject);
    /// assert_eq!(obj[&prop_value], 20);
    /// assert_eq!(obj[&prop_double_value], 40);
    /// assert_eq!(obj[&prop_square_value], 400);
    /// ```
    /// The constraints on property lifetimes ensure that circular references between property
    /// initializers are impossible.
    pub fn new_prop_fn_init<'a, P, F: Fn(&Extended<T>) -> P>(
        &'a self,
        init_fn: F,
    ) -> FnInitProperty<'a, T, P, F> {
        self.new_prop(FnInit { init_fn })
    }

    /// Creates a new property within this subject. This works identically to
    /// [`Self::new_prop_default_init`], but returns a [`DynInitProperty`].
    pub fn new_prop_dyn_default_init<'a, P: Default>(&'a self) -> DynInitProperty<'a, T, P> {
        self.new_prop(Box::new(DefaultInit)) // TODO: Reusable box
    }

    /// Creates a new property within this subject. This works identically to
    /// [`Self::new_prop_const_init`], but returns a [`DynInitProperty`].
    pub fn new_prop_dyn_const_init<'a, P: 'a + Sync + Clone>(
        &'a self,
        value: P,
    ) -> DynInitProperty<'a, T, P> {
        self.new_prop(Box::new(ConstInit { value }))
    }

    /// Creates a new property within this subject. This works identically to
    /// [`Self::new_prop_fn_init`], but returns a [`DynInitProperty`].
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
        return offset;
    }

    fn free_nodrop_prop(&mut self, _offset: usize) {
        // TODO: remove from layout
    }
}

/// Identifies a property that is present on objects of the appropriate [`Subject`].
///
/// ## Property Lifetime
/// The lifetime of a reference to a property on an object is limited by both the object and
/// the property itself. This allows memory to be reclaimed/reused for properties that have been
/// dropped.
///
/// ```compile_fail
/// use dynprops::{Subject, Dynamic};
///
/// let subject = Subject::new();
/// let prop = subject.new_prop_const_init(5);
/// let mut obj = Dynamic::new(&subject);
/// let x = &mut obj[&prop];
/// drop(prop);
/// *x = 10; // ERROR: This reference requires prop to be alive
/// ```
pub struct Property<'a, T, P, I: 'a + Init<T, P>> {
    subject: &'a Subject<T>,
    offset: usize,
    init_bit_offset: usize,
    initer: I,
    _phantom: PhantomData<fn() -> P>,
}

/// A shortcut for a [`Property`] that is initialized by a [`DefaultInit`].
pub type DefaultInitProperty<'a, T, P> = Property<'a, T, P, DefaultInit>;

/// A shortcut for a [`Property`] that is initialized by a [`ConstInit`].
pub type ConstInitProperty<'a, T, P> = Property<'a, T, P, ConstInit<P>>;

/// A shortcut for a [`Property`] that is initialized by a [`FnInit`].
pub type FnInitProperty<'a, T, P, F> = Property<'a, T, P, FnInit<F>>;

/// A shortcut for a [`Property`] that is initialized by a [`DynInit`].
pub type DynInitProperty<'a, T, P> = Property<'a, T, P, DynInit<'a, T, P>>;

impl<'a, T, P, I: Init<T, P>> Property<'a, T, P, I> {
    /// Gets the [`Subject`] this property is associated with. This defines which [`Dynamic`]s and
    /// [`Extended`]s contain this property.
    pub fn subject(&self) -> &Subject<T> {
        self.subject
    }
}

impl<'a, T, P, I: Init<T, P>> Drop for Property<'a, T, P, I> {
    fn drop(&mut self) {
        self.subject.free_prop::<P>(self.offset);
    }
}

/// Defines how a [`Property`] is initialized when first accessed.
pub trait Init<T, P> {
    /// Creates the initial value for the property on the given object.
    fn init(&self, obj: &Extended<T>) -> P;
}

/// An [`Init`] which initializes values using [`Default::default()`].
pub struct DefaultInit;

/// An [`Init`] which initializes values by cloning a given value.
pub struct ConstInit<P: Clone> {
    pub value: P,
}

/// An [`Init`] which initializes values by executing a closure.
pub struct FnInit<F> {
    pub init_fn: F,
}

/// An [`Init`] that uses dynamic dispatch to defer to another [`Init`] at runtime.
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

/// Extends a value of type `T` with properties defined in a particular [`Subject<T>`].
///
/// Property values are accessed by index, like so:
/// ```
/// use dynprops::{Subject, Extended};
///
/// let subject = Subject::new();
/// let prop = subject.new_prop_default_init();
/// let mut obj = Extended::new_extend(5, &subject);
/// obj[&prop] = "Foo";
/// assert_eq!(obj[&prop], "Foo");
/// ```
///
/// The base value of an [`Extended`] object can always be accessed through the `value` field:
/// ```
/// use dynprops::{Subject, Extended};
///
/// let subject = Subject::new();
/// let mut obj = Extended::new_extend(5, &subject);
/// obj.value = 15;
/// assert_eq!(obj.value, 15);
/// ```
pub struct Extended<'a, T> {
    pub value: T,
    subject: &'a Subject<T>,
    data: Mutex<ExtendedData>,
}

/// An object consisting entirely of dynamic properties defined in a particular [`Subject`].
///
/// Property values are accessed by index, like so:
/// ```
/// use dynprops::{Subject, Dynamic};
/// let subject = Subject::new();
/// let prop = subject.new_prop_default_init();
/// let mut obj = Dynamic::new(&subject);
/// obj[&prop] = "Bar";
/// assert_eq!(obj[&prop], "Bar");
/// ```
pub type Dynamic<'a> = Extended<'a, ()>;

impl<'a, T> Extended<'a, T> {
    /// Creates an [`Extended`] wrapper over the given value. This extends it with all of the
    /// [`Property`]s defined on `subject`.
    pub fn new_extend(value: T, subject: &'a Subject<T>) -> Self {
        Extended {
            value,
            subject,
            data: Mutex::new(ExtendedData::new(subject.pin_layout())),
        }
    }

    /// Gets the [`Subject`] this object is associated with. This defines which [`Property`]s are
    /// available on the object.
    pub fn subject(&self) -> &Subject<T> {
        self.subject
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
            let mut data = self.data.lock().unwrap();
            let init_word = data
                .get_ptr(get_data_layout, init_word_offset)
                .cast::<usize>()
                .as_mut();
            let value_ptr = data.get_ptr(get_data_layout, index.offset).cast::<P>();

            // Drop the lock as soon as we have the pointers we need. We don't want to hold the
            // lock during initialization, since other properties within the same object can
            // be referenced.
            drop(data);

            let init_bit = (*init_word & (1 << init_word_bit)) != 0;
            if !init_bit {
                // Do initialization
                let init_value = index.initer.init(&self);

                // Lock to write the initial value
                let data = self.data.lock().unwrap();
                let init_bit = (*init_word & (1 << init_word_bit)) != 0;
                if !init_bit {
                    ptr::write(value_ptr.as_ptr(), init_value);
                    *init_word |= 1 << init_word_bit;
                }
                drop(data);
            }
            return value_ptr;
        }
    }
}

impl<'a> Dynamic<'a> {
    /// Creates a new [`Dynamic`] object.
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

impl<'a, T> Drop for Extended<'a, T> {
    fn drop(&mut self) {
        let mut data = self.data.lock().unwrap();
        let layout = self.subject.layout.lock().unwrap();
        for prop in layout.drop_props.iter() {
            let get_data_layout = || self.subject.pin_layout();
            let init_word_offset = (prop.init_bit_offset / 8) & !(mem::align_of::<usize>() - 1);
            let init_word_bit = prop.init_bit_offset - (init_word_offset * 8);
            unsafe {
                let init_word = data
                    .get_ptr(get_data_layout, init_word_offset)
                    .cast::<usize>()
                    .as_mut();
                let init_bit = (*init_word & (1 << init_word_bit)) != 0;
                if init_bit {
                    // Drop property
                    let value_ptr = data.get_ptr(get_data_layout, prop.offset);
                    (prop.drop)(value_ptr);
                }
            }
        }
    }
}

/// The internal representation of the data for a [Extended]. Note that, unlike [Vec], we can't move
/// underlying data when new properties are added. That's because properties can be initialized
/// from a shared borrow at the same time that pre-existing properties are being referenced. This
/// limits are design choices here considerably. The current implementation stores data in either
/// a "head" chunk or an "overflow" chunk. The "head" chunk consists of the properties that
/// existed when the [Extended] was created, and the "overflow" chunks contain blocks of new
/// properties that were added later on.
struct ExtendedData {
    head_chunk: Chunk,
    overflow_chunks: Vec<Chunk>,
}

/// Describes a chunk within [ExtendedData].
struct Chunk {
    ptr: NonNull<u8>,
    layout: Layout,
    data_end: usize,
}

impl ExtendedData {
    fn new(head_layout: Layout) -> Self {
        ExtendedData {
            head_chunk: Chunk::new(head_layout, head_layout.size()),
            overflow_chunks: Vec::new(),
        }
    }

    /// Gets a pointer to a particular property within the [ExtendedData], given the offset of the
    /// property within the entire [SubjectLayout]. If an additional chunk needs to be allocated,
    /// `get_data_layout` will be used to get the latest size/alignment of the [SubjectLayout].
    fn get_ptr(&mut self, get_data_layout: impl FnOnce() -> Layout, offset: usize) -> NonNull<u8> {
        // Is the data in the head chunk?
        if offset < self.head_chunk.data_end {
            unsafe {
                return NonNull::new_unchecked(self.head_chunk.ptr.as_ptr().add(offset));
            }
        }

        // Is the data in an existing overflow chunk?
        let mut overflow_data_end = self.head_chunk.data_end;
        match self.overflow_chunks.last() {
            Some(last_overflow_chunk) => {
                overflow_data_end = last_overflow_chunk.data_end;
                if offset < last_overflow_chunk.data_end {
                    // Use binary search to figure out which chunk
                    let mut lo_chunk_index = 0;
                    let mut hi_chunk_index = self.overflow_chunks.len() - 1;
                    let mut chunk_data_start = self.head_chunk.data_end;
                    loop {
                        if !(lo_chunk_index < hi_chunk_index) {
                            break;
                        }
                        let mid_chunk_index = (lo_chunk_index + hi_chunk_index) / 2;
                        let mid_chunk = &self.overflow_chunks[mid_chunk_index];
                        if offset < mid_chunk.data_end {
                            hi_chunk_index = mid_chunk_index;
                        } else {
                            chunk_data_start = mid_chunk.data_end;
                            lo_chunk_index = mid_chunk_index + 1;
                        }
                    }
                    unsafe {
                        let overflow_chunk = &self.overflow_chunks[lo_chunk_index];
                        return NonNull::new_unchecked(
                            overflow_chunk
                                .ptr
                                .as_ptr()
                                .sub(chunk_data_start)
                                .add(offset),
                        );
                    }
                }
            }
            _ => {}
        }

        // Create a new overflow chunk
        let data_layout = get_data_layout();
        let new_data_end = data_layout.size();
        assert!(offset < new_data_end);
        let chunk_layout =
            Layout::from_size_align(new_data_end - overflow_data_end, data_layout.align()).unwrap();
        let overflow_chunk = Chunk::new(chunk_layout, new_data_end);
        let result = unsafe {
            NonNull::new_unchecked(
                overflow_chunk
                    .ptr
                    .as_ptr()
                    .sub(overflow_data_end)
                    .add(offset),
            )
        };
        self.overflow_chunks.push(overflow_chunk);
        return result;
    }
}

impl Chunk {
    fn new(layout: Layout, data_end: usize) -> Self {
        let ptr = if layout.size() > 0 {
            unsafe {
                match NonNull::new(alloc_zeroed(layout)) {
                    Some(ptr) => ptr,
                    None => handle_alloc_error(layout),
                }
            }
        } else {
            NonNull::dangling()
        };
        Chunk {
            ptr,
            layout,
            data_end,
        }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        if self.layout.size() > 0 {
            unsafe {
                dealloc(self.ptr.as_ptr(), self.layout);
            }
        }
    }
}

#[cfg(test)]
mod tests;
