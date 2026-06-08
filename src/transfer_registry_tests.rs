use super::*;

#[test]
fn test_format_card_id() {
    assert_eq!(format_card_id(0).unwrap(),    "CARD0000");
    assert_eq!(format_card_id(5).unwrap(),    "CARD0005");
    assert_eq!(format_card_id(10).unwrap(),   "CARD0010");
    assert_eq!(format_card_id(1234).unwrap(), "CARD1234");
    assert_eq!(format_card_id(9999).unwrap(), "CARD9999");
    assert!(format_card_id(10000).is_err());
    assert!(format_card_id(10001).is_err());
    assert!(format_card_id(u32::MAX).is_err());
}
