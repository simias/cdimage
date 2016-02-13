//! The actual generic interface to access the underlying image
//! format. This interface is exposed publicaly to make it possible
//! for the users to add their own custom formats and extensions if
//! they desire.

/// Abstract interface to an image formats
pub trait Backend {
    /// Return a string identifying the image format in a
    /// human-readable way. If the backend is daisy-chained it should
    /// mention the underlying image format as well.
    fn format_name(&self) -> String;
}
