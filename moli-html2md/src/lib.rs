mod anchors;
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

#[cfg(test)]
extern crate self as moli_html2md;

#[cfg(test)]
mod copy_tests;
