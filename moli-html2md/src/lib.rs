mod anchors;
mod content;
mod converter;
mod dom;
mod form;
mod html_fallback;
mod machine;
mod math;
mod mathml;
mod media;
mod options;
mod output;
mod structure;
mod table;
mod visibility;
mod writer;

pub use converter::{Converter, convert};
pub use dom::{Dom, NodeKind};
pub use options::Options;

pub use media::{SrcsetCandidate, parse_srcset, resource_url_attributes};


#[cfg(test)]
extern crate self as moli_html2md;

#[cfg(test)]
mod copy_tests;
