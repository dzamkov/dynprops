use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::*;

#[proc_macro_derive(Extend, attributes(prop_data))]
pub fn derive_extend(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let prop_data = match prop_data(&input.data) {
        Ok(prop_data) => prop_data,
        Err(err) => return TokenStream::from(err.to_compile_error()),
    };
    TokenStream::from(quote! {
        unsafe impl #impl_generics Extend for #name #ty_generics #where_clause {
            fn subject() -> &'static ::dynprops::Subject {
                static ONCE: ::std::sync::Once = ::std::sync::Once::new();
                static mut VALUE: *mut ::dynprops::Subject = 0 as *mut ::dynprops::Subject;
                unsafe {
                    ONCE.call_once(|| {
                        let subject = ::dynprops::Subject::new();
                        VALUE = ::std::boxed::Box::into_raw(::std::boxed::Box::new(subject));
                    });
                    &*VALUE
                }
            }

            fn prop_data(&self) -> &::dynprops::PropertyData<#name #ty_generics> {
                #prop_data
            }
        }
    })
}

/// Gets the expression used to access the property data field from a value of a given data type.
fn prop_data(data: &Data) -> syn::Result<TokenStream2> {
    match data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => {
                    let mut prop_data_fields = fields.named.iter().filter(|field| {
                        field
                            .attrs
                            .iter()
                            .any(|attr| attr.path.is_ident("prop_data"))
                    });
                    match as_singleton(&mut prop_data_fields) {
                        Some(prop_data_field) => {
                            let name = prop_data_field.ident.as_ref().unwrap();
                            Ok(quote! { &self.#name })
                        }
                        None => Err(syn::Error::new(
                            data.fields.span(),
                            "Exactly one field must be marked with a #[prop_data] attribute",
                        )),
                    }
                }
                Fields::Unnamed(_) => todo!(),
                Fields::Unit => todo!(), // TODO: Error here
            }
        }
        Data::Enum(_) | Data::Union(_) => todo!(), // TODO: Error here
    }
}

/// Determines whether an iterator has a single value, and if so, returns it.
fn as_singleton<I: Iterator>(it: &mut I) -> Option<I::Item> {
    match it.next() {
        Some(value) => match it.next() {
            Some(_) => None,
            None => Some(value),
        },
        None => None,
    }
}

/// Rewrites a function to automatically memoize its result by storing it as a
/// `Property` value. This requires the function to have exactly one argument, whost type must
/// implement `Extend`.
///
/// There are two possible modes of operation, specified using an argument to the attribute.
/// `clone` (the default) will cause the rewritten function to return a [`clone`](Clone::clone) of
/// the property value. `share` will cause the rewritten function to return an immutable reference
/// to the property value.
///
/// ```
/// use dynprops::{Dynamic, memoize};
/// use std::cell::Cell;
///
/// #[memoize(share)]
/// fn data(context: &Dynamic) -> &Cell<i32> {
///     Cell::new(0)
/// }
///
/// let context = Dynamic::new();
/// assert_eq!(data(&context).get(), 0);
/// data(&context).set(9);
/// assert_eq!(data(&context).get(), 9);
/// ```
#[proc_macro_attribute]
pub fn memoize(args: TokenStream, input: TokenStream) -> TokenStream {
    let opts = match parse_memoize_opts(parse_macro_input!(args)) {
        Ok(opts) => opts,
        Err(err) => {
            return TokenStream::from(err.to_compile_error());
        }
    };
    let input = parse_macro_input!(input as syn::ItemFn);
    TokenStream::from(match memoize_inner(opts, input) {
        Ok(res) => res,
        Err(err) => err.to_compile_error(),
    })
}

fn memoize_inner(opts: MemoizeMode, input: ItemFn) -> syn::Result<TokenStream2> {
    let vis = &input.vis;
    let sig = &input.sig;
    let block = &input.block;
    let arg = match as_singleton(&mut sig.inputs.iter()) {
        Some(FnArg::Typed(arg)) => arg,
        _ => todo!(), // TODO: Error here
    };
    let pat = &arg.pat;
    let arg_ty = match &*arg.ty {
        Type::Reference(TypeReference { elem: ty, .. }) => &**ty,
        _ => todo!(), // TODO: Error here
    };
    let res_ty = match &sig.output {
        ReturnType::Type(_, ty) => &**ty,
        _ => todo!(), // TODO: Error here
    };
    match opts {
        MemoizeMode::Clone => Ok(quote! {
            #vis #sig {
                static ONCE: ::std::sync::Once = ::std::sync::Once::new();
                static mut PROP: *mut ::dynprops::Property<#arg_ty, #res_ty> =
                    0 as *mut ::dynprops::Property<#arg_ty, #res_ty>;
                let prop = unsafe {
                    ONCE.call_once(|| {
                        let prop = ::dynprops::Property::new();
                        PROP = ::std::boxed::Box::into_raw(::std::boxed::Box::new(prop));
                    });
                    &*PROP
                };
                <#res_ty as Clone>::clone(prop.get_with_init(#pat, || {
                    #block
                }))
            }
        }),
        MemoizeMode::Share => {
            let inner_ty = match &*res_ty {
                Type::Reference(TypeReference { elem, .. }) => elem,
                _ => {
                    return Err(syn::Error::new(
                        res_ty.span(),
                        "Expected reference type when using `share`",
                    ))
                }
            };
            Ok(quote! {
                #vis #sig {
                    static ONCE: ::std::sync::Once = ::std::sync::Once::new();
                    static mut PROP: *mut ::dynprops::Property<#arg_ty, #inner_ty> =
                        0 as *mut ::dynprops::Property<#arg_ty, #inner_ty>;
                    let prop = unsafe {
                        ONCE.call_once(|| {
                            let prop = ::dynprops::Property::new();
                            PROP = ::std::boxed::Box::into_raw(::std::boxed::Box::new(prop));
                        });
                        &*PROP
                    };
                    prop.get_with_init(#pat, || {
                        #block
                    })
                }
            })
        }
    }
}

fn parse_memoize_opts(args: AttributeArgs) -> syn::Result<MemoizeMode> {
    let mut mode = MemoizeMode::Clone;
    for arg in args {
        match arg {
            NestedMeta::Meta(Meta::Path(id)) => match id.get_ident() {
                Some(id) if id == "clone" => mode = MemoizeMode::Clone,
                Some(id) if id == "share" => mode = MemoizeMode::Share,
                _ => return Err(syn::Error::new(id.span(), "Unexpect attribute argument")),
            },
            _ => return Err(syn::Error::new(arg.span(), "Unexpect attribute argument")),
        }
    }
    Ok(mode)
}

/// The operation mode for the [`memoize`] attribute.
enum MemoizeMode {
    Clone,
    Share,
}
