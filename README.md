A rust crate for creating and extending objects with typed dynamic properties.

## Example
```rust
// Define a type that can be extended with dynamic properties. To automatically derive Extend,
// the type must be a struct with exactly one PropertyData field marked with #[prop_data]
#[derive(Extend)]
struct Thing { #[prop_data] prop_data: PropertyData }

// Create and access properties on an value
let prop_a = new_prop_const_init(5);
let mut prop_b = new_prop_const_init("Foo");
let thing = Thing { prop_data: PropertyData::new() };
assert_eq!(*prop_a.get(&thing), 5);
assert_eq!(*prop_b.get(&thing), "Foo");

// Mutable properties can be changed on a value (even if the value is not mutable)
prop_b.set(&thing, "Foobar");
assert_eq!(*prop_b.get(&thing), "Foobar");

// New properties can be introduced after an object is already created
let prop_c = new_prop_default_init::<Thing, u32>();
assert_eq!(*prop_c.get(&thing), 0u32);

// Properties can be initialized based on a function of other properties on the object
let prop_d = new_prop_fn_init(|thing| prop_b.get(&thing).len());
assert_eq!(*prop_d.get(&thing), 6);
```