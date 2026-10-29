pub mod components;
pub mod origin;
mod parsing;
pub mod search_params;

#[cfg(test)]
mod file_url;

pub use origin::is_about_srcdoc;
pub use parsing::parse_url;

pub use origin::{
    WebOrigin, is_about_blank, is_opaque_origin, is_potentially_trustworthy_url,
    origin_ascii_serialization, origin_ascii_serialization_with_about_blank_inheritance,
    origin_unicode_serialization, parsed_same_origin, same_origin, tuple_origin_url,
};
