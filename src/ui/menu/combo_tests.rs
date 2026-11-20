#[test]
fn combo_opacities_are_the_blob_values() {
    // combobox-border 50 % / open 90 % / disabled 20 %, combobox-text 95 % / disabled 40 %.
    assert_eq!(super::combo_alphas(true, false), (0.5, 0.95));
    assert_eq!(super::combo_alphas(true, true), (0.9, 0.95));
    assert_eq!(super::combo_alphas(false, false), (0.2, 0.4));
}
