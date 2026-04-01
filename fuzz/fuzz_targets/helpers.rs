use read_fonts::{FileRef, FontRef, ReadError};

/// Makes fuzzing agnostic of collection and non-collection inputs
///
/// Picks a single font if data is a collection.
///
/// Borrowed from fuzzing in fontations
pub(crate) fn select_font(data: &[u8]) -> Result<FontRef<'_>, ReadError> {
    // Take the last byte as the collection index to let the fuzzer guide
    let i = data.last().copied().unwrap_or_default();
    match FileRef::new(data)? {
        FileRef::Collection(cr) => {
            let _ = cr.len();
            let _ = cr.is_empty();
            let _ = cr.iter().count();
            cr.get(i.into())
        }
        FileRef::Font(f) => Ok(f),
    }
}
