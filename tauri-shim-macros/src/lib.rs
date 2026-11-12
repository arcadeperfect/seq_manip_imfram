//! `#[tauri::command]` without Tauri: the function as written.
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn command(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
