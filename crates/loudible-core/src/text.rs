//! Text rules: characters, matching and ordering of user-visible strings.

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::{decompose_canonical, is_combining_mark};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
use unicode_segmentation::UnicodeSegmentation;

/// The grapheme clusters of `text`, the characters a user sees.
pub fn characters(text: &str) -> impl Iterator<Item = &str> {
    text.graphemes(true)
}

/// Whether the first scalar of a grapheme is white space, like `Character.isWhitespace`.
fn is_whitespace(character: &str) -> bool {
    character.chars().next().is_some_and(char::is_whitespace)
}

/// Splits on runs of white space, omitting empty pieces.
pub fn split_whitespace(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = None;
    for (index, character) in text.grapheme_indices(true) {
        match (is_whitespace(character), start) {
            (true, Some(from)) => {
                pieces.push(&text[from..index]);
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(from) = start {
        pieces.push(&text[from..]);
    }
    pieces
}

/// Whether a grapheme is one ASCII letter or digit.
pub fn is_ascii_alphanumeric(character: &str) -> bool {
    character.len() == 1 && character.as_bytes()[0].is_ascii_alphanumeric()
}

/// Whether `character` is a control or format character (`CharacterSet.controlCharacters`).
pub fn is_control(character: char) -> bool {
    matches!(
        character.general_category(),
        GeneralCategory::Control | GeneralCategory::Format
    )
}

fn is_ignored_diacritic(character: char) -> bool {
    is_combining_mark(character)
        && matches!(
            character.general_category(),
            GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark
        )
}

/// Foundation doesn't match kana by its unvoiced form, or a Hangul syllable by its leading jamo.
fn stays_composed(character: char) -> bool {
    matches!(character, '\u{3040}'..='\u{30FF}' | '\u{AC00}'..='\u{D7A3}')
}

/// Text in the form that case- and diacritic-insensitive comparison uses.
fn folded(text: &str) -> String {
    let mut decomposed = String::with_capacity(text.len());
    for character in text.nfc() {
        if stays_composed(character) {
            decomposed.push(character);
        } else {
            decompose_canonical(character, |part| {
                if !is_ignored_diacritic(part) {
                    decomposed.push(part);
                }
            });
        }
    }
    caseless::default_case_fold_str(&decomposed)
}

/// `text.range(of: term, options: [.caseInsensitive, .diacriticInsensitive]) != nil`.
pub fn contains_folded(text: &str, term: &str) -> bool {
    folded(text).contains(&folded(term))
}

/// How `localizedStandardCompare` sorts punctuation: the common characters in collation order,
/// before digits, which come before letters.
const PUNCTUATION_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SortToken {
    Punctuation(usize),
    /// Significant digit count first, so longer numbers sort later.
    Number(usize, Vec<char>),
    Letter(char),
}

fn sort_tokens(text: &str) -> Vec<SortToken> {
    let characters: Vec<char> = folded(text).chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if character.is_ascii_digit() {
            let start = index;
            while characters.get(index).is_some_and(char::is_ascii_digit) {
                index += 1;
            }
            let digits: Vec<char> = characters[start..index]
                .iter()
                .copied()
                .skip_while(|digit| *digit == '0')
                .collect();
            tokens.push(SortToken::Number(digits.len(), digits));
            continue;
        }
        tokens.push(
            match PUNCTUATION_ORDER.chars().position(|c| c == character) {
                Some(rank) => SortToken::Punctuation(rank),
                None => SortToken::Letter(character),
            },
        );
        index += 1;
    }
    tokens
}

/// `a.localizedStandardCompare(b)` for English text: numbers compare by value; then ties go to
/// the shorter form, the unaccented form, and finally the lowercase form.
pub fn standard_compare(a: &str, b: &str) -> Ordering {
    let lowered = |text: &str| text.nfc().collect::<String>().to_lowercase();
    sort_tokens(a)
        .cmp(&sort_tokens(b))
        .then_with(|| folded(a).len().cmp(&folded(b).len()))
        .then_with(|| lowered(a).cmp(&lowered(b)))
        .then_with(|| b.cmp(a))
}
