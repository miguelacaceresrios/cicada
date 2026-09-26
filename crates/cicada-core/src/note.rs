/// Convierte una nota en notación científica (`C4`, `F#2`, `Bb0`) a número MIDI.
/// `C4` = 60.
pub fn parse_note(s: &str) -> Option<u8> {
    let mut chars = s.chars();
    let class = match chars.next()? {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (accidental, octave) = match rest.strip_prefix('#') {
        Some(o) => (1, o),
        None => match rest.strip_prefix('b') {
            Some(o) => (-1, o),
            None => (0, rest),
        },
    };
    if octave.is_empty() || !octave.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let midi = (octave.parse::<i32>().ok()? + 1) * 12 + class + accidental;
    u8::try_from(midi).ok().filter(|m| *m <= 127)
}

/// Frecuencia en Hz de una nota MIDI, afinación La 440.
pub fn midi_to_hz(midi: u8) -> f32 {
    440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes() {
        assert_eq!(parse_note("C4"), Some(60));
        assert_eq!(parse_note("A4"), Some(69));
        assert_eq!(parse_note("C1"), Some(24));
        assert_eq!(parse_note("G0"), Some(19));
        assert_eq!(parse_note("F#2"), Some(42));
        assert_eq!(parse_note("Bb0"), Some(22));
        assert_eq!(parse_note("X"), None);
        assert_eq!(parse_note("C"), None);
        assert_eq!(parse_note("G9"), Some(127));
        assert_eq!(parse_note("A9"), None);
    }
}
