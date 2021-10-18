use dynprops::*;
use static_init::dynamic;

#[derive(Extend)]
struct Context {
    param: i32,
    #[prop_data]
    prop_data: PropertyData,
}

#[dynamic]
static DOUBLE: DynInitProperty<Context, i32> =
    new_prop_fn_init(|context: &Context| context.param * 2).into_dyn_init();

#[dynamic]
static SQUARE: DynInitProperty<Context, i32> =
    new_prop_fn_init(|context: &Context| context.param * context.param).into_dyn_init();

#[dynamic]
static SQUARE_PLUS_DOUBLE: DynInitProperty<Context, i32> =
    new_prop_fn_init(|context: &Context| SQUARE.get(context) + DOUBLE.get(context)).into_dyn_init();

#[test]
fn test_static_init() {
    let obj = Context {
        param: 3,
        prop_data: PropertyData::new(),
    };
    assert_eq!(*DOUBLE.get(&obj), 6);
    assert_eq!(*SQUARE.get(&obj), 9);
    assert_eq!(*SQUARE_PLUS_DOUBLE.get(&obj), 15);
}
