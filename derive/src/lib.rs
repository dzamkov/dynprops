use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::*;

#[proc_macro_derive(Extend, attributes(prop_data))]
pub fn derive_extend(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let prop_data = prop_data(&input.data);
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
fn prop_data(data: &Data) -> TokenStream2 {
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
                            quote! { &self.#name }
                        }
                        None => todo!("Non-singleton"), // TODO: Error here
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
