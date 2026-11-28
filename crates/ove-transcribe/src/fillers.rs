//! Filler words per language (add-transcript-editing D7). Hesitation sounds are filler candidates; words that are
//! fillers only in some contexts ("like", "you know") are flagged separately and never selected by default.

/// The word with surrounding punctuation removed and lower-cased.
pub fn normalise(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn list(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &["uh", "um", "uhm", "umm", "er", "erm", "ah", "hmm", "mm", "mhm"],
        "de" => &["äh", "ähm", "öh", "hm", "hmm"],
        "fr" => &["euh", "heu", "ben", "hum"],
        "es" => &["eh", "em", "este", "mmm"],
        "it" => &["ehm", "eh", "uhm", "mah"],
        "pt" => &["é", "hã", "hum", "ahn"],
        "nl" => &["eh", "ehm", "uhm", "hm"],
        _ => &["uh", "um", "hmm", "mm"],
    }
}

fn context_list(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &["like", "so", "well", "basically", "actually", "literally", "you know", "i mean"],
        "de" => &["also", "halt", "quasi", "sozusagen"],
        "fr" => &["genre", "bon", "voilà", "quoi"],
        "es" => &["pues", "bueno", "o sea"],
        _ => &[],
    }
}

/// True for a hesitation sound in `language` (ISO 639-1; other languages use a short common list).
pub fn is_filler(word: &str, language: &str) -> bool {
    let w = normalise(word);
    !w.is_empty() && list(language).contains(&w.as_str())
}

/// True for a word that is a filler only in some contexts: flagged, never selected by default.
pub fn is_context_filler(word: &str, language: &str) -> bool {
    let w = normalise(word);
    !w.is_empty() && context_list(language).contains(&w.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fillers_by_language() {
        assert!(is_filler("Um,", "en") && is_filler("uh...", "en") && is_filler("Ähm", "de"));
        assert!(!is_filler("umbrella", "en") && !is_filler("like", "en"));
        assert!(is_context_filler("Like,", "en") && !is_filler("Like", "en"));
        assert!(is_filler("hmm", "xx"));
    }
}
