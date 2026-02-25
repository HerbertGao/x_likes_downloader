use super::*;

#[test]
fn test_parse_filename_standard() {
    let (user, id) =
        FileOrganizer::parse_filename("user_abc_1234567890123456789_photo.jpg").unwrap();
    assert_eq!(user, "user_abc");
    assert_eq!(id, "1234567890123456789");
}

#[test]
fn test_parse_filename_cdn_short_number() {
    // CDN filename contains short number — must not be mistaken for tweet ID
    let (user, id) =
        FileOrganizer::parse_filename("userA888_2021584971287195730_8_xAbCdEfGh.mp4").unwrap();
    assert_eq!(user, "userA888");
    assert_eq!(id, "2021584971287195730");
}

#[test]
fn test_parse_filename_trailing_underscore() {
    // Trailing underscore produces empty token — must be ignored
    let (user, id) =
        FileOrganizer::parse_filename("userB579_1993218560311935011_aBcDeFgHiJ_.mp4").unwrap();
    assert_eq!(user, "userB579");
    assert_eq!(id, "1993218560311935011");
}

#[test]
fn test_parse_filename_trailing_underscore_with_inner_underscore() {
    let (user, id) =
        FileOrganizer::parse_filename("userC_2015339771552502225_G_xYzAbCdEfG_.jpg").unwrap();
    assert_eq!(user, "userC");
    assert_eq!(id, "2015339771552502225");
}

#[test]
fn test_parse_filename_pure_numeric_username() {
    // 15-digit numeric username must not be mistaken for tweet ID
    let (user, id) =
        FileOrganizer::parse_filename("123456789012345_1993218560311935011_originalname.mp4")
            .unwrap();
    assert_eq!(user, "123456789012345");
    assert_eq!(id, "1993218560311935011");
}

#[test]
fn test_parse_filename_username_with_underscore() {
    let (user, id) =
        FileOrganizer::parse_filename("user_name_1234567890123456789_photo.jpg").unwrap();
    assert_eq!(user, "user_name");
    assert_eq!(id, "1234567890123456789");
}

#[test]
fn test_parse_filename_no_valid_tweet_id() {
    // No token >= 16 digits — should fail
    assert!(FileOrganizer::parse_filename("user_123_photo.jpg").is_err());
}

#[test]
fn test_parse_filename_too_few_tokens() {
    assert!(FileOrganizer::parse_filename("user_photo.jpg").is_err());
}
