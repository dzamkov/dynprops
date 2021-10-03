//! Creating and extending objects with typed dynamic properties.
//!
//! ## Example
//!
//! ```
//! use dynprops::{Subject, Dynamic};
//!
//! let subject = Subject::new();
//! let prop_a = subject.new_prop_const_init(5);
//! let mut prop_b = subject.new_prop_const_init("Foo");
//! let mut obj = Dynamic::new(&subject);
//! assert_eq!(*obj.get(&prop_a), 5);
//! assert_eq!(*obj.get(&prop_b), "Foo");
//!
//! // Mutable properties can be changed on an object (even if the object is not mutable)
//! obj.set(&mut prop_b, "Foobar");
//! assert_eq!(*obj.get(&prop_b), "Foobar");
//!
//! // New properties can be introduced after an object is already created
//! let prop_c = subject.new_prop_default_init::<u32>();
//! assert_eq!(*obj.get(&prop_c), 0u32);
//!
//! // Properties can be initialized based on a function of other properties on the object
//! let prop_d = subject.new_prop_fn_init(|obj| obj.get(&prop_b).len());
//! assert_eq!(*obj.get(&prop_d), 6);
//! ```
use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::cmp::max;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::{mem, ptr, usize};

#[cfg(loom)]
use loom::sync::atomic::AtomicUsize;

#[cfg(loom)]
use loom::sync::{Arc, Mutex};

#[cfg(not(loom))]
use std::sync::atomic::AtomicUsize;

#[cfg(not(loom))]
use std::sync::{Arc, Mutex};

/// Identifies a category of objects and a dynamic set of [`Property`]s that apply to those objects.
/// New properties can be introduced into the subject at any time using [`Subject::new_prop`]
/// and its derivatives. When accessing a property of an object, the subject of the property
/// must match the subject of the object.
pub struct Subject<T> {
    id: usize,
    info: Mutex<SubjectInfo>,
    _phantom: PhantomData<fn(T)>,
}

#[cfg(loom)]
loom::lazy_static! {
    static ref NEXT_SUBJECT_ID: AtomicUsize = AtomicUsize::new(0);
}

#[cfg(not(loom))]
static NEXT_SUBJECT_ID: AtomicUsize = AtomicUsize::new(0);

const MIN_CHUNK_BODY_SIZE: usize = 128;

struct SubjectInfo {
    next_chunk_id: usize,
    open_chunks: Vec<Arc<Mutex<ChunkInfo>>>,
}

struct ChunkInfo {
    id: usize,
    layout: Layout,
    in_use_init_bits: usize,
    in_use_size: usize,
    drop_props: Vec<DropPropertyInfo>,
}

struct DropPropertyInfo {
    offset: usize,
    init_bit_offset: usize,
    drop: unsafe fn(NonNull<u8>),
}

struct PropertyInfo {
    chunk_id: usize,
    chunk: Arc<Mutex<ChunkInfo>>,
    offset: usize,
    init_bit_offset: usize,
}

impl<T> Subject<T> {
    /// Creates a new subject.
    pub fn new() -> Self {
        Subject {
            id: NEXT_SUBJECT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            info: Mutex::new(SubjectInfo {
                next_chunk_id: 0,
                open_chunks: Vec::new(),
            }),
            _phantom: PhantomData,
        }
    }

    /// Creates a new property within this subject. An [`Init`] must be supplied to specify how the
    /// initial value of the property is determined.
    pub fn new_prop<P, I: Init<T, P>>(&self, initer: I) -> Property<T, P, I> {
        let info = self.alloc_prop::<P>();
        return Property {
            subject_id: self.id,
            chunk_id: info.chunk_id,
            chunk: info.chunk,
            offset: info.offset,
            init_bit_offset: info.init_bit_offset,
            initer,
            _phantom: PhantomData,
        };
    }

    /// Creates a new property within this subject. Upon initialization, the property will have the
    /// default value (as defined by [`Default::default()`]) for type `P`.
    pub fn new_prop_default_init<P: Default>(&self) -> DefaultInitProperty<T, P> {
        self.new_prop(DefaultInit)
    }

