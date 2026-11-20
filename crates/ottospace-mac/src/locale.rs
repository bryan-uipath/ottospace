//! The user's language and region as macOS's settings name them, which the core has no way to know.

use objc2_core_foundation::CFLocale;

/// The current locale's identifier, e.g. `en_US` or `de_CH@rg=chzzzz`; `None` if it cannot be read.
pub fn identifier() -> Option<String> {
    CFLocale::current()?.identifier().map(|id| id.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_locale_reads_as_language_and_region() {
        let id = super::identifier().expect("a locale");
        assert!(id.chars().next().is_some_and(|c| c.is_ascii_lowercase()), "{id}");
    }
}
