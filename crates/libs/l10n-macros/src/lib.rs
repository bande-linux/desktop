use std::path::Path;

use proc_macro::TokenStream;

#[proc_macro]
pub fn translations(input: TokenStream) -> TokenStream {
    let domain = input.to_string();
    let domain = domain.trim().trim_matches('"');
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let locales = Path::new(&root).join("locales");
    let mut languages = std::fs::read_dir(&locales)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join(format!("{domain}.ftl")).is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    languages.sort();
    let entries = languages
        .iter()
        .map(|language| {
            let file = locales.join(language).join(format!("{domain}.ftl"));
            format!(
                "({language:?}, include_str!({:?})),",
                file.display().to_string()
            )
        })
        .collect::<String>();
    format!("&[{entries}]")
        .parse()
        .unwrap_or_else(|_| TokenStream::new())
}
