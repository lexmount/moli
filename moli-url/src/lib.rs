pub mod components;
pub mod origin;
pub mod search_params;

#[cfg(test)]
mod file_url;
#[cfg(test)]
mod hierarchical_path;
#[cfg(test)]
mod path_encoding;

pub use origin::{
    WebOrigin, is_about_blank, is_opaque_origin, is_potentially_trustworthy_url,
    origin_ascii_serialization, origin_ascii_serialization_with_about_blank_inheritance,
    origin_unicode_serialization, parsed_same_origin, same_origin, tuple_origin_url,
};
