use crate::*;

#[test]
fn test_dyn_add() {
    let subject = Subject::new();
    let dynamic = Dynamic::new(&subject);
    for i in 0..100 {
        let prop = subject.new_prop_const_init(i);
        assert_eq!(dynamic[&prop], i);
    }
}

#[test]
#[should_panic]
fn test_wrong_subject() {
    let subject_a = Subject::new();
    let subject_b = Subject::new();
    let _prop_a = subject_a.new_prop_const_init(1);
    let prop_b = subject_b.new_prop_const_init(2);
    let dynamic_a = Dynamic::new(&subject_a);
    let _ = dynamic_a[&prop_b];
}

mod lazy_static {
    use crate::*;
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
}
