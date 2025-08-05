//! OpenType GSUB lookups.

mod alternate;
mod ligature;
mod multiple;
mod reverse_chain;
mod single;

pub(crate) use ligature::apply_lig_subst1;
