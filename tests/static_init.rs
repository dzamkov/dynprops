use dynprops::*;
use static_init::dynamic;

struct Context {
    param: i32,
}

#[dynamic]
static CONTEXT: Subject<Context> = Subject::new();

#[dynamic]
static DOUBLE: DynInitProperty<Context, i32> = CONTEXT
    .new_prop_fn_init(|context| context.value.param * 2)
    .into_dyn_init();

#[dynamic]
static SQUARE: DynInitProperty<Context, i32> = CONTEXT
    .new_prop_fn_init(|context| context.value.param * context.value.param)
    .into_dyn_init();

#[dynamic]
static SQUARE_PLUS_DOUBLE: DynInitProperty<Context, i32> = CONTEXT
    .new_prop_fn_init(|context| context[&SQUARE] + context[&DOUBLE])
    .into_dyn_init();

#[test]
fn test_static_init() {
    let obj = Extended::new_extend(Context { param: 5 }, &CONTEXT);
    assert_eq!(obj[&DOUBLE], 10);
    assert_eq!(obj[&SQUARE], 25);
    assert_eq!(obj[&SQUARE_PLUS_DOUBLE], 35);
}
