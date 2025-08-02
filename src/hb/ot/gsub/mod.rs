//! OpenType GSUB lookups.

mod alternate;
mod ligature;
mod multiple;
mod reverse_chain;
mod single;

pub(crate) use ligature::apply_ligature_set;
