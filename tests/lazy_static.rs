use dynprops::*;
use lazy_static::*;

struct Context {
    param: i32,
}

lazy_static! {
    static ref CONTEXT: Subject<Context> = Subject::new();
    static ref DOUBLE: DynInitProperty<'static, Context, i32> = CONTEXT
        .new_prop_fn_init(|context| { context.value.param * 2 })
        .into_dyn_init();
    static ref SQUARE: DynInitProperty<'static, Context, i32> = CONTEXT
        .new_prop_fn_init(|context| { context.value.param * context.value.param })
        .into_dyn_init();
    static ref SQUARE_PLUS_DOUBLE: DynInitProperty<'static, Context, i32> = CONTEXT
        .new_prop_fn_init(|context| { context[&SQUARE] + context[&DOUBLE] })
        .into_dyn_init();
}

#[test]
fn test_lazy_static() {
    let obj = Extended::new_extend(Context { param: 3 }, &CONTEXT);
    assert_eq!(obj[&DOUBLE], 6);
    assert_eq!(obj[&SQUARE], 9);
    assert_eq!(obj[&SQUARE_PLUS_DOUBLE], 15);
}
