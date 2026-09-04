//! OpenType GSUB lookups.

mod alternate;
mod ligature;
pub(crate) use ligature::collect_seconds;
mod multiple;
mod reverse_chain;
mod single;
