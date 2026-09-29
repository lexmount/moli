mod anchors;
mod content;
mod converter;
mod dom;
mod form;
mod html_table;
mod machine;
mod math;
mod mathml;
mod media;
mod options;
mod output;
mod table;
mod visibility;
mod writer;

pub use converter::{Converter, convert};
pub use dom::{Dom, NodeKind};
pub use options::Options;

#[doc(hidden)]
pub use media::{SrcsetCandidate, parse_srcset};

#[cfg(test)]
extern crate self as moli_html2md;

#[cfg(test)]
mod copy_tests;
