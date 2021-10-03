use dynprops::*;
use lazy_static::*;

struct Context {
    param: i32,
}

lazy_static! {
    static ref CONTEXT: Subject<Context> = Subject::new();
    static ref DOUBLE: DynInitProperty<Context, i32> = CONTEXT
        .new_prop_fn_init(|context| { context.value.param * 2 })
        .into_dyn_init();
    static ref SQUARE: DynInitProperty<Context, i32> = CONTEXT
        .new_prop_fn_init(|context| { context.value.param * context.value.param })
        .into_dyn_init();
    static ref SQUARE_PLUS_DOUBLE: DynInitProperty<Context, i32> = CONTEXT
        .new_prop_fn_init(|context| { context.get(&SQUARE) + context.get(&DOUBLE) })
        .into_dyn_init();
}

#[test]
fn test_lazy_static() {
    let obj = Extended::new_extend(Context { param: 3 }, &CONTEXT);
    assert_eq!(*obj.get(&DOUBLE), 6);
    assert_eq!(*obj.get(&SQUARE), 9);
    assert_eq!(*obj.get(&SQUARE_PLUS_DOUBLE), 15);
}
