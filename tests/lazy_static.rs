use dynprops::*;
use lazy_static::*;

struct Context {
    param: i32,
}

lazy_static! {
    static ref CONTEXT: Subject<Context> = Subject::new();
    static ref DOUBLE: DynInitProperty<'static, Context, i32> =
        CONTEXT.new_prop_dyn_fn_init(|context| { context.value.param * 2 });
    static ref SQUARE: DynInitProperty<'static, Context, i32> =
        CONTEXT.new_prop_dyn_fn_init(|context| { context.value.param * context.value.param });
    static ref SQUARE_PLUS_DOUBLE: DynInitProperty<'static, Context, i32> =
        CONTEXT.new_prop_dyn_fn_init(|context| { context[&SQUARE] + context[&DOUBLE] });
}

#[test]
fn test_lazy_static() {
    let obj = Extended::new_extend(Context { param: 3 }, &CONTEXT);
    assert_eq!(obj[&DOUBLE], 6);
    assert_eq!(obj[&SQUARE], 9);
    assert_eq!(obj[&SQUARE_PLUS_DOUBLE], 15);
}