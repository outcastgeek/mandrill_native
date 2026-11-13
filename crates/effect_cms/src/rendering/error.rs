//! The rendering owner's typed failure set.
//!
//! The registry's lookup failure, the SEO owner's failure and the consumer's page
//! validator failure keep distinct identities; none is flattened into a message.

use std::error::Error;
use std::fmt;

use crate::definitions::UnknownResourceKind;
use crate::seo::SEOError;

/// A list or detail page production failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderingError<E> {
    /// The requested kind is not registered for rendering.
    Definition(UnknownResourceKind),
    /// The SEO owner refused or could not produce the page value.
    Seo(SEOError<E>),
    /// The consumer's page validator refused the page.
    Consumer(E),
}

impl<E: fmt::Display> fmt::Display for RenderingError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition(error) => fmt::Display::fmt(error, formatter),
            Self::Seo(error) => fmt::Display::fmt(error, formatter),
            Self::Consumer(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl<E: Error + 'static> Error for RenderingError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Definition(error) => Some(error),
            Self::Seo(error) => Some(error),
            Self::Consumer(error) => Some(error),
        }
    }
}
