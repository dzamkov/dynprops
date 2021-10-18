//! Creating and extending objects with typed dynamic properties.
//!
//! ## Example
//!
//! ```
//! use dynprops::*;
//!
//! // Define a type that can be extended with dynamic properties. To automatically derive Extend,
//! // the type must be a struct with exactly one PropertyData field marked with #[prop_data]
//! #[derive(Extend)]
//! struct Dynamic { #[prop_data] prop_data: PropertyData }
//!
//! // Create and access properties on an value
//! let prop_a = new_prop_const_init(5);
//! let mut prop_b = new_prop_const_init("Foo");
//! let mut obj = Dynamic { prop_data: PropertyData::new() };
//! assert_eq!(*prop_a.get(&obj), 5);
//! assert_eq!(*prop_b.get(&obj), "Foo");
//!
//! // Mutable properties can be changed on an object (even if the object is not mutable)
//! prop_b.set(&obj, "Foobar");
//! assert_eq!(*prop_b.get(&obj), "Foobar");
//!
//! // New properties can be introduced after an object is already created
//! let prop_c = new_prop_default_init::<Dynamic, u32>();
//! assert_eq!(*prop_c.get(&obj), 0u32);
//!
//! // Properties can be initialized based on a function of other properties on the object
//! let prop_d = new_prop_fn_init(|obj| prop_b.get(&obj).len());
//! assert_eq!(*prop_d.get(&obj), 6);
//! ```
pub use dynprops_derive::*;
use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::cmp::max;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::{mem, ptr, usize};

/// Types which can store values for arbitrary [`Property`]s.
pub unsafe trait Extend {
    /// Gets the [`Subject`] which identifies which [`Property`]s apply to values of this type.
    /// This must return the same subject every time it is called.
    fn subject() -> &'static Subject;

    /// Gets the [`PropertyData`] for this object.
    fn prop_data(&self) -> &PropertyData;
}

/// Identifies a category of objects and a dynamic set of [`Property`]s that apply to those objects.
pub struct Subject {
    info: Mutex<SubjectInfo>,
}

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

impl Subject {
    /// Creates a new subject.
    pub fn new() -> Self {
        Subject {
            info: Mutex::new(SubjectInfo {
                next_chunk_id: 0,
                open_chunks: Vec::new(),
            }),
        }
    }

    fn alloc_prop<P>(&self) -> PropertyInfo {
        let mut info = self.info.lock().unwrap();
        return info.alloc_prop::<P>();
    }
}

const MIN_CHUNK_BODY_SIZE: usize = 128;

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

/// Identifies a property that is present on objects of type `T`.
pub struct Property<T: Extend, P, I: Init<T, P>> {
    info: PropertyInfo,
    initer: I,
    _phantom: PhantomData<fn(T) -> P>,
}

impl<T: Extend, P, I: Init<T, P>> Property<T, P, I> {
    /// Creates a new property with the given value initializer.
    pub fn new(initer: I) -> Self {
        Self {
            info: T::subject().alloc_prop::<P>(),
            initer,
            _phantom: PhantomData,
        }
    }

    /// Gets the value of this property on the given object.
    pub fn get<'a>(&'a self, obj: &'a T) -> &'a P {
        unsafe { obj.prop_data().get(&self.info, || self.initer.init(&obj)) }
    }

    /// Gets a mutable reference to the value of this property on the given object.
    pub fn get_mut<'a>(&'a mut self, obj: &'a T) -> &'a mut P {
        unsafe {
            obj.prop_data()
                .get_mut(&self.info, || self.initer.init(&obj))
        }
    }

    /// Sets the value of this property on the given object.
    pub fn set(&mut self, obj: &T, value: P) {
        unsafe { obj.prop_data().set(&self.info, value) }
    }

    /// Replaces the initializer on this property.
    pub fn into_other_init<N: Init<T, P>>(self, initer: N) -> Property<T, P, N> {
        Property {
            info: self.info,
            initer: initer,
            _phantom: PhantomData,
        }
    }
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

/// Creates a new [`Property`] whose values are initialized using [`Default::default`].
pub fn new_prop_default_init<T: Extend, P: Default>() -> DefaultInitProperty<T, P> {
    Property::new(DefaultInit)
}

/// Creates a new [`Property`] whose values are initialized to the given constant.
pub fn new_prop_const_init<T: Extend, P: Clone>(value: P) -> ConstInitProperty<T, P> {
    Property::new(ConstInit { value })
}

/// Creates a new [`Property`] whose values are initialized using the given function.
pub fn new_prop_fn_init<T: Extend, P, F: Fn(&T) -> P>(init_fn: F) -> FnInitProperty<T, P, F> {
    Property::new(FnInit { init_fn })
}

impl<T: Extend, P, I: 'static + Init<T, P> + Sync> Property<T, P, I> {
    /// Converts this property into a [`DynInitProperty`] by wrapping its initializer in a
    /// [`DynInit`]. Note that this will add overhead if it is already a [`DynInitProperty`].
    pub fn into_dyn_init(mut self) -> DynInitProperty<T, P> {
        unsafe {
            let initer = Box::new(ptr::read(&mut self.initer)) as DynInit<'static, T, P>;
            self.into_other_init(initer)
        }
    }
}

