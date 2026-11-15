//! Text helpers shared by everything that matches what people type or what a control is called:
//! lowercase, accents folded, punctuation gone, so "Précédent" and "precedent" are the same word.

/// Folds the accented Latin letters French, German, Spanish, Italian and Portuguese use.
fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        '’' | '‘' | '`' => '\'',
        other => other,
    }
}

/// Lowercase, accents folded, runs of whitespace collapsed to one space, trimmed.
pub fn normalize(s: &str) -> String {
    let lower: String = s.to_lowercase().chars().map(fold).collect();
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// [`normalize`]d words, without punctuation.
pub fn words(s: &str) -> Vec<String> {
    normalize(s)
        .replace('\'', "' ")
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ';' | ':' | '"' | '«' | '»'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Words that only make a request polite, dropped before matching a command.
const FILLER: &[&str] =
    &["capri", "stp", "svp", "please", "pls", "steuplait", "merci", "thanks", "thx", "hey", "he", "eh", "ok", "okay"];

/// [`words`] without filler and "s'il te plaît": what is left is the request itself.
pub fn request(s: &str) -> Vec<String> {
    let mut joined = words(s).join(" ");
    for phrase in ["s' il te plait", "s' il vous plait"] {
        joined = joined.replace(phrase, " ");
    }
    joined.split_whitespace().filter(|w| !FILLER.contains(w)).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_accents_and_case() {
        assert_eq!(normalize("  Précédent   Titre "), "precedent titre");
        assert_eq!(normalize("Ajouter aux Titres likés"), "ajouter aux titres likes");
    }

    #[test]
    fn requests_lose_their_politeness() {
        assert_eq!(request("Capri, mets en pause s'il te plaît !"), ["mets", "en", "pause"]);
        assert_eq!(request("Next track please"), ["next", "track"]);
    }
}
