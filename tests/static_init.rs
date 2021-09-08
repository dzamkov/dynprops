use dynprops::*;
use static_init::dynamic;

struct Context {
    param: i32,
}

#[dynamic]
static CONTEXT: Subject<Context> = Subject::new();

#[dynamic]
static DOUBLE: DynInitProperty<'static, Context, i32> =
    CONTEXT.new_prop_dyn_fn_init(|context| { context.value.param * 2 });
    
#[dynamic]
static SQUARE: DynInitProperty<'static, Context, i32> =
    CONTEXT.new_prop_dyn_fn_init(|context| { context.value.param * context.value.param });

#[dynamic]
static SQUARE_PLUS_DOUBLE: DynInitProperty<'static, Context, i32> =
    CONTEXT.new_prop_dyn_fn_init(|context| { context[&SQUARE] + context[&DOUBLE] });

#[test]
fn test_static_init() {
    let obj = Extended::new_extend(Context { param: 5 }, &CONTEXT);
    assert_eq!(obj[&DOUBLE], 10);
    assert_eq!(obj[&SQUARE], 25);
    assert_eq!(obj[&SQUARE_PLUS_DOUBLE], 35);
}