/// Defines how a [`Property`] is initialized when first accessed.
pub trait Init<T, P> {
    /// Creates the initial value for the property on the given object.
    fn init(&self, obj: &T) -> P;
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

impl<T, P, F: Fn(&T) -> P> Init<T, P> for FnInit<F> {
    fn init(&self, obj: &T) -> P {
        (self.init_fn)(obj)
    }
}

impl<T, P: Clone> Init<T, P> for ConstInit<P> {
    fn init(&self, _obj: &T) -> P {
        self.value.clone()
    }
}

impl<T, P: Default> Init<T, P> for DefaultInit {
    fn init(&self, _obj: &T) -> P {
        Default::default()
    }
}

impl<'a, T, P> Init<T, P> for DynInit<'a, T, P> {
    fn init(&self, obj: &T) -> P {
        self.as_ref().init(obj)
    }
}

/// Encapsulates the values for all the [`Property`]s on an object.
pub struct PropertyData {
    chunks: Mutex<Vec<Chunk>>,
}

impl PropertyData {
    /// Creates a [`PropertyData`] with all properties uninitialized.
    pub fn new() -> Self {
        PropertyData {
            chunks: Mutex::new(Vec::new()),
        }
    }

    /// Gets a dynamic property in this [`PropertyData`], initializing it if needed.
    unsafe fn get<P>(&self, info: &PropertyInfo, initer: impl Fn() -> P) -> &P {
        // Search for chunk
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, info.chunk_id) {
            Ok(chunk) => {
                if let Some(res) = chunk.try_get_mut::<P>(info.offset, info.init_bit_offset) {
                    // Extending lifetime here because we need to drop the lock while returning
                    // a reference to something behind it. This is okay because the contents of the
                    // reference are initialized and can't change anymore (without a mutable
                    // reference to the the property).
                    return mem::transmute(res);
                }
            }
            Err(_) => {}
        }

        // Initialize value (make sure not to hold lock due to the potential for recursive access)
        // TODO: Prevent simultaneous initializations of same value
        drop(chunks);
        let init_value = initer();

        // Search for chunk again
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, info.chunk_id) {
            Ok(chunk) => {
                let res = chunk.get_mut_with_init(info.offset, info.init_bit_offset, init_value);
                return mem::transmute(res);
            }
            Err(after) => {
                // Initialize chunk
                let chunk = Chunk::new(&info.chunk);
                chunks.insert(after, chunk);
                let chunk = &mut chunks[after];
                let res = chunk.get_mut_with_init(info.offset, info.init_bit_offset, init_value);
                return mem::transmute(res);
            }
        }
    }

    /// Gets a mutable reference to a dynamic property in this [`PropertyData`], initializing
    /// it if needed.
    unsafe fn get_mut<P>(&self, info: &PropertyInfo, initer: impl Fn() -> P) -> &mut P {
        // Search for chunk
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, info.chunk_id) {
            Ok(chunk) => {
                if let Some(res) = chunk.try_get_mut::<P>(info.offset, info.init_bit_offset) {
                    return mem::transmute(res);
                }
            }
            Err(_) => {}
        }

        // Initialize value (make sure not to hold lock due to the potential for recursive access)
        // TODO: Prevent simultaneous initializations of same value
        drop(chunks);
        let init_value = initer();

        // Search for chunk again
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, info.chunk_id) {
            Ok(chunk) => {
                let res = chunk.get_mut_with_init(info.offset, info.init_bit_offset, init_value);
                return mem::transmute(res);
            }
            Err(after) => {
                // Initialize chunk
                let chunk = Chunk::new(&info.chunk);
                chunks.insert(after, chunk);
                let chunk = &mut chunks[after];
                let res = chunk.get_mut_with_init(info.offset, info.init_bit_offset, init_value);
                return mem::transmute(res);
            }
        }
    }

    /// Sets the value of a dynamic property in this [`PropertyData`].
    unsafe fn set<P>(&self, info: &PropertyInfo, value: P) {
        // Search for chunk
        let mut chunks = self.chunks.lock().unwrap();
        match Self::find_chunk_mut(&mut chunks, info.chunk_id) {
            Ok(chunk) => {
                chunk.set(info.offset, info.init_bit_offset, value);
            }
            Err(after) => {
                // Initialize chunk
                let mut chunk = Chunk::new(&info.chunk);
                chunk.set(info.offset, info.init_bit_offset, value);
                chunks.insert(after, chunk);
            }
        }
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

/// Describes a chunk within [`PropertyData`].
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
