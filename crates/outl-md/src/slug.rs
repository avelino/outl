//! Slugify page names for stable filesystem paths.
//!
//! `[[Avelino]]` (the user-visible name, kept verbatim in `title::`)
//! maps to `pages/avelino.md` on disk. The rule:
//!
//! - Lowercase.
//! - Common Latin diacritics fold to their ASCII base (`á` → `a`,
//!   `ç` → `c`, `ñ` → `n`, ...).
//! - Any other non-`[a-z0-9]` character becomes `-`.
//! - Consecutive `-` collapse; leading/trailing `-` are stripped.
//! - Empty input maps to `untitled`.
//!
//! Future autocomplete (`[[`, `#`) will search by `title::`
//! property — not by slug — so users can have `[[Meu Projeto]]` and
//! `[[meu-projeto]]` resolve to the same file via the slug, but the
//! human-facing display always uses the title.

/// Fallback name when the input slugifies to nothing.
pub const UNTITLED_SLUG: &str = "untitled";

/// Convert a user-visible page name into a filesystem-safe slug.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = true; // suppress leading '-'
    for ch in name.chars() {
        if let Some(n) = fold_char(ch) {
            if n.is_ascii_alphanumeric() {
                out.push(n.to_ascii_lowercase());
                prev_dash = false;
            } else if !prev_dash {
                out.push('-');
                prev_dash = true;
            }
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return UNTITLED_SLUG.to_string();
    }
    out
}

/// Map a single character to its ASCII fold, if any. Returns:
/// - `Some(c)` for any alphanumeric or folded-letter result.
/// - `None` only when the character should become a `-` (handled by caller).
fn fold_char(ch: char) -> Option<char> {
    let folded = match ch {
        // Latin lowercase accents.
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'ē' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'ō' | 'ø' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'ū' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ý' | 'ÿ' => 'y',
        'œ' => 'o', // collapse ligatures
        'æ' => 'a',
        'ß' => 's',
        // Latin uppercase: lowercase first, then fold next round.
        'A'..='Z' => return Some(ch.to_ascii_lowercase()),
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' => 'a',
        'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => 'e',
        'Í' | 'Ì' | 'Î' | 'Ï' | 'Ī' => 'i',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'Ō' | 'Ø' => 'o',
        'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ū' => 'u',
        'Ç' => 'c',
        'Ñ' => 'n',
        'Ý' | 'Ÿ' => 'y',
        'Œ' => 'o',
        'Æ' => 'a',
        // Anything else passes through unchanged for the caller to decide.
        c if c.is_ascii_alphanumeric() => c,
        _ => return None,
    };
    Some(folded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_lowercase_passthrough() {
        assert_eq!(slugify("avelino"), "avelino");
        assert_eq!(slugify("Avelino"), "avelino");
    }

    #[test]
    fn spaces_become_single_dash() {
        assert_eq!(slugify("meu projeto"), "meu-projeto");
        assert_eq!(slugify("  meu   projeto  "), "meu-projeto");
    }

    #[test]
    fn accents_fold_to_ascii() {
        assert_eq!(slugify("ação"), "acao");
        assert_eq!(slugify("orçamento"), "orcamento");
        assert_eq!(slugify("está"), "esta");
        assert_eq!(slugify("Não"), "nao");
        assert_eq!(slugify("São Paulo"), "sao-paulo");
        assert_eq!(slugify("café"), "cafe");
    }

    #[test]
    fn punctuation_becomes_dash() {
        assert_eq!(slugify("meu/projeto"), "meu-projeto");
        assert_eq!(slugify("meu.projeto"), "meu-projeto");
        assert_eq!(slugify("meu_projeto"), "meu-projeto");
        assert_eq!(slugify("meu:projeto"), "meu-projeto");
    }

    #[test]
    fn iso_date_stays_intact() {
        assert_eq!(slugify("2026-05-24"), "2026-05-24");
    }

    #[test]
    fn empty_or_all_punct_becomes_untitled() {
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("   "), "untitled");
        assert_eq!(slugify("---"), "untitled");
        assert_eq!(slugify("///"), "untitled");
    }

    #[test]
    fn emoji_collapses_to_dashes() {
        // Emojis aren't ASCII alphanumerics → become dashes → trimmed.
        assert_eq!(slugify("ship 🚀"), "ship");
        assert_eq!(slugify("🚀launch🚀"), "launch");
    }

    #[test]
    fn cjk_collapses_too() {
        // No fold for CJK — they become dashes. Document the behavior.
        assert_eq!(slugify("こんにちは world"), "world");
    }

    #[test]
    fn ligatures_fold() {
        assert_eq!(slugify("œuvre"), "ouvre"); // œ → o
        assert_eq!(slugify("Æsop"), "asop");
        // German ß folds to a single 's' (we don't expand to "ss" — that
        // would require multi-char output and adds little for slug clarity).
        assert_eq!(slugify("straße"), "strase");
    }

    #[test]
    fn numbers_preserved() {
        assert_eq!(slugify("Q4 2026 plan"), "q4-2026-plan");
    }

    #[test]
    fn slug_is_idempotent() {
        let s = slugify("Meu Projeto X");
        assert_eq!(slugify(&s), s);
    }
}
