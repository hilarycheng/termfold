#[test]
fn legacy_entry_preserves_capabilities_with_16_bit_colour_pairs() {
    let modern = include_bytes!("../terminfo/compiled/t/termfold-256color");
    let legacy = include_bytes!("../terminfo/compiled/legacy/t/termfold-256color");
    assert_eq!(&modern[..2], &0x021eu16.to_le_bytes());
    assert_eq!(&legacy[..2], &0x011au16.to_le_bytes());
    assert_eq!(&modern[2..12], &legacy[2..12]);
    let field = |offset| u16::from_le_bytes([modern[offset], modern[offset + 1]]) as usize;
    let numeric_start = (12 + field(2) + field(4) + 1) & !1;
    let numeric_count = field(6);
    assert_eq!(&modern[12..numeric_start], &legacy[12..numeric_start]);
    let numbers = |bytes: &[u8], width| {
        bytes[numeric_start..numeric_start + numeric_count * width]
            .chunks_exact(width)
            .map(|value| match width {
                2 => i16::from_le_bytes(value.try_into().unwrap()) as i32,
                4 => i32::from_le_bytes(value.try_into().unwrap()),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>()
    };
    let modern_numbers = numbers(modern, 4);
    let legacy_numbers = numbers(legacy, 2);
    assert_eq!(modern_numbers[13], 256); // max_colors
    assert_eq!(modern_numbers[14], 65536); // max_pairs
    assert_eq!(legacy_numbers[14], 32767);
    for (index, (&modern, &legacy)) in modern_numbers.iter().zip(&legacy_numbers).enumerate() {
        if index != 14 {
            assert_eq!(modern, legacy);
        }
    }
    assert_eq!(
        &modern[numeric_start + numeric_count * 4..],
        &legacy[numeric_start + numeric_count * 2..],
    );
}
