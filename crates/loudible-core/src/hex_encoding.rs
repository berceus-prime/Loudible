//! Lowercase hexadecimal, as Audible's vouchers and FFmpeg's key options use.

pub fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes a hexadecimal string. Returns `None` for odd lengths or non-hex characters.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let characters = text.as_bytes();
    if !characters.len().is_multiple_of(2) {
        return None;
    }
    characters
        .chunks(2)
        .map(|pair| Some(nibble(pair[0])? << 4 | nibble(pair[1])?))
        .collect()
}

fn nibble(character: u8) -> Option<u8> {
    (character as char).to_digit(16).map(|digit| digit as u8)
}
