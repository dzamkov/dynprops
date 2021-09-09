A rust crate for creating and extending objects with typed dynamic properties.

## Example
```rust
use dynprops::{Subject, Dynamic};

let subject = Subject::new();
let prop_a = subject.new_prop_const_init(5);
let prop_b = subject.new_prop_const_init("Foo");
let mut obj = Dynamic::new(&subject);
assert_eq!(obj[&prop_a], 5);
assert_eq!(obj[&prop_b], "Foo");

// Properties can be changed on a mutable object
obj[&prop_b] = "Foobar";
assert_eq!(obj[&prop_b], "Foobar");

// New properties can be introduced after an object is already created
let prop_c = subject.new_prop_default_init::<u32>();
assert_eq!(obj[&prop_c], 0u32);

// Properties can be initialized based on a function of other properties on the object
let prop_d = subject.new_prop_fn_init(|obj| obj[&prop_b].len());
assert_eq!(obj[&prop_d], 6);
```

## Use Case: "Almost Static" Resources
You may be familiar with the [lazy_static](https://crates.io/crates/lazy_static) or [static_init](https://crates.io/crates/static_init) crates.
These allow you to define immutable `static` values with arbitrarily complicated initializers. The values can then be used at any point in your program, from any thread.
This is a great pattern for reusable resources that need no context to construct, e.g. a lookup table mapping names to hardcoded functions. It's certainly easier than constructing
every resource upfront and passing them down to every function that *could* use them. It's also great for modularization, since you can define the resources in just the module
that needs them and no other module has to be aware of them.

But what do you do about the resources that need just a little bit of context? For example, a particular mesh loaded on the GPU, or a stored procedure in a database.
We can't construct these resources without having context (GPU device handle, database connection) that is not available statically, but the resources are still common and
potentially reusable. Do you cheat and force the context in a `static` variable, thereby incurring the wrath of the dreaded "global state"? Or do you abandon statics altogether,
and go back to a humble life of manually initializing every resource that could ever be needed upfront?

Well, using this crate, you don't have to choose. You can attach those resources as "typed dynamic properties" on the context, and they will be initialized automatically upon first access. Lets
look at the GPU mesh example. The context we're using here is a [`wgpu::Device`](https://github.com/gfx-rs/wgpu). First, we would define a `Subject` to encapsulate the properties
for this context:

```rust
use static_init::dynamic;
use dynprops::{Subject, Extended};

#[dynamic]
pub static DEVICE: Subject<wgpu::Device> = Subject::new();
```

Then, we would define a `Property` associated with the subject. In this case, a cube mesh:

```rust
pub struct Mesh {
  vertex_buffer: wgpu::Buffer,
  index_buffer: wgpu::Buffer,
  index_count: usize
}

pub fn build_cube_mesh(device: &wgpu::Device) -> Mesh {
  // ...
}

#[dynamic]
pub static CUBE_MESH: DynInitProperty<'static, wgpu::Device, Mesh> = DEVICE
    .new_prop_fn_init(|device| build_cube_mesh(&device.value))
    .into_dyn_init();
```

When we initially create a device, we'll have to wrap it in an `Extended` to give it access to properties from a particular `Subject`:

```rust
let device = create_device();
let device = Extended::new_extend(device, &DEVICE);
```

Then, whenever we want access to our mesh, e.g. for rendering, we just access the property on the extended device:

```rust
pub fn render_cube(device: &Extended<'static, wgpu::Device>) {
  let cube_mesh = device[&CUBE_MESH];
  cube_mesh.render(&device.value); // Use .value to access the base value for an Extended
}
```

Just like that, we defined a reusable resource that we can use without explicit passing or global state.
