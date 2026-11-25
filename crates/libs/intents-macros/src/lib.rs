mod intent;
mod record;
mod thing;
mod words;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::ToTokens;
use syn::{Attribute, Data, DeriveInput, Expr, ExprLit, Field, Fields, Ident, Lit, Meta};

/// Declares an intent: its struct's fields are the parameters, its doc comments the descriptions.
///
/// `#[intent(id = "clock.create_timer", title = text!("New Timer"), effects = [Change])]`, and
/// optionally `undo` when performing it leaves a receipt, and
/// `phrases = [phrase(text!("timer {length}"), kind = "plain")]` for Search, where the named
/// values are arguments the phrase fixes.
#[proc_macro_derive(Intent, attributes(intent))]
pub fn intent(input: TokenStream) -> TokenStream {
    derived(input, intent::expand)
}

/// Declares a thing: a struct with a `reference` and a `title`, whose other fields are `Option`s
/// of the properties' types, `None` where this thing has none.
///
/// `#[thing(id = "clock.timer", title = text!("Timer"))]`, with `view_of = ["files.file"]` when its
/// references are those of another type, and on a field `#[thing(find)]` when
/// `find` returns it too, `#[thing(foreign)]` when someone else could have chosen its text, and
/// `#[thing(content)]` when it is returned only when asked for by name. A field
/// `foreign: Vec<&'static str>` is no property: it names those that are foreign in this thing only.
#[proc_macro_derive(Thing, attributes(thing))]
pub fn thing(input: TokenStream) -> TokenStream {
    derived(input, thing::expand)
}

/// Declares the named values a `compute` gives a list of.
#[proc_macro_derive(Record)]
pub fn record(input: TokenStream) -> TokenStream {
    derived(input, record::expand)
}

/// Makes an enum a closed list of words: `LowPower` is written `low-power`, and a variant with
/// `#[word("90")]` as that says.
#[proc_macro_derive(Words, attributes(word))]
pub fn words(input: TokenStream) -> TokenStream {
    derived(input, words::expand)
}

fn derived(input: TokenStream, expand: fn(DeriveInput) -> syn::Result<Tokens>) -> TokenStream {
    syn::parse(input)
        .and_then(expand)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

fn description(attributes: &[Attribute]) -> String {
    attributes
        .iter()
        .filter_map(|attribute| match &attribute.meta {
            Meta::NameValue(pair) if pair.path.is_ident("doc") => match &pair.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(text),
                    ..
                }) => Some(text.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn named_fields(input: &DeriveInput) -> syn::Result<Vec<(&Ident, String, &Field)>> {
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Some(fields.named.iter().collect::<Vec<_>>()),
            Fields::Unit => Some(Vec::new()),
            Fields::Unnamed(_) => None,
        },
        _ => None,
    }
    .ok_or_else(|| syn::Error::new_spanned(&input.ident, "only a struct with named fields"))?;
    Ok(fields
        .into_iter()
        .filter_map(|field| {
            let ident = field.ident.as_ref()?;
            let name = ident.to_string();
            Some((ident, name.trim_start_matches("r#").to_owned(), field))
        })
        .collect())
}

fn required<T>(value: Option<T>, at: &impl ToTokens, name: &str) -> syn::Result<T> {
    value.ok_or_else(|| syn::Error::new_spanned(at, format!("`{name} = …` is missing")))
}