    /// Creates a new property within this subject. Upon initialization, the property will have the
    /// given value.
    pub fn new_prop_const_init<P: Clone>(&self, value: P) -> ConstInitProperty<T, P> {
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
    /// let prop_double_value = subject.new_prop_fn_init(|obj| obj.get(&prop_value) * 2);
    /// let prop_square_value = subject.new_prop_fn_init(|obj| obj.get(&prop_value) * obj.get(&prop_value));
    /// let obj = Extended::new_extend(20, &subject);
    /// assert_eq!(*obj.get(&prop_value), 20);
    /// assert_eq!(*obj.get(&prop_double_value), 40);
    /// assert_eq!(*obj.get(&prop_square_value), 400);
    /// ```
    /// The constraints on property lifetimes ensure that circular references between property
    /// initializers are impossible.
    pub fn new_prop_fn_init<P, F: Fn(&Extended<T>) -> P>(
        &self,
        init_fn: F,
    ) -> FnInitProperty<T, P, F> {
        self.new_prop(FnInit { init_fn })
    }

    fn alloc_prop<P>(&self) -> PropertyInfo {
        let mut info = self.info.lock().unwrap();
        return info.alloc_prop::<P>();
    }
}

impl SubjectInfo {
    fn alloc_prop<P>(&mut self) -> PropertyInfo {
        // Check for a suitable open chunk to add the property to
        // TODO: Remove unusable open chunks
        for chunk in self.open_chunks.iter() {
            let mut chunk_value = chunk.lock().unwrap();
            if let Some(prop_info) = chunk_value.try_alloc_prop::<P>() {
                return prop_info(chunk.clone());
            }
        }

        // Define a new chunk
        let layout = Layout::from_size_align(
            max(MIN_CHUNK_BODY_SIZE, mem::size_of::<P>()),
            max(mem::align_of::<usize>(), mem::align_of::<P>()),
        )
        .unwrap();
        let mut chunk = ChunkInfo::new(self.next_chunk_id, layout);
        self.next_chunk_id += 1;

        // Allocate property in chunk
        let prop_info = chunk.try_alloc_prop::<P>().unwrap();
        let chunk = Arc::new(Mutex::new(chunk));
        self.open_chunks.push(chunk.clone());
        return prop_info(chunk);
    }
}

impl ChunkInfo {
    fn new(id: usize, layout: Layout) -> Self {
        ChunkInfo {
            id,
            layout,
            in_use_init_bits: 0,
            in_use_size: 0,
            drop_props: Vec::new(),
        }
    }

    fn try_alloc_prop<P>(&mut self) -> Option<impl Fn(Arc<Mutex<ChunkInfo>>) -> PropertyInfo> {
        let size = mem::size_of::<P>();
        let align = mem::align_of::<P>();
        if align <= self.layout.align() && self.in_use_init_bits != usize::MAX {
            let offset = (self.in_use_size + align - 1) & !(align - 1);
            let new_size = offset + size;
            if new_size <= self.layout.size() {
                self.in_use_size = new_size;
                let init_bit_offset = self.in_use_init_bits.trailing_ones() as usize;
                self.in_use_init_bits |= 1 << init_bit_offset;
                if mem::needs_drop::<P>() {
                    self.drop_props.push(DropPropertyInfo {
                        offset,
                        init_bit_offset,
                        drop: Self::drop_in_place::<P>,
                    });
                }
                let chunk_id = self.id;
                return Some(move |chunk| PropertyInfo {
                    chunk_id,
                    chunk,
                    offset,
                    init_bit_offset,
                });
            }
        }
        return None;
    }

    unsafe fn drop_in_place<P>(ptr: NonNull<u8>) {
        ptr::drop_in_place(ptr.cast::<P>().as_ptr());
    }
}

/// Identifies a property that is present on objects of the appropriate [`Subject`].
pub struct Property<T, P, I: Init<T, P>> {
    subject_id: usize,
    chunk_id: usize,
    chunk: Arc<Mutex<ChunkInfo>>,
    offset: usize,
    init_bit_offset: usize,
    initer: I,
    _phantom: PhantomData<fn(T) -> P>,
}

/// A shortcut for a [`Property`] that is initialized by a [`DefaultInit`].
pub type DefaultInitProperty<T, P> = Property<T, P, DefaultInit>;

/// A shortcut for a [`Property`] that is initialized by a [`ConstInit`].
pub type ConstInitProperty<T, P> = Property<T, P, ConstInit<P>>;

/// A shortcut for a [`Property`] that is initialized by a [`FnInit`].
pub type FnInitProperty<T, P, F> = Property<T, P, FnInit<F>>;

/// A shortcut for a [`Property`] that is initialized by a [`DynInit`]. Any property can be
/// converted into a [`DynInitProperty`] using [`Property::into_dyn_init`].
pub type DynInitProperty<T, P> = Property<T, P, DynInit<'static, T, P>>;

impl<T, P, I: 'static + Init<T, P> + Sync> Property<T, P, I> {
    /// Converts this property into a [`DynInitProperty`] by wrapping its initializer in a
    /// [`DynInit`]. Note that this will add overhead if it is already a [`DynInitProperty`].
    pub fn into_dyn_init(mut self) -> DynInitProperty<T, P> {
        unsafe {
            let result = Property {
                subject_id: self.subject_id,
                chunk_id: self.chunk_id,
                chunk: ptr::read(&self.chunk),
                offset: self.offset,
                init_bit_offset: self.init_bit_offset,
                initer: Box::new(ptr::read(&mut self.initer)) as DynInit<'static, T, P>,
                _phantom: PhantomData,
            };
            mem::forget(self);
            return result;
        }
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
/// Property values are accessed using `get`, `get_mut` and `set`, like so:
/// ```
/// use dynprops::{Subject, Extended};
///
/// let subject = Subject::new();
/// let mut prop = subject.new_prop_default_init();
/// let mut obj = Extended::new_extend(5, &subject);
/// obj.set(&mut prop, "Foo");
/// assert_eq!(*obj.get(&prop), "Foo");
/// *obj.get_mut(&mut prop) = "Bar";
/// assert_eq!(*obj.get(&prop), "Bar");
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
pub struct Extended<T> {
    pub value: T,
    subject_id: usize,
    chunks: Mutex<Vec<Chunk>>,
}

/// An object consisting entirely of dynamic properties defined in a particular [`Subject`].
///
/// Property values are accessed using `get`, `get_mut` and `set`, like so:
/// ```
/// use dynprops::{Subject, Dynamic};
///
/// let subject = Subject::new();
/// let mut prop = subject.new_prop_default_init();
/// let mut obj = Dynamic::new(&subject);
/// obj.set(&mut prop, "Foo");
/// assert_eq!(*obj.get(&prop), "Foo");
/// *obj.get_mut(&mut prop) = "Bar";
/// assert_eq!(*obj.get(&prop), "Bar");
/// ```
pub type Dynamic = Extended<()>;

impl<T> Extended<T> {
    /// Creates an [`Extended`] wrapper over the given value. This extends it with all of the
    /// [`Property`]s defined on `subject`.
    pub fn new_extend(value: T, subject: &Subject<T>) -> Self {
        Extended {
            value,
            subject_id: subject.id,
            chunks: Mutex::new(Vec::new()),
        }
    }

    /// Gets a dynamic property in this [`Extended`], initializing it if needed.
    pub fn get<'a, P, I: Init<T, P>>(&'a self, index: &'a Property<T, P, I>) -> &'a P {
        self.assert_subject(index.subject_id);

        // Search for chunk
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, index.chunk_id) {
            Ok(chunk) => unsafe {
                if let Some(res) = chunk.try_get_mut::<P>(index.offset, index.init_bit_offset) {
                    // Extending lifetime here because we need to drop the lock while returning
                    // a reference to something behind it. This is okay because the contents of the
                    // reference are initialized and can't change anymore (without a mutable
                    // reference to the the property).
                    return mem::transmute(res);
                }
            },
            Err(_) => {}
        }

        // Initialize value (make sure not to hold lock due to the potential for recursive access)
        // TODO: Prevent simultaneous initializations of same value
        drop(chunks);
        let init_value = index.initer.init(&self);

        // Search for chunk again
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, index.chunk_id) {
            Ok(chunk) => unsafe {
                let res = chunk.get_mut_with_init(index.offset, index.init_bit_offset, init_value);
                return mem::transmute(res);
            },
            Err(after) => unsafe {
                // Initialize chunk
                let chunk = Chunk::new(&index.chunk);
                chunks.insert(after, chunk);
                let chunk = &mut chunks[after];
                let res = chunk.get_mut_with_init(index.offset, index.init_bit_offset, init_value);
                return mem::transmute(res);
            },
        }
    }

    /// Gets a mutable reference to a dynamic property in this [`Extended`], initializing
    /// it if needed.
    pub fn get_mut<'a, P, I: Init<T, P>>(&'a self, index: &'a mut Property<T, P, I>) -> &'a mut P {
        self.assert_subject(index.subject_id);

        // Search for chunk
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, index.chunk_id) {
            Ok(chunk) => unsafe {
                if let Some(res) = chunk.try_get_mut::<P>(index.offset, index.init_bit_offset) {
                    return mem::transmute(res);
                }
            },
            Err(_) => {}
        }

        // Initialize value (make sure not to hold lock due to the potential for recursive access)
        // TODO: Prevent simultaneous initializations of same value
        drop(chunks);
        let init_value = index.initer.init(&self);

        // Search for chunk again
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, index.chunk_id) {
            Ok(chunk) => unsafe {
                let res = chunk.get_mut_with_init(index.offset, index.init_bit_offset, init_value);
                return mem::transmute(res);
            },
            Err(after) => unsafe {
                // Initialize chunk
                let chunk = Chunk::new(&index.chunk);
                chunks.insert(after, chunk);
                let chunk = &mut chunks[after];
                let res = chunk.get_mut_with_init(index.offset, index.init_bit_offset, init_value);
                return mem::transmute(res);
            },
        }
    }

    /// Sets the value of a dynamic property in this [`Extended`].
    pub fn set<P, I: Init<T, P>>(&self, index: &mut Property<T, P, I>, value: P) {
        self.assert_subject(index.subject_id);

        // Search for chunk
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, index.chunk_id) {
            Ok(chunk) => unsafe {
                chunk.set(index.offset, index.init_bit_offset, value);
            },
            Err(after) => unsafe {
                // Initialize chunk
                let mut chunk = Chunk::new(&index.chunk);
                chunk.set(index.offset, index.init_bit_offset, value);
                chunks.insert(after, chunk);
            },
        }
    }

    /// Asserts that this [`Extended`] is of the given subject.
    fn assert_subject(&self, subject_id: usize) {
        assert_eq!(self.subject_id, subject_id, "Subject mismatch");
    }

    /// Searches for the chunk with the given id within `chunks`. Returns a reference to the chunk
    /// if found, or the index where the chunk would be if it existed.
    fn find_chunk_mut(chunks: &mut Vec<Chunk>, chunk_id: usize) -> Result<&mut Chunk, usize> {
        // Binary search for pre-existing chunk
        let mut lo = 0;
        if chunks.len() > 0 {
            let mut hi = chunks.len();
            loop {
                if !(lo < hi) {
                    break;
                }
                let mid = (lo + hi) / 2;
                let mid_chunk = &mut chunks[mid];
                if chunk_id < mid_chunk.id {
                    hi = mid;
                } else if chunk_id > mid_chunk.id {
                    lo = mid + 1;
                } else {
                    // Need unsafe here because of limitations of borrow checker
                    // https://github.com/rust-lang/rust/issues/43234
                    unsafe {
                        return Ok(mem::transmute(mid_chunk));
                    }
                }
            }
        }
        return Err(lo);
    }
}

impl Dynamic {
    /// Creates a new [`Dynamic`] object.
    pub fn new(subject: &Subject<()>) -> Self {
        Self::new_extend((), subject)
    }
}

/// Describes a chunk within an [`Extended`].
struct Chunk {
    id: usize,
    info: Arc<Mutex<ChunkInfo>>,
    init_word: usize,
    ptr: NonNull<u8>,
}

impl Chunk {
    fn new(info: &Arc<Mutex<ChunkInfo>>) -> Self {
        let info_value = info.lock().unwrap();
        unsafe {
            match NonNull::new(alloc(info_value.layout)) {
                Some(ptr) => Chunk {
                    id: info_value.id,
                    info: info.clone(),
                    init_word: 0,
                    ptr,
                },
                None => handle_alloc_error(info_value.layout),
            }
        }
    }

    /// Attempts to get a reference to a pre-initialized property in this chunk, returning
    /// [`None`] if the the property has not been initialized yet.
    unsafe fn try_get_mut<P>(&mut self, offset: usize, init_bit_offset: usize) -> Option<&mut P> {
        let mut ptr = NonNull::new_unchecked(self.ptr.as_ptr().add(offset)).cast::<P>();
        if (self.init_word & (1 << init_bit_offset)) > 0 {
            return Some(ptr.as_mut());
        } else {
            return None;
        }
    }

    /// Attempts to get a reference to a property in this chunk, using [`init_value`] to initialize
    /// it if it isn't initialized yet.
    unsafe fn get_mut_with_init<P>(
        &mut self,
        offset: usize,
        init_bit_offset: usize,
        init_value: P,
    ) -> &mut P {
        let mut ptr = NonNull::new_unchecked(self.ptr.as_ptr().add(offset)).cast::<P>();
        if (self.init_word & (1 << init_bit_offset)) == 0 {
            self.init_word |= 1 << init_bit_offset;
            ptr::write(ptr.as_ptr(), init_value);
        }
        return ptr.as_mut();
    }

    /// Sets the value of a property in this chunk.
    unsafe fn set<P>(&mut self, offset: usize, init_bit_offset: usize, value: P) {
        let mut ptr = NonNull::new_unchecked(self.ptr.as_ptr().add(offset)).cast::<P>();
        if (self.init_word & (1 << init_bit_offset)) == 0 {
            self.init_word |= 1 << init_bit_offset;
            ptr::write(ptr.as_ptr(), value);
        } else {
            *ptr.as_mut() = value;
        }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        let info = self.info.lock().unwrap();
        for drop_prop in info.drop_props.iter() {
            if (self.init_word & (1 << drop_prop.init_bit_offset)) > 0 {
                unsafe {
                    let ptr = self.ptr.as_ptr().add(drop_prop.offset);
                    (drop_prop.drop)(NonNull::new_unchecked(ptr));
                }
            }
        }
        unsafe {
            dealloc(self.ptr.as_ptr(), info.layout);
        }
    }
}

#[cfg(test)]
#[cfg(not(loom))]
mod tests;

#[cfg(test)]
#[cfg(loom)]
mod tests_loom;
