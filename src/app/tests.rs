use std::net::SocketAddr;

use super::*;
use crate::message::{DeviceCert, IdentityAnnounce, VersionAnnounce};
use iroh::{EndpointAddr, SecretKey, TransportAddr};

const SELF_ID: [u8; 32] = [9; 32];
const PEER_ID: [u8; 32] = [7; 32];
const PEER_ID_2: [u8; 32] = [8; 32];
const PEER_ID_3: [u8; 32] = [6; 32];

fn app() -> AppState {
    AppState::new(SELF_ID, vec!["general".to_string()], "general")
}

fn multi_channel_app() -> AppState {
    AppState::new(
        SELF_ID,
        vec!["general".to_string(), "random".to_string()],
        "random",
    )
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Unwraps a `TranscriptLine::Chat`'s inner message, panicking with a
/// helpful message if the line is actually a `TranscriptLine::System`.
fn as_chat(line: &TranscriptLine) -> &ChatMessage {
    match line {
        TranscriptLine::Chat(message) => message,
        TranscriptLine::System(text) => {
            panic!("expected a chat message, got system line: {text:?}")
        }
    }
}

/// Unwraps a `TranscriptLine::System`'s inner text, panicking if the
/// line is actually a `TranscriptLine::Chat`.
fn as_system(line: &TranscriptLine) -> &str {
    match line {
        TranscriptLine::System(text) => text,
        TranscriptLine::Chat(_) => panic!("expected a system line, got a chat message"),
    }
}

/// Types `text` character-by-character then presses `Enter`, returning
/// whatever action (if any) that produced -- shared by the `/alias`
/// tests below, which all follow this shape.
fn submit(app: &mut AppState, text: &str) -> Option<InputAction> {
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter))
}

#[test]
fn typing_composes_input() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    assert_eq!(app.active().input, "hi");
}

#[test]
fn backspace_edits_input() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.active().input, "h");
}

#[test]
fn paste_text_inserts_the_whole_string_at_once() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.paste_text("i there");
    assert_eq!(app.active().input, "hi there");
    assert_eq!(app.active().cursor, "hi there".chars().count());
}

#[test]
fn paste_text_inserts_embedded_newlines_literally() {
    // Without bracketed paste, a multi-line paste's embedded `Enter`
    // keystrokes would submit partway through -- `paste_text` is the
    // atomic alternative, so it can safely turn a pasted newline into
    // a real line break instead of flattening it to a space.
    let mut app = app();
    app.paste_text("line one\nline two\r\nline three");
    assert_eq!(app.active().input, "line one\nline two\nline three");
}

#[test]
fn paste_text_does_not_submit_even_when_it_contains_a_slash_command() {
    let mut app = app();
    let before = app.active().messages.len();
    app.paste_text("/join sneaky\nrest of paste");
    assert_eq!(app.active().messages.len(), before, "paste must not submit");
    assert_eq!(app.active().input, "/join sneaky\nrest of paste");
}

#[test]
fn alt_enter_inserts_a_newline_instead_of_submitting() {
    let mut app = app();
    let before = app.active().messages.len();
    app.handle_key(key(KeyCode::Char('h')));
    let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    app.handle_key(key(KeyCode::Char('i')));
    assert!(action.is_none());
    assert_eq!(app.active().input, "h\ni");
    assert_eq!(app.active().messages.len(), before, "must not submit");
}

#[test]
fn ctrl_j_inserts_a_newline_instead_of_submitting() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    let action = app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert!(action.is_none());
    assert_eq!(app.active().input, "h\n");
}

#[test]
fn plain_enter_still_submits_a_multiline_message() {
    let mut app = app();
    for c in "line one".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    for c in "line two".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let sent = app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.active().input, "");
    match sent.expect("enter with non-empty input returns an action") {
        InputAction::Send(_, message) => assert_eq!(message.text, "line one\nline two"),
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn home_and_end_move_within_the_current_line_only() {
    let mut app = app();
    app.paste_text("first\nsecond"); // cursor ends on the second line
    app.handle_key(key(KeyCode::Home));
    assert_eq!(
        app.active().cursor,
        "first\n".chars().count(),
        "Home lands at the start of the current line, not the whole buffer"
    );
    app.handle_key(key(KeyCode::End));
    assert_eq!(
        app.active().cursor,
        app.active().input.chars().count(),
        "End lands at the end of the current line"
    );
}

#[test]
fn ctrl_k_keeps_a_multiline_draft_unchanged_while_opening_the_picker() {
    let mut app = app();
    app.paste_text("first\nsecond");
    app.handle_key(key(KeyCode::Home)); // start of "second"
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(
        app.active().input,
        "first\nsecond",
        "opening the picker must not change the draft"
    );
    assert!(app.command_picker.is_some());
}

#[test]
fn ctrl_u_deletes_only_to_the_start_of_the_current_line() {
    let mut app = app();
    app.paste_text("first\nsecond");
    app.handle_key(key(KeyCode::End)); // already at the end of "second"
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(
        app.active().input,
        "first\n",
        "Ctrl+U must not touch the previous line"
    );
}

#[test]
fn enter_sends_and_clears_input() {
    let mut app = app();
    let before = app.active().messages.len();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    let sent = app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.active().input, "");
    assert_eq!(app.active().messages.len(), before + 1);
    let last = as_chat(app.active().messages.back().unwrap());
    assert_eq!(last.text, "hi");
    assert_eq!(last.sender, SELF_ID);
    match sent.expect("enter with non-empty input returns an action") {
        InputAction::Send(channel, message) => {
            assert_eq!(channel, "general");
            assert_eq!(message.text, "hi");
            assert_eq!(message.sender, SELF_ID);
        }
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn empty_enter_does_not_send() {
    let mut app = app();
    let before = app.active().messages.len();
    let sent = app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.active().messages.len(), before);
    assert!(sent.is_none());
}

#[test]
fn esc_quits() {
    let mut app = app();
    app.handle_key(key(KeyCode::Esc));
    assert!(app.should_quit);
}
#[test]
fn esc_clears_an_active_search_before_other_normal_escape_actions() {
    let mut app = app();
    let matching = compose_message(PEER_ID, "find me", None);
    let matching_id = matching.id;
    let non_matching = compose_message(PEER_ID, "leave me", None);
    let non_matching_id = non_matching.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), matching));
    app.handle_net_event(NetEvent::Received("general".to_string(), non_matching));
    app.stage_attachment("general", staged("map.png"));
    submit(&mut app, "/search find");
    assert_eq!(app.active().visible_chat_ids(), vec![matching_id]);

    app.handle_key(key(KeyCode::Esc));

    assert!(app.active().search.is_none());
    assert_eq!(
        app.active().visible_chat_ids(),
        vec![matching_id, non_matching_id],
        "clearing the filter restores the unfiltered transcript"
    );
    assert_eq!(app.active().staged, vec![staged("map.png")]);
    assert!(!app.should_quit);
    assert_eq!(
        as_system(app.active().messages.back().unwrap()),
        "search cleared"
    );
}

#[test]
fn ctrl_c_quits() {
    let mut app = app();
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
}

#[test]
fn display_name_maps_self_and_falls_back_to_hex_for_others() {
    let app = app();
    assert_eq!(app.display_name(&SELF_ID), "you");
    assert_eq!(app.display_name(&PEER_ID), hex_prefix(&PEER_ID));
}

#[test]
fn display_name_uses_petname_when_one_is_set() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    assert_eq!(app.display_name(&PEER_ID), "Alice");
}

#[test]
fn display_name_uses_nickname_when_no_petname_is_set() {
    let mut app = app();
    let result = app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "Alice".to_string(),
        },
    ));
    assert!(result.is_none());
    assert_eq!(
        app.display_name(&PEER_ID),
        format!("Alice ({})", hex_prefix(&PEER_ID)),
        "a broadcast nickname must be shown alongside the hex id, since it's spoofable"
    );
}

#[test]
fn display_name_prefers_petname_over_nickname() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "Alice".to_string(),
        },
    ));
    assert_eq!(
        app.display_name(&PEER_ID),
        "Bob",
        "a pinned local petname must win over a peer's own broadcast nickname"
    );
}

#[test]
fn identity_event_overwrites_previous_nickname_for_the_same_sender() {
    let mut app = app();
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "Alice".to_string(),
        },
    ));
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "Alicia".to_string(),
        },
    ));
    assert_eq!(
        app.display_name(&PEER_ID),
        format!("Alicia ({})", hex_prefix(&PEER_ID)),
        "display_name must reflect the last-seen nickname, not the first"
    );
}

fn device_cert(user_key: &SecretKey, device_id: [u8; 32]) -> DeviceCert {
    DeviceCert::new(device_id, user_key, 1_000)
}

#[test]
fn canonical_id_is_unchanged_when_no_cert_is_known() {
    let app = app();
    assert_eq!(app.canonical_id(&PEER_ID), PEER_ID);
}

#[test]
fn a_peer_with_no_certificate_displays_exactly_as_before() {
    let app = app();
    assert_eq!(app.display_name(&PEER_ID), hex_prefix(&PEER_ID));
}

#[test]
fn device_event_makes_canonical_id_resolve_to_the_user_id() {
    let mut app = app();
    let user_key = SecretKey::generate();
    let user_id = *user_key.public().as_bytes();
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    assert_eq!(app.canonical_id(&PEER_ID), user_id);
}

#[test]
fn two_certified_devices_share_a_display_name() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));
    app.load_contacts(HashMap::from([(
        app.canonical_id(&PEER_ID),
        "Jurgen".to_string(),
    )]));

    assert_eq!(app.display_name(&PEER_ID), "Jurgen");
    assert_eq!(app.display_name(&PEER_ID_2), "Jurgen");
}

#[test]
fn unique_peer_ids_collapses_two_certified_devices_into_one() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));

    let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_2]);

    assert_eq!(
        unique,
        vec![PEER_ID],
        "two devices certified under the same user key must collapse to one"
    );
}

#[test]
fn unique_peer_ids_keeps_uncertified_peers_distinct() {
    let app = app();
    let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_2]);
    assert_eq!(
        unique,
        vec![PEER_ID, PEER_ID_2],
        "peers with no known certificate must never be merged"
    );
}

#[test]
fn unique_peer_ids_preserves_original_order_including_mixed_groups() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));

    // PEER_ID and PEER_ID_2 are the same person's two devices;
    // PEER_ID_3 has no certificate at all.
    let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_3, PEER_ID_2]);

    assert_eq!(unique, vec![PEER_ID, PEER_ID_3]);
}

#[test]
fn display_name_shows_you_for_a_peer_certified_under_our_own_user_key() {
    let mut app = app();
    let user_key = SecretKey::generate();
    let user_id = *user_key.public().as_bytes();
    app.load_self_user_id(user_id);
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));

    assert_eq!(
        app.display_name(&PEER_ID),
        "you",
        "a paired sibling device is us, not a stranger"
    );
}

#[test]
fn display_name_you_check_for_a_sibling_device_overrides_any_petname() {
    let mut app = app();
    let user_key = SecretKey::generate();
    let user_id = *user_key.public().as_bytes();
    app.load_self_user_id(user_id);
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.load_contacts(HashMap::from([(user_id, "My Phone".to_string())]));

    assert_eq!(
        app.display_name(&PEER_ID),
        "you",
        "recognizing our own identity must win over a petname, mirroring the self_id check above it"
    );
}

#[test]
fn unique_peer_ids_excludes_a_sibling_device_entirely() {
    let mut app = app();
    let user_key = SecretKey::generate();
    let user_id = *user_key.public().as_bytes();
    app.load_self_user_id(user_id);
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));

    let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_2]);

    assert_eq!(
        unique,
        vec![PEER_ID_2],
        "our own sibling device is already shown as the sidebar's separate 'you' row, \
             so it must not also appear as a peer"
    );
}

#[test]
fn unique_peer_ids_is_empty_when_the_only_peer_is_a_sibling_device() {
    let mut app = app();
    let user_key = SecretKey::generate();
    let user_id = *user_key.public().as_bytes();
    app.load_self_user_id(user_id);
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));

    assert!(app.unique_peer_ids(&[PEER_ID]).is_empty());
}

#[test]
fn peer_presence_keeps_a_left_peer_with_its_last_seen_time() {
    let mut app = app();
    app.record_peer_seen("general", PEER_ID, 100);
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

    assert_eq!(
        app.peer_presence(),
        vec![PeerPresence {
            id: PEER_ID,
            last_seen_unix_ms: None,
        }]
    );

    app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));

    assert_eq!(
        app.peer_presence(),
        vec![PeerPresence {
            id: PEER_ID,
            last_seen_unix_ms: Some(100),
        }]
    );
}

#[test]
fn hovered_peer_reports_alias_nickname_presence_and_version() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    assert!(app.set_peer_hover(Some(PEER_ID)));
    assert!(!app.set_peer_hover(Some(PEER_ID)));
    assert_eq!(
        app.hovered_peer(),
        Some(PeerDetails {
            id: PEER_ID,
            alias: None,
            nickname: None,
            last_seen_unix_ms: None,
            version: None,
        })
    );

    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "ally".to_string(),
        },
    ));
    app.handle_net_event(NetEvent::Version(
        "general".to_string(),
        VersionAnnounce {
            sender: PEER_ID,
            version: "1.2.3".to_string(),
            git_hash: "abc1234".to_string(),
        },
    ));
    app.record_peer_seen("general", PEER_ID, 100);
    app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));

    assert_eq!(
        app.hovered_peer(),
        Some(PeerDetails {
            id: PEER_ID,
            alias: Some("Alice".to_string()),
            nickname: Some("ally".to_string()),
            last_seen_unix_ms: Some(100),
            version: Some("1.2.3".to_string()),
        })
    );
}

#[test]
fn hovered_peer_is_hidden_for_an_unlisted_peer_and_behind_overlays() {
    let mut app = app();
    app.set_peer_hover(Some(PEER_ID));
    assert_eq!(app.hovered_peer(), None, "not in the active channel");

    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    assert!(app.hovered_peer().is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(app.hovered_peer(), None, "the command picker owns input");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    submit(&mut app, "/hints");
    assert!(app.peer_hover.is_none(), "the peers panel was resized");
    assert!(
        app.set_peer_hover(Some(PEER_ID)),
        "peer hover doesn't need the hints panel"
    );

    submit(&mut app, "/mouse");
    assert!(app.peer_hover.is_none());
    assert!(!app.set_peer_hover(Some(PEER_ID)));
}

#[test]
fn peer_presence_loads_last_seen_per_channel() {
    let mut app = multi_channel_app();
    app.load_last_seen(HashMap::from([
        ("general".to_string(), HashMap::from([(PEER_ID, 100)])),
        ("random".to_string(), HashMap::from([(PEER_ID_2, 200)])),
    ]));

    assert_eq!(
        app.peer_presence(),
        vec![PeerPresence {
            id: PEER_ID_2,
            last_seen_unix_ms: Some(200),
        }]
    );
    app.active = 0;
    assert_eq!(
        app.peer_presence(),
        vec![PeerPresence {
            id: PEER_ID,
            last_seen_unix_ms: Some(100),
        }]
    );
}

#[test]
fn peer_presence_prefers_live_presence_and_deduplicates_certified_devices() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));
    app.record_peer_seen("general", PEER_ID, 100);
    app.record_peer_seen("general", PEER_ID_2, 200);
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

    assert_eq!(
        app.peer_presence(),
        vec![PeerPresence {
            id: PEER_ID,
            last_seen_unix_ms: None,
        }]
    );

    app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));
    assert_eq!(
        app.peer_presence(),
        vec![PeerPresence {
            id: PEER_ID_2,
            last_seen_unix_ms: Some(200),
        }]
    );
}

#[test]
fn device_event_migrates_a_nickname_seen_before_the_certificate() {
    let mut app = app();
    let user_key = SecretKey::generate();
    let user_id = *user_key.public().as_bytes();
    // The nickname arrives first -- gossip has no delivery ordering
    // guarantee, so this must not get orphaned once the cert arrives.
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "alice".to_string(),
        },
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));

    assert_eq!(
        app.display_name(&PEER_ID),
        format!("alice ({})", hex_prefix(&PEER_ID))
    );
    assert_eq!(
        app.nicknames.get(&user_id).map(String::as_str),
        Some("alice"),
        "the nickname must be re-keyed onto the canonical user id"
    );
}

#[test]
fn alias_assigns_a_petname_for_a_currently_online_peer() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

    let full_id = hex_id(&PEER_ID);
    let action = submit(&mut app, &format!("/alias {} Alice", &full_id[..8]));

    match action.expect("a successful /alias returns an action") {
        InputAction::Alias(id, name) => {
            assert_eq!(id, PEER_ID);
            assert_eq!(name, "Alice");
        }
        _ => panic!("expected InputAction::Alias"),
    }
    assert_eq!(app.display_name(&PEER_ID), "Alice");
}

#[test]
fn alias_matches_a_peer_known_only_from_a_past_message() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hi", None),
    ));

    let full_id = hex_id(&PEER_ID);
    let action = submit(&mut app, &format!("/alias {} Bob", &full_id[..8]));

    assert!(
        matches!(action, Some(InputAction::Alias(id, _)) if id == PEER_ID),
        "a peer seen only as a message sender must still be aliasable"
    );
}

#[test]
fn alias_reports_an_error_for_an_unknown_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/alias ffffffff Carol");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no known peer"), "got: {last}");
}

#[test]
fn alias_reports_an_error_for_an_ambiguous_prefix() {
    let mut app = app();
    let mut peer_a = [0xAB; 32];
    let mut peer_b = [0xAB; 32];
    peer_a[4] = 0x01;
    peer_b[4] = 0x02;
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), peer_a));
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), peer_b));

    // Both ids share this 4-byte (8 hex char) prefix.
    let full_id = hex_id(&peer_a);
    let action = submit(&mut app, &format!("/alias {} Dave", &full_id[..8]));

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("multiple peers"), "got: {last}");
}

#[test]
fn alias_rejects_a_non_hex_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/alias not-hex Eve");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("invalid hex prefix"), "got: {last}");
}

#[test]
fn alias_usage_error_when_name_is_missing() {
    let mut app = app();
    let action = submit(&mut app, "/alias ffffffff");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"));
}

#[test]
fn alias_applies_to_a_second_device_once_its_certificate_is_known() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));

    let full_id = hex_id(&PEER_ID);
    submit(&mut app, &format!("/alias {} Jurgen", &full_id[..8]));

    assert_eq!(app.display_name(&PEER_ID), "Jurgen");
    assert_eq!(
        app.display_name(&PEER_ID_2),
        "Jurgen",
        "aliasing one of a person's devices must apply to the other once both certs are known"
    );
}

#[test]
fn forgetting_a_device_stops_it_from_being_grouped_under_its_canonical_id() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    assert_ne!(
        app.canonical_id(&PEER_ID),
        PEER_ID,
        "sanity check: cert took effect"
    );

    submit(
        &mut app,
        &format!("/forget-device {}", &hex_id(&PEER_ID)[..8]),
    );

    assert_eq!(
        app.canonical_id(&PEER_ID),
        PEER_ID,
        "a forgotten device must resolve to itself, ignoring any known certificate"
    );
}

#[test]
fn forget_device_returns_the_action_with_the_resolved_id() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    let full_id = hex_id(&PEER_ID);

    let action = submit(&mut app, &format!("/forget-device {}", &full_id[..8]));

    match action.expect("a successful /forget-device returns an action") {
        InputAction::ForgetDevice(id) => assert_eq!(id, PEER_ID),
        _ => panic!("expected InputAction::ForgetDevice"),
    }
}

#[test]
fn forget_device_reports_an_error_for_an_unknown_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/forget-device ffffffff");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no known peer"), "got: {last}");
}

#[test]
fn nick_command_returns_action_and_pushes_system_notice() {
    let mut app = app();
    let action = submit(&mut app, "/nick Alice");
    match action.expect("a successful /nick returns an action") {
        InputAction::Nick(name) => assert_eq!(name, "Alice"),
        _ => panic!("expected InputAction::Nick"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("Alice"), "got: {last}");
}

#[test]
fn nick_usage_error_when_name_is_missing() {
    let mut app = app();
    let action = submit(&mut app, "/nick");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"));
}

#[test]
fn pair_with_no_argument_returns_the_pair_action() {
    let mut app = app();
    let action = submit(&mut app, "/pair");
    assert!(matches!(action, Some(InputAction::Pair)));
}

fn sample_pairing_ticket() -> String {
    let addr = EndpointAddr::from_parts(
        SecretKey::generate().public(),
        [TransportAddr::Ip(SocketAddr::from(([127, 0, 0, 1], 4242)))],
    );
    PairingTicket {
        addr,
        secret: rand::random(),
    }
    .encode_string()
}

#[test]
fn pair_with_a_valid_ticket_warns_instead_of_acting_immediately() {
    let mut app = app();
    let ticket = sample_pairing_ticket();
    let action = submit(&mut app, &format!("/pair {ticket}"));
    assert!(action.is_none(), "must not act until explicitly confirmed");
    assert!(!app.should_quit);
    assert!(app.restart_with_ticket.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("ERASE"), "got: {last}");
    assert!(
        last.contains("1 channel(s)"),
        "should name the current channel count, got: {last}"
    );
}

#[test]
fn pair_with_a_malformed_ticket_reports_an_error_and_never_arms_confirmation() {
    let mut app = app();
    let action = submit(&mut app, "/pair not-a-real-ticket");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("invalid pairing ticket"), "got: {last}");

    // Confirmation must not have been armed -- a normal message typed
    // right after still sends normally instead of being swallowed as
    // a confirm/cancel keypress.
    let sent = submit(&mut app, "hello");
    assert!(matches!(sent, Some(InputAction::Send(..))));
}

#[test]
fn confirming_a_pending_pair_redeem_arms_restart_and_quits() {
    let mut app = app();
    let ticket = sample_pairing_ticket();
    submit(&mut app, &format!("/pair {ticket}"));

    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.restart_with_ticket, Some(ticket));
    assert!(app.should_quit);
}

#[test]
fn canceling_a_pending_pair_redeem_clears_it_without_quitting() {
    let mut app = app();
    let ticket = sample_pairing_ticket();
    submit(&mut app, &format!("/pair {ticket}"));

    app.handle_key(key(KeyCode::Esc));

    assert!(!app.should_quit);
    assert!(app.restart_with_ticket.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "pairing cancelled");

    // No longer armed -- typing now reaches the input box normally
    // instead of being swallowed as a confirm/cancel keypress.
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.active().input, "h");
}

#[test]
fn unrelated_keys_are_ignored_while_a_pair_redeem_is_pending() {
    let mut app = app();
    let ticket = sample_pairing_ticket();
    submit(&mut app, &format!("/pair {ticket}"));

    app.handle_key(key(KeyCode::Char('y')));

    assert!(!app.should_quit);
    assert!(app.restart_with_ticket.is_none());
    assert_eq!(
        app.active().input,
        "",
        "a stray keystroke must not leak into the input box while armed"
    );
}

#[test]
fn newer_peer_version_pushes_a_notice_once() {
    let mut app = app();
    let announce = VersionAnnounce {
        sender: PEER_ID,
        version: "999.0.0".to_string(),
        git_hash: "abc1234".to_string(),
    };
    app.handle_net_event(NetEvent::Version("general".to_string(), announce.clone()));
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("999.0.0"), "got: {last}");

    // Re-announcing the exact same version (e.g. on a reconnect) must
    // not repeat the notice.
    let before = app.active().messages.len();
    app.handle_net_event(NetEvent::Version("general".to_string(), announce));
    assert_eq!(
        app.active().messages.len(),
        before,
        "must not repeat the notice for an unchanged re-announce"
    );
}

#[test]
fn older_peer_version_does_not_notice_but_is_recorded_for_peer_details() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    let before = app.active().messages.len();

    app.handle_net_event(NetEvent::Version(
        "general".to_string(),
        VersionAnnounce {
            sender: PEER_ID,
            version: "0.0.0".to_string(),
            git_hash: "abc1234".to_string(),
        },
    ));
    assert_eq!(
        app.active().messages.len(),
        before,
        "an older version must not push a notice"
    );

    submit(&mut app, "/who");
    assert_eq!(
        app.viewing_peers
            .as_ref()
            .unwrap()
            .peers
            .first()
            .unwrap()
            .version
            .as_deref(),
        Some("0.0.0")
    );
}

#[test]
fn version_command_reports_our_own_build() {
    let mut app = app();
    let action = submit(&mut app, "/version");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains(version::VERSION), "got: {last}");
}

#[test]
fn version_command_also_reports_a_known_newer_peer_version() {
    let mut app = app();
    app.handle_net_event(NetEvent::Version(
        "general".to_string(),
        VersionAnnounce {
            sender: PEER_ID,
            version: "999.0.0".to_string(),
            git_hash: "abc1234".to_string(),
        },
    ));
    submit(&mut app, "/version");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("999.0.0"), "got: {last}");
}

#[test]
fn left_right_move_cursor_without_editing() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    assert_eq!(app.active().cursor, 2);
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.active().cursor, 1);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.active().cursor, 2);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.active().cursor, 2);
    assert_eq!(app.active().input, "hi");
}

#[test]
fn insert_in_the_middle_of_input() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Char('X')));
    assert_eq!(app.active().input, "hXi");
    assert_eq!(app.active().cursor, 2);
}

#[test]
fn home_and_end_jump_cursor() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    app.handle_key(key(KeyCode::Home));
    assert_eq!(app.active().cursor, 0);
    app.handle_key(key(KeyCode::End));
    assert_eq!(app.active().cursor, 2);
}

#[test]
fn ctrl_a_and_ctrl_e_jump_cursor() {
    let mut app = app();
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(app.active().cursor, 0);
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert_eq!(app.active().cursor, 2);
}

#[test]
fn delete_key_removes_char_at_cursor() {
    let mut app = app();
    for c in ['h', 'i', '!'] {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Home));
    app.handle_key(key(KeyCode::Delete));
    assert_eq!(app.active().input, "i!");
    assert_eq!(app.active().cursor, 0);
}

#[test]
fn ctrl_u_deletes_to_start() {
    let mut app = app();
    for c in ['h', 'i', '!'] {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Left));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.active().input, "!");
    assert_eq!(app.active().cursor, 0);
}

#[test]
fn ctrl_k_opens_the_command_picker_without_changing_the_draft() {
    let mut app = app();
    for c in ['h', 'i', '!'] {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert!(app.command_picker.is_some());
    assert_eq!(app.active().input, "hi!");
    assert!(!app.is_composing());
}
#[test]
fn command_catalog_exposes_command_shortcuts() {
    let shortcuts: Vec<_> = command_catalog()
        .iter()
        .filter_map(|command| command.shortcut.map(|shortcut| (command.syntax, shortcut)))
        .collect();
    assert_eq!(
        shortcuts,
        vec![
            ("/paste", "Ctrl+V"),
            ("/reply [text]", "Ctrl+R"),
            ("/thread", "Ctrl+T"),
        ]
    );
}

#[test]
fn command_picker_prioritizes_syntax_and_alias_matches_over_descriptions() {
    let mut app = app();
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    type_text(&mut app, "file");
    assert_eq!(
        app.command_picker_matches().first(),
        Some(&21),
        "/files must precede commands that only mention files in their descriptions"
    );

    for _ in 0..4 {
        app.handle_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "/s");
    assert!(app.command_picker_matches().contains(&23));
}

#[test]
fn command_picker_enter_executes_the_selected_command_and_preserves_the_draft() {
    let mut app = app();
    type_text(&mut app, "old draft");
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    type_text(&mut app, "invite");

    let action = app.handle_key(key(KeyCode::Enter));
    assert!(matches!(action, Some(InputAction::Invite(channel)) if channel == "general"));
    assert!(app.command_picker.is_none());
    assert_eq!(app.active().input, "old draft");
    assert_eq!(app.active().cursor, 9);
}

#[test]
fn command_picker_executes_files_and_safe_optional_commands_without_changing_the_draft() {
    for query in ["files", "send"] {
        let mut app = app();
        type_text(&mut app, "old draft");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        type_text(&mut app, query);

        let action = app.handle_key(key(KeyCode::Enter));

        match action.expect("the selected command returns its action") {
            InputAction::ReadFiles { channel, .. } if query == "files" => {
                assert_eq!(channel, "general");
                assert!(app.viewing_files.is_some());
            }
            InputAction::ReadFolder { .. } if query == "send" => {
                assert!(app.picking_file.is_some());
            }
            _ => panic!("expected the action for /{query}"),
        }
        assert!(app.command_picker.is_none());
        assert_eq!(app.active().input, "old draft");
        assert_eq!(app.active().cursor, 9);
    }
}

#[test]
fn command_picker_inserts_required_commands_and_replaces_the_draft() {
    let mut app = app();
    type_text(&mut app, "old draft");
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    type_text(&mut app, "search");

    let action = app.handle_key(key(KeyCode::Enter));

    assert!(action.is_none());
    assert!(app.command_picker.is_none());
    assert_eq!(app.active().input, "/search ");
    assert_eq!(app.active().cursor, "/search ".chars().count());
    assert!(app.active().search.is_none());
}

#[test]
fn command_picker_inserts_deliberate_optional_commands() {
    for (query, input) in [
        ("pair", "/pair "),
        ("save", "/save"),
        ("leave", "/leave"),
        ("rename", "/rename "),
    ] {
        let mut app = app();
        type_text(&mut app, "old draft");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        type_text(&mut app, query);

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(action.is_none(), "/{query} must not run from the picker");
        assert!(app.command_picker.is_none());
        assert_eq!(app.active().input, input);
        assert_eq!(app.active().cursor, input.chars().count());
    }
}

#[test]
fn ctrl_w_deletes_word_backward() {
    let mut app = app();
    for c in "hello world".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(app.active().input, "hello ");
    assert_eq!(app.active().cursor, 6);
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(app.active().input, "");
    assert_eq!(app.active().cursor, 0);
}

#[test]
fn peer_joined_and_left_update_peers() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    assert_eq!(app.active().peers, vec![PEER_ID]);
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    assert_eq!(
        app.active().peers,
        vec![PEER_ID],
        "joining twice should not duplicate"
    );
    app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));
    assert!(app.active().peers.is_empty());
}

#[test]
fn received_message_is_deduped_by_id() {
    let mut app = app();
    let message = compose_message(PEER_ID, "hi from a peer", None);
    let before = app.active().messages.len();
    app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
    assert_eq!(app.active().messages.len(), before + 1);
    // Gossip is best-effort and can redeliver -- the same id must not
    // be shown twice.
    app.handle_net_event(NetEvent::Received("general".to_string(), message));
    assert_eq!(app.active().messages.len(), before + 1);
}

#[test]
fn a_message_is_still_deduped_after_hundreds_of_others_arrive() {
    // Regression test for a real duplication bug: `seen_ids` used to be
    // a 256-entry recency window, but `net::Net::sync_history` can
    // redeliver a channel's *entire* history at any time (not just
    // messages sent since the last sync -- see
    // `backfill::BackfillStore::fetch`), so an old-but-still-displayed
    // message must still be rejected as a duplicate no matter how much
    // channel activity happened after it.
    let mut app = app();
    let first = compose_message(PEER_ID, "the first message", None);
    app.handle_net_event(NetEvent::Received("general".to_string(), first.clone()));

    for i in 0..300 {
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, &format!("filler {i}"), None),
        ));
    }

    let before = app.active().messages.len();
    let result = app.handle_net_event(NetEvent::Received("general".to_string(), first));
    assert!(
        result.is_none(),
        "a redelivery of an old message must still be rejected as a duplicate"
    );
    assert_eq!(
        app.active().messages.len(),
        before,
        "must not be re-inserted into the transcript"
    );
}

#[test]
fn tab_and_backtab_cycle_channels_and_clear_unread() {
    let mut app = multi_channel_app();
    assert_eq!(app.active, 1, "starts on the last-joined channel");

    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.active, 0, "Tab wraps around");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.active, 1);

    app.handle_key(key(KeyCode::BackTab));
    assert_eq!(app.active, 0, "Shift+Tab moves backward");
    app.handle_key(key(KeyCode::BackTab));
    assert_eq!(app.active, 1, "and wraps the other way");
}

#[test]
fn switching_channels_preserves_each_channels_own_draft() {
    // Regression test for features.md's "Per-channel input drafts":
    // `input`/`cursor` live on `Channel`, not `AppState`, so switching
    // tabs must neither leak a half-typed draft into the newly active
    // channel nor lose it from the one it was written for.
    let mut app = multi_channel_app();
    assert_eq!(app.active().name, "random");

    for c in "hello from random".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Left));
    let cursor_in_random = app.active().cursor;

    app.handle_key(key(KeyCode::Tab)); // wraps to "general"
    assert_eq!(app.active().name, "general");
    assert_eq!(
        app.active().input,
        "",
        "switching tabs must not leak the other channel's draft in"
    );
    assert_eq!(app.active().cursor, 0);

    for c in "typed in general".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }

    app.handle_key(key(KeyCode::Tab)); // back to "random"
    assert_eq!(app.active().name, "random");
    assert_eq!(
        app.active().input,
        "hello from random",
        "switching away and back must not lose or clear the draft"
    );
    assert_eq!(app.active().cursor, cursor_in_random);

    app.handle_key(key(KeyCode::BackTab)); // back to "general"
    assert_eq!(app.active().input, "typed in general");
}

#[test]
fn switching_into_a_channel_clears_its_unread_count_and_sets_an_anchor() {
    let mut app = multi_channel_app();
    app.active = 1; // on "random"
    app.channels[0].unread_count = 2; // "general" has unread activity
    app.channels[0].first_unread = Some(7);

    app.handle_key(key(KeyCode::Tab)); // wraps to "general"
    assert_eq!(app.active, 0);
    assert_eq!(app.active().unread_count, 0);
    assert_eq!(app.active().first_unread, None);
    assert_eq!(app.active().unread_anchor, Some(7));
}

#[test]
fn received_messages_on_an_inactive_channel_increment_unread_count_from_the_first() {
    let mut app = multi_channel_app();
    app.active = 1; // "random" is active; "general" is not
    let first = compose_message(PEER_ID, "hello", None);
    let first_id = first.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), first));
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "again", None),
    ));

    assert_eq!(app.channels[0].unread_count, 2);
    assert_eq!(app.channels[0].first_unread, Some(first_id));
    assert_eq!(
        app.channels[1].unread_count, 0,
        "active channel is untouched"
    );
}

#[test]
fn received_message_on_active_channel_does_not_mark_unread() {
    let mut app = app();
    let message = compose_message(PEER_ID, "hello", None);
    app.handle_net_event(NetEvent::Received("general".to_string(), message));
    assert_eq!(app.active().unread_count, 0);
    assert_eq!(app.active().first_unread, None);
}

#[test]
fn duplicate_messages_do_not_increment_an_unread_count() {
    let mut app = multi_channel_app();
    app.active = 1; // on "random"
    let message = compose_message(PEER_ID, "hello", None);

    app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
    app.handle_net_event(NetEvent::Received("general".to_string(), message));

    assert_eq!(app.channels[0].unread_count, 1);
}

#[test]
fn scrolling_after_switching_discards_the_unread_anchor() {
    let mut app = multi_channel_app();
    app.active = 1; // on "random"
    app.channels[0].unread_count = 1;
    app.channels[0].first_unread = Some(7);

    app.handle_key(key(KeyCode::Tab));
    app.handle_key(key(KeyCode::Up));

    assert_eq!(app.active().unread_anchor, None);
}

#[test]
fn messages_and_peers_are_isolated_per_channel() {
    let mut app = multi_channel_app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "only in general", None),
    ));

    assert_eq!(app.channels[0].peers, vec![PEER_ID]);
    assert_eq!(app.channels[0].messages.len(), 1);
    assert!(app.channels[1].peers.is_empty());
    assert!(app.channels[1].messages.is_empty());
}

#[test]
fn slash_join_with_arg_returns_join_action() {
    let mut app = app();
    for c in "/join project-x".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let action = app.handle_key(key(KeyCode::Enter));
    match action.expect("non-empty /join returns an action") {
        InputAction::Join(arg) => assert_eq!(arg, "project-x"),
        _ => panic!("expected InputAction::Join"),
    }
    assert_eq!(app.active().input, "");
}

#[test]
fn slash_join_without_arg_shows_usage_and_returns_none() {
    let mut app = app();
    for c in "/join".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let action = app.handle_key(key(KeyCode::Enter));
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"));
}

#[test]
fn slash_join_rejects_the_reserved_device_sync_channel_name() {
    let mut app = app();
    let action = submit(&mut app, "/join _devices");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("reserved"), "got: {last}");
}

#[test]
fn unknown_command_shows_error_and_returns_none() {
    let mut app = app();
    for c in "/bogus".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let action = app.handle_key(key(KeyCode::Enter));
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "unknown command: /bogus");
}

#[test]
fn help_command_pushes_a_system_notice_and_returns_none() {
    let mut app = app();
    let action = submit(&mut app, "/help");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("commands:"), "got: {last}");
}

#[test]
fn help_puts_each_command_on_its_own_line() {
    let mut app = app();
    submit(&mut app, "/help");
    let last = as_system(app.active().messages.back().unwrap());
    for command in ["/join ", "/msg ", "/sync ", "/help "] {
        assert!(
            last.lines().any(|line| line.starts_with(command)),
            "{command} should start its own line, got: {last}"
        );
    }
    assert!(last.contains("/who peer details"), "got: {last}");
    assert!(
        last.lines().any(|line| line == "Ctrl+C -- quit"),
        "got: {last}"
    );
}

#[test]
fn help_lists_the_files_overlay_and_its_keys() {
    let mut app = app();
    submit(&mut app, "/help");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("/files"), "got: {last}");
    assert!(last.contains("Ctrl+O"), "got: {last}");
}

#[test]
fn hint_hover_reports_a_change_only_when_the_row_changes() {
    let mut app = app();
    assert!(app.set_hint_hover(Some(12)));
    assert_eq!(app.hovered_hint(), Some(12));
    assert!(!app.set_hint_hover(Some(12)));
    assert!(app.set_hint_hover(Some(1)));
    assert_eq!(app.hovered_hint(), Some(1));
    assert!(app.set_hint_hover(None));
    assert_eq!(app.hovered_hint(), None);
    assert!(!app.set_hint_hover(None));
}

#[test]
fn hiding_hints_and_opening_an_overlay_cancel_hover() {
    let mut app = app();
    app.set_hint_hover(Some(12));
    submit(&mut app, "/hints");
    assert!(!app.can_hover_hints());
    assert!(app.hint_hover.is_none());
    assert!(!app.set_hint_hover(Some(12)));
    submit(&mut app, "/hints");
    app.set_hint_hover(Some(12));
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(!app.can_hover_hints());
    assert_eq!(app.hovered_hint(), None);
    assert!(app.set_hint_hover(None));
}

#[test]
fn disabling_the_mouse_cancels_hint_hover_but_keeps_the_hints_panel() {
    let mut app = app();
    app.set_hint_hover(Some(12));
    submit(&mut app, "/mouse");
    assert!(app.show_hints);
    assert!(!app.can_hover());
    assert!(app.hint_hover.is_none());
    assert!(!app.set_hint_hover(Some(12)));
    submit(&mut app, "/mouse");
    assert!(app.set_hint_hover(Some(12)));
}

#[test]
fn opening_the_command_picker_cancels_hint_hover() {
    let mut app = app();
    app.set_hint_hover(Some(12));
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert!(app.command_picker.is_some());
    assert!(!app.can_hover_hints());
    assert_eq!(app.hovered_hint(), None);
}

#[test]
fn hints_are_shown_by_default() {
    assert!(app().show_hints);
}

#[test]
fn slash_hints_toggles_visibility_and_reports_state() {
    let mut app = app();

    submit(&mut app, "/hints");
    assert!(!app.show_hints);
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "command hints hidden");

    submit(&mut app, "/hints");
    assert!(app.show_hints);
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "command hints shown");
}

#[test]
fn bell_is_enabled_by_default() {
    assert!(app().bell_enabled);
}

#[test]
fn slash_bell_toggles_and_reports_state_and_returns_an_action() {
    let mut app = app();

    let action = submit(&mut app, "/bell");
    assert!(!app.bell_enabled);
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "message bell off");
    match action.expect("/bell returns an action") {
        InputAction::Bell(enabled) => assert!(!enabled),
        _ => panic!("expected InputAction::Bell"),
    }

    let action = submit(&mut app, "/bell");
    assert!(app.bell_enabled);
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "message bell on");
    match action.expect("/bell returns an action") {
        InputAction::Bell(enabled) => assert!(enabled),
        _ => panic!("expected InputAction::Bell"),
    }
}

#[test]
fn mouse_is_enabled_by_default() {
    assert!(app().mouse_enabled);
}

#[test]
fn slash_mouse_toggles_and_reports_state_and_returns_an_action() {
    let mut app = app();

    let action = submit(&mut app, "/mouse");
    assert!(!app.mouse_enabled);
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "mouse reporting off");
    match action.expect("/mouse returns an action") {
        InputAction::Mouse(enabled) => assert!(!enabled),
        _ => panic!("expected InputAction::Mouse"),
    }

    let action = submit(&mut app, "/mouse");
    assert!(app.mouse_enabled);
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "mouse reporting on");
    match action.expect("/mouse returns an action") {
        InputAction::Mouse(enabled) => assert!(enabled),
        _ => panic!("expected InputAction::Mouse"),
    }
}

#[test]
fn slash_hints_leaves_the_mouse_alone() {
    let mut app = app();
    submit(&mut app, "/hints");
    assert!(app.mouse_enabled);
}

#[test]
fn load_settings_seeds_bell_and_mouse_enabled() {
    let mut app = app();
    app.load_settings(false, false, Some("me".to_string()));
    assert!(!app.bell_enabled);
    assert!(!app.mouse_enabled);
    assert_eq!(app.local_nickname.as_deref(), Some("me"));
}

#[test]
fn slash_nick_updates_this_devices_who_row() {
    let mut app = app();

    let action = submit(&mut app, "/nick me");
    submit(&mut app, "/who");

    assert!(matches!(action, Some(InputAction::Nick(name)) if name == "me"));
    let self_row = app
        .viewing_peers
        .as_ref()
        .unwrap()
        .peers
        .iter()
        .find(|peer| peer.id == SELF_ID)
        .unwrap();
    assert_eq!(self_row.nickname.as_deref(), Some("me"));
}

#[test]
fn slash_invite_returns_invite_action_for_active_channel() {
    let mut app = multi_channel_app();
    for c in "/invite".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let action = app.handle_key(key(KeyCode::Enter));
    match action.expect("/invite returns an action") {
        InputAction::Invite(channel) => assert_eq!(channel, "random"),
        _ => panic!("expected InputAction::Invite"),
    }
}

#[test]
fn slash_paste_returns_paste_action_for_active_channel() {
    let mut app = multi_channel_app();
    let action = submit(&mut app, "/paste");
    match action.expect("/paste returns an action") {
        InputAction::Paste(channel) => assert_eq!(channel, "random"),
        _ => panic!("expected InputAction::Paste"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("clipboard"), "got: {last}");
}

#[test]
fn ctrl_v_triggers_the_same_paste_action_as_the_slash_command() {
    let mut app = app();
    let action = app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    match action.expect("Ctrl+V returns an action") {
        InputAction::Paste(channel) => assert_eq!(channel, "general"),
        _ => panic!("expected InputAction::Paste"),
    }
}

#[test]
fn super_v_also_triggers_paste_best_effort_for_macos() {
    let mut app = app();
    let action = app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SUPER));
    assert!(matches!(action, Some(InputAction::Paste(_))));
}

#[test]
fn plain_v_without_a_modifier_just_types_the_character() {
    let mut app = app();
    let action = app.handle_key(key(KeyCode::Char('v')));
    assert!(action.is_none());
    assert_eq!(app.active().input, "v");
}

#[test]
fn compose_own_message_matches_the_calling_apps_identity() {
    let app = app();
    let message = app.compose_own_message("hello from clipboard");
    assert_eq!(message.sender, SELF_ID);
    assert_eq!(message.text, "hello from clipboard");
    assert!(message.attachment.is_none());
}

/// A staged attachment named `filename`, as `files::stage_image` would
/// build it -- the tests below only care that attachments are kept,
/// ordered, and sent, not where their bytes come from.
fn staged(filename: &str) -> StagedAttachment {
    StagedAttachment {
        filename: filename.to_string(),
        size: 10,
        source: crate::files::StagedSource::Bytes(vec![0; 10]),
    }
}

#[test]
fn staging_an_attachment_puts_it_on_the_named_channel_only() {
    let mut app = multi_channel_app(); // active: "random"
    app.stage_attachment("general", staged("map.png"));

    assert_eq!(app.channels[0].name, "general");
    assert_eq!(app.channels[0].staged, vec![staged("map.png")]);
    assert!(app.channels[1].staged.is_empty());
}

#[test]
fn staging_for_a_channel_that_is_gone_is_a_no_op() {
    let mut app = app();
    app.stage_attachment("left-meanwhile", staged("map.png"));
    assert!(app.active().staged.is_empty());
}

#[test]
fn enter_sends_every_staged_attachment_and_clears_the_stage() {
    let mut app = app();
    app.stage_attachment("general", staged("a.png"));
    app.stage_attachment("general", staged("b.pdf"));

    let action = app.handle_key(key(KeyCode::Enter));

    match action.expect("Enter with staged attachments returns an action") {
        InputAction::SendStaged {
            channel,
            items,
            caption,
        } => {
            assert_eq!(channel, "general");
            assert_eq!(items, vec![staged("a.png"), staged("b.pdf")]);
            assert_eq!(caption, "", "nothing was typed, so no caption");
        }
        _ => panic!("expected InputAction::SendStaged"),
    }
    assert!(app.active().staged.is_empty());
}

#[test]
fn typed_text_becomes_the_caption_of_the_staged_attachments() {
    let mut app = app();
    app.stage_attachment("general", staged("map.png"));

    let action = submit(&mut app, "the new tilemap");

    match action.expect("returns an action") {
        InputAction::SendStaged { items, caption, .. } => {
            assert_eq!(caption, "the new tilemap");
            assert_eq!(items.len(), 1);
        }
        _ => panic!("expected InputAction::SendStaged"),
    }
    assert_eq!(
        app.active().input,
        "",
        "the caption is consumed from the input box"
    );
}

#[test]
fn sending_staged_attachments_leaves_a_breadcrumb_per_file_and_no_chat_message() {
    let mut app = app();
    app.stage_attachment("general", staged("a.png"));
    app.stage_attachment("general", staged("b.pdf"));
    let before = app.active().messages.len();

    submit(&mut app, "hello");

    // Only system breadcrumbs: the attachments' chat messages arrive
    // later from the importer (`NetEvent::FileReady`), with the caption
    // riding on the first of them.
    let added: Vec<&TranscriptLine> = app.active().messages.iter().skip(before).collect();
    assert_eq!(added.len(), 2);
    assert_eq!(as_system(added[0]), "sending a.png...");
    assert_eq!(as_system(added[1]), "sending b.pdf...");
}

#[test]
fn whitespace_only_input_still_sends_staged_attachments_without_a_caption() {
    let mut app = app();
    app.stage_attachment("general", staged("map.png"));
    app.paste_text("   ");

    let action = app.handle_key(key(KeyCode::Enter));

    match action.expect("returns an action") {
        InputAction::SendStaged { caption, .. } => assert_eq!(caption, ""),
        _ => panic!("expected InputAction::SendStaged"),
    }
    assert_eq!(app.active().input, "");
}

#[test]
fn a_slash_command_runs_as_a_command_and_leaves_the_stage_alone() {
    let mut app = app();
    app.stage_attachment("general", staged("map.png"));

    let action = submit(&mut app, "/who");

    assert!(action.is_none(), "/who is a local command, not a send");
    assert_eq!(app.active().staged, vec![staged("map.png")]);
}

#[test]
fn slash_send_still_sends_immediately_even_with_attachments_staged() {
    let mut app = app();
    app.stage_attachment("general", staged("staged.png"));

    let action = submit(&mut app, "/send ~/other.png");

    match action.expect("/send returns an action") {
        InputAction::SendFile { channel, path } => {
            assert_eq!(channel, "general");
            assert_eq!(path, "~/other.png");
        }
        _ => panic!("expected InputAction::SendFile"),
    }
    assert_eq!(app.active().staged.len(), 1, "the stage is untouched");
}

#[test]
fn backspace_on_an_empty_input_unstages_the_newest_attachment() {
    let mut app = app();
    app.stage_attachment("general", staged("first.png"));
    app.stage_attachment("general", staged("second.png"));

    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.active().staged, vec![staged("first.png")]);

    app.handle_key(key(KeyCode::Backspace));
    assert!(app.active().staged.is_empty());
}

#[test]
fn backspace_with_text_typed_edits_the_text_and_keeps_the_stage() {
    let mut app = app();
    app.stage_attachment("general", staged("map.png"));
    app.handle_key(key(KeyCode::Char('h')));
    app.handle_key(key(KeyCode::Char('i')));

    app.handle_key(key(KeyCode::Backspace));

    assert_eq!(app.active().input, "h");
    assert_eq!(app.active().staged.len(), 1);
}

#[test]
fn esc_clears_the_stage_before_it_quits() {
    let mut app = app();
    app.stage_attachment("general", staged("a.png"));
    app.stage_attachment("general", staged("b.png"));

    app.handle_key(key(KeyCode::Esc));
    assert!(app.active().staged.is_empty());
    assert!(!app.should_quit, "the first Esc only clears the stage");

    app.handle_key(key(KeyCode::Esc));
    assert!(app.should_quit);
}

#[test]
fn esc_cancels_an_armed_reply_before_clearing_the_stage() {
    let mut app = app();
    app.stage_attachment("general", staged("a.png"));
    app.replying_to = Some(1);

    app.handle_key(key(KeyCode::Esc));

    assert!(app.replying_to.is_none());
    assert_eq!(
        app.active().staged.len(),
        1,
        "the reply is backed out of first; the stage survives"
    );
}

#[test]
fn staged_attachments_stay_with_their_channel_across_tab_switches() {
    let mut app = multi_channel_app(); // starts on "random"
    app.stage_attachment("random", staged("map.png"));

    app.handle_key(key(KeyCode::Tab)); // wraps around to "general"
    assert_eq!(app.active().name, "general");
    assert!(
        app.active().staged.is_empty(),
        "another channel's stage must not show up here"
    );

    // Enter in "general" sends plain text, never random's attachment.
    let action = submit(&mut app, "hi");
    assert!(matches!(action, Some(InputAction::Send(..))));

    app.handle_key(key(KeyCode::Tab)); // back to "random"
    assert_eq!(app.active().staged, vec![staged("map.png")]);
}

#[test]
fn a_fresh_session_is_composing() {
    assert!(app().is_composing());
}

#[test]
fn composing_stops_while_the_switcher_is_open() {
    let mut app = app();
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(!app.is_composing());

    app.handle_key(key(KeyCode::Esc));
    assert!(app.is_composing());
}

#[test]
fn composing_stops_during_a_reply_or_thread_pick_and_a_pairing_confirmation() {
    let mut picking_reply = app();
    picking_reply.picking_reply = Some(1);
    assert!(!picking_reply.is_composing());

    let mut picking_thread = app();
    picking_thread.picking_thread = Some(1);
    assert!(!picking_thread.is_composing());

    let mut redeeming = app();
    redeeming.pending_redeem = Some("ticket".to_string());
    assert!(!redeeming.is_composing());
}

// -- the file picker, opened by a bare /send -----------------------------

use crate::picker::{Entry, Kind};

fn picked_file(name: &str) -> Entry {
    Entry::new(
        name.to_string(),
        Kind::File {
            size: 1,
            modified: std::time::SystemTime::UNIX_EPOCH,
        },
    )
}

fn picked_folder(name: &str) -> Entry {
    Entry::new(name.to_string(), Kind::Folder)
}

fn listing(request: u64, entries: Vec<Entry>) -> FolderListing {
    FolderListing {
        request,
        result: Ok(entries),
    }
}

/// The request id of the folder read `action` asks for.
fn read_request(action: Option<InputAction>) -> u64 {
    match action {
        Some(InputAction::ReadFolder { request, .. }) => request,
        _ => panic!("expected InputAction::ReadFolder"),
    }
}

/// Opens the picker with a bare `/send` on `app`, whose picker folder is
/// `/pics`, and answers its read with a folder and two files.
fn open_picker(app: &mut AppState) {
    app.load_browse_dir(PathBuf::from("/pics"));
    let request = read_request(submit(app, "/send"));
    app.apply_folder_listing(listing(
        request,
        vec![
            picked_folder("docs"),
            picked_file("a.txt"),
            picked_file("b.txt"),
        ],
    ));
}

#[test]
fn a_bare_send_opens_the_picker_and_asks_for_the_remembered_folder_to_be_read() {
    let mut app = app();
    app.load_browse_dir(PathBuf::from("/pics"));

    let action = submit(&mut app, "/send");

    match action {
        Some(InputAction::ReadFolder { request, dir }) => {
            assert_eq!(request, 1);
            assert_eq!(dir, PathBuf::from("/pics"));
        }
        _ => panic!("expected InputAction::ReadFolder"),
    }
    let picker = app.picking_file.as_ref().expect("the picker is open");
    assert_eq!(picker.dir(), Path::new("/pics"));
    assert!(picker.is_reading());
    assert_eq!(app.active().input, "", "the command was consumed");
    assert!(app.active().messages.is_empty(), "no usage notice any more");
}

#[test]
fn the_picker_opens_at_the_root_until_a_folder_is_seeded() {
    let mut app = app();

    match submit(&mut app, "/send") {
        Some(InputAction::ReadFolder { dir, .. }) => {
            assert_eq!(dir, PathBuf::from(std::path::MAIN_SEPARATOR_STR));
        }
        _ => panic!("expected InputAction::ReadFolder"),
    }
}

#[test]
fn send_with_a_path_still_sends_it_and_leaves_the_picker_closed() {
    let mut app = app();

    let action = submit(&mut app, "/send ~/Downloads/a.png");

    assert!(matches!(action, Some(InputAction::SendFile { .. })));
    assert!(app.picking_file.is_none());
}

#[test]
fn the_picker_owns_keys_and_pastes_while_open() {
    let mut app = app();
    open_picker(&mut app);
    assert!(!app.is_composing());

    app.handle_key(key(KeyCode::Char('a')));
    app.paste_text("txt\n");

    assert_eq!(app.active().input, "", "nothing lands in the draft");
    let picker = app.picking_file.as_ref().expect("still open");
    assert_eq!(
        picker.query(),
        "atxt",
        "a paste filters, minus its line break"
    );
}

#[test]
fn the_picker_answers_keys_before_its_folder_has_arrived() {
    let mut app = app();
    app.load_browse_dir(PathBuf::from("/pics"));
    submit(&mut app, "/send"); // never answered: a huge or slow folder

    assert!(app.handle_key(key(KeyCode::Down)).is_none());
    assert!(app.handle_key(key(KeyCode::Char('x'))).is_none());
    assert!(app.handle_key(key(KeyCode::Enter)).is_none());
    assert!(app.picking_file.is_some(), "still open, and still waiting");

    app.handle_key(key(KeyCode::Esc));
    assert!(app.picking_file.is_none(), "and Esc still closes it");
}

#[test]
fn escape_closes_the_picker_without_staging_or_quitting() {
    let mut app = app();
    open_picker(&mut app);

    let action = app.handle_key(key(KeyCode::Esc));

    assert!(action.is_none());
    assert!(app.picking_file.is_none());
    assert!(!app.should_quit);
    assert!(app.is_composing());
}

#[test]
fn ctrl_c_still_quits_from_the_picker() {
    let mut app = app();
    open_picker(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit);
}

#[test]
fn choosing_a_file_stages_it_on_the_channel_the_picker_was_opened_on() {
    let mut app = multi_channel_app(); // "random" is active
    open_picker(&mut app);
    app.handle_key(key(KeyCode::Down)); // a.txt
    app.active = 0; // a join switched channels meanwhile

    let action = app.handle_key(key(KeyCode::Enter));

    match action {
        Some(InputAction::StageFiles {
            channel,
            paths,
            folder,
        }) => {
            assert_eq!(channel, "random");
            assert_eq!(paths, [PathBuf::from("/pics/a.txt")]);
            assert_eq!(folder, PathBuf::from("/pics"));
        }
        _ => panic!("expected InputAction::StageFiles"),
    }
    assert!(app.picking_file.is_none(), "choosing closes the picker");
}

#[test]
fn marked_files_are_all_staged_together() {
    let mut app = app();
    open_picker(&mut app);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char(' '))); // a.txt
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Char(' '))); // b.txt

    match app.handle_key(key(KeyCode::Enter)) {
        Some(InputAction::StageFiles { paths, .. }) => assert_eq!(
            paths,
            [PathBuf::from("/pics/a.txt"), PathBuf::from("/pics/b.txt")]
        ),
        _ => panic!("expected InputAction::StageFiles"),
    }
}

#[test]
fn the_folder_files_were_chosen_from_is_where_the_picker_opens_next() {
    let mut app = app();
    open_picker(&mut app);
    // Into docs, then choose a file there.
    let request = read_request(app.handle_key(key(KeyCode::Enter)));
    app.apply_folder_listing(listing(request, vec![picked_file("deep.txt")]));
    app.handle_key(key(KeyCode::Enter));

    match submit(&mut app, "/send") {
        Some(InputAction::ReadFolder { dir, .. }) => {
            assert_eq!(dir, PathBuf::from("/pics/docs"));
        }
        _ => panic!("expected InputAction::ReadFolder"),
    }
}

#[test]
fn cancelling_does_not_change_where_the_picker_opens_next() {
    let mut app = app();
    open_picker(&mut app);
    let request = read_request(app.handle_key(key(KeyCode::Enter))); // into docs
    app.apply_folder_listing(listing(request, vec![picked_file("deep.txt")]));
    app.handle_key(key(KeyCode::Esc));

    match submit(&mut app, "/send") {
        Some(InputAction::ReadFolder { dir, .. }) => {
            assert_eq!(dir, PathBuf::from("/pics"));
        }
        _ => panic!("expected InputAction::ReadFolder"),
    }
}

#[test]
fn opening_a_folder_asks_for_it_under_a_fresh_request_id() {
    let mut app = app();
    open_picker(&mut app); // request 1

    match app.handle_key(key(KeyCode::Enter)) {
        Some(InputAction::ReadFolder { request, dir }) => {
            assert_eq!(request, 2);
            assert_eq!(dir, PathBuf::from("/pics/docs"));
        }
        _ => panic!("expected InputAction::ReadFolder"),
    }
}

#[test]
fn going_up_asks_for_the_parent_folder() {
    let mut app = app();
    open_picker(&mut app);
    let request = read_request(app.handle_key(key(KeyCode::Enter))); // into docs
    app.apply_folder_listing(listing(request, vec![picked_file("deep.txt")]));

    match app.handle_key(key(KeyCode::Backspace)) {
        Some(InputAction::ReadFolder { dir, .. }) => {
            assert_eq!(dir, PathBuf::from("/pics"));
        }
        _ => panic!("expected InputAction::ReadFolder"),
    }
}

#[test]
fn an_answer_only_reaches_the_picker_waiting_for_it() {
    let mut app = app();
    app.load_browse_dir(PathBuf::from("/pics"));
    let first = read_request(submit(&mut app, "/send"));
    app.handle_key(key(KeyCode::Esc));
    let second = read_request(submit(&mut app, "/send"));
    assert_ne!(first, second, "every read has an id of its own");

    // The first picker's slow answer turns up after the second opened.
    app.apply_folder_listing(listing(first, vec![picked_file("stale.txt")]));
    let picker = app.picking_file.as_ref().expect("open");
    assert!(picker.is_reading(), "still waiting for its own");
    assert_eq!(picker.visible().len(), 0);

    app.apply_folder_listing(listing(second, vec![picked_file("fresh.txt")]));
    let picker = app.picking_file.as_ref().expect("open");
    assert!(!picker.is_reading());
    assert_eq!(picker.visible().len(), 1);
}

#[test]
fn an_answer_after_the_picker_closed_is_ignored() {
    let mut app = app();
    app.load_browse_dir(PathBuf::from("/pics"));
    let request = read_request(submit(&mut app, "/send"));
    app.handle_key(key(KeyCode::Esc));

    app.apply_folder_listing(listing(request, vec![picked_file("late.txt")]));

    assert!(app.picking_file.is_none());
}

#[test]
fn slash_leave_with_no_arg_leaves_the_active_channel() {
    let mut app = multi_channel_app();
    let action = submit(&mut app, "/leave");
    match action.expect("/leave returns an action") {
        InputAction::Leave(channel) => assert_eq!(channel, "random"),
        _ => panic!("expected InputAction::Leave"),
    }
}

#[test]
fn slash_leave_with_a_name_leaves_that_channel_even_if_inactive() {
    let mut app = multi_channel_app();
    assert_eq!(app.active().name, "random");
    let action = submit(&mut app, "/leave general");
    match action.expect("/leave returns an action") {
        InputAction::Leave(channel) => assert_eq!(channel, "general"),
        _ => panic!("expected InputAction::Leave"),
    }
}

#[test]
fn slash_leave_refuses_an_unjoined_channel() {
    let mut app = multi_channel_app();
    let action = submit(&mut app, "/leave nonexistent");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "not currently in #nonexistent");
}

#[test]
fn slash_leave_refuses_the_only_channel() {
    let mut app = app();
    let action = submit(&mut app, "/leave");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "cannot leave your only channel");
}

#[test]
fn slash_who_includes_this_devices_endpoint_without_a_transcript_notice() {
    let mut app = app();
    let messages = app.active().messages.len();

    let action = submit(&mut app, "/who");

    assert!(action.is_none());
    let overlay = app.viewing_peers.as_ref().expect("/who opens an overlay");
    assert_eq!(overlay.heading, "#general");
    assert_eq!(
        overlay
            .peers
            .iter()
            .map(|peer| (peer.id, peer.alias.as_deref(), peer.nickname.as_deref()))
            .collect::<Vec<_>>(),
        vec![(SELF_ID, None, None)]
    );
    assert_eq!(app.active().messages.len(), messages);
    assert!(!app.is_composing());
}

#[test]
fn slash_who_snapshots_endpoint_alias_and_nickname_for_each_active_peer() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID_2));
    app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "alice".to_string(),
        },
    ));

    submit(&mut app, "/who");

    let overlay = app.viewing_peers.as_ref().unwrap();
    assert_eq!(
        overlay
            .peers
            .iter()
            .map(|peer| (peer.id, peer.alias.as_deref(), peer.nickname.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (PEER_ID, Some("Bob"), Some("alice")),
            (PEER_ID_2, None, None),
            (SELF_ID, None, None),
        ]
    );
}

#[test]
fn slash_who_only_snapshots_the_active_channels_peers() {
    let mut app = multi_channel_app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    assert_eq!(app.active().name, "random");

    submit(&mut app, "/who");

    let overlay = app.viewing_peers.as_ref().unwrap();
    assert_eq!(overlay.heading, "#random");
    assert_eq!(
        overlay.peers.iter().map(|peer| peer.id).collect::<Vec<_>>(),
        [SELF_ID]
    );
}

#[test]
fn slash_who_lists_online_peers_before_remembered_offline_peers() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    app.record_peer_seen("general", PEER_ID_2, 100);

    submit(&mut app, "/who");

    assert_eq!(
        app.viewing_peers
            .as_ref()
            .unwrap()
            .peers
            .iter()
            .map(|peer| (peer.id, peer.last_seen_unix_ms))
            .collect::<Vec<_>>(),
        vec![(PEER_ID, None), (SELF_ID, None), (PEER_ID_2, Some(100))]
    );
}

#[test]
fn slash_who_deduplicates_certified_devices_like_the_peer_sidebar() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID_2));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));
    app.load_contacts(HashMap::from([(
        *user_key.public().as_bytes(),
        "Alice".to_string(),
    )]));

    submit(&mut app, "/who");

    let overlay = app.viewing_peers.as_ref().unwrap();
    assert_eq!(
        overlay.peers.iter().map(|peer| peer.id).collect::<Vec<_>>(),
        vec![PEER_ID, SELF_ID]
    );
    assert!(
        overlay
            .peers
            .iter()
            .filter(|peer| peer.id != SELF_ID)
            .all(|peer| peer.alias.as_deref() == Some("Alice"))
    );
}

#[test]
fn esc_closes_the_who_overlay_and_its_paste_never_reaches_the_draft() {
    let mut app = app();
    submit(&mut app, "/who");
    app.paste_text("hidden");

    app.handle_key(key(KeyCode::Esc));

    assert!(app.viewing_peers.is_none());
    assert!(app.is_composing());
    assert!(!app.should_quit);
    assert!(app.active().input.is_empty());
}

#[test]
fn remove_channel_switches_active_to_the_previous_tab_when_removing_the_active_one() {
    let mut app = multi_channel_app();
    assert_eq!(app.active().name, "random");
    app.remove_channel("random");
    assert_eq!(app.channels.len(), 1);
    assert_eq!(app.active().name, "general");
}

#[test]
fn remove_channel_keeps_the_active_tab_selected_when_removing_a_different_one() {
    let mut app = multi_channel_app();
    assert_eq!(app.active().name, "random");
    app.remove_channel("general");
    assert_eq!(app.channels.len(), 1);
    assert_eq!(
        app.active().name,
        "random",
        "removing an earlier tab must not change which channel is active"
    );
}

#[test]
fn remove_channel_pushes_a_left_system_notice() {
    let mut app = multi_channel_app();
    app.remove_channel("random");
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "left #random");
}

#[test]
fn remove_channel_is_a_no_op_for_an_unknown_channel() {
    let mut app = multi_channel_app();
    app.remove_channel("nonexistent");
    assert_eq!(app.channels.len(), 2);
    assert_eq!(app.active().name, "random");
}

#[test]
fn joined_event_adds_new_channel_and_switches_to_it() {
    let mut app = app();
    app.handle_net_event(NetEvent::Joined("project-x".to_string()));
    assert_eq!(app.channels.len(), 2);
    assert_eq!(app.active().name, "project-x");
}

#[test]
fn joined_event_on_existing_channel_switches_without_duplicating() {
    let mut app = multi_channel_app();
    app.active = 0;
    app.handle_net_event(NetEvent::Joined("random".to_string()));
    assert_eq!(
        app.channels.len(),
        2,
        "must not duplicate an existing channel"
    );
    assert_eq!(app.active().name, "random");
}

#[test]
fn channel_sync_joined_adds_a_background_tab_without_switching_active() {
    // Unlike a plain `Joined` (the user explicitly ran /join), a
    // channel learned via the private device-sync channel must not
    // yank focus away from whatever the user is currently looking at.
    let mut app = app();
    assert_eq!(app.active().name, "general");

    app.handle_net_event(NetEvent::ChannelSyncJoined("project-x".to_string()));

    assert_eq!(app.channels.len(), 2);
    assert_eq!(
        app.active().name,
        "general",
        "a background sync join must not switch the active tab"
    );
    assert!(app.channels.iter().any(|c| c.name == "project-x"));
}

#[test]
fn channel_sync_joined_on_an_already_known_channel_does_not_duplicate() {
    let mut app = multi_channel_app();
    app.handle_net_event(NetEvent::ChannelSyncJoined("random".to_string()));
    assert_eq!(app.channels.len(), 2, "must not duplicate an existing tab");
}

#[test]
fn lagged_pushes_system_message_on_the_affected_channel() {
    let mut app = multi_channel_app();
    app.active = 1; // "random" is active; "general" is not

    app.handle_net_event(NetEvent::Lagged("general".to_string()));

    let last = as_system(app.channels[0].messages.back().unwrap());
    assert!(last.contains("lagged"));
    assert!(
        app.channels[1].messages.is_empty(),
        "lag on another channel shouldn't touch the active one"
    );
}

#[test]
fn join_failed_pushes_system_message_on_active_channel() {
    let mut app = app();
    app.handle_net_event(NetEvent::JoinFailed(
        "project-x".to_string(),
        "boom".to_string(),
    ));
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "failed to join #project-x: boom");
}

#[test]
fn handle_net_event_returns_channel_and_message_for_a_new_received_message() {
    let mut app = app();
    let message = compose_message(PEER_ID, "hello", None);
    let result = app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
    match result {
        Some((channel, returned)) => {
            assert_eq!(channel, "general");
            assert_eq!(returned, message);
        }
        None => panic!("expected Some for a newly-received message"),
    }
}

#[test]
fn handle_net_event_returns_none_for_a_duplicate_received_message() {
    let mut app = app();
    let message = compose_message(PEER_ID, "hello", None);
    app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
    let result = app.handle_net_event(NetEvent::Received("general".to_string(), message));
    assert!(result.is_none());
}

#[test]
fn handle_net_event_returns_none_for_non_received_events() {
    let mut app = app();
    let result = app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    assert!(result.is_none());
}

#[test]
fn load_history_seeds_transcript_and_dedupes_like_live_messages() {
    let mut app = app();
    let message = compose_message(PEER_ID, "from before restart", None);
    app.load_history("general", vec![message.clone()]);

    assert_eq!(app.active().messages.len(), 1);
    assert_eq!(as_chat(app.active().messages.back().unwrap()), &message);

    // A live redelivery of the same id must still be deduped.
    let before = app.active().messages.len();
    app.handle_net_event(NetEvent::Received("general".to_string(), message));
    assert_eq!(app.active().messages.len(), before);
}

#[test]
fn load_history_on_an_unjoined_channel_is_a_no_op() {
    let mut app = app();
    app.load_history("nonexistent", vec![compose_message(PEER_ID, "hi", None)]);
    assert_eq!(app.active().messages.len(), 0);
}

#[test]
fn an_old_file_share_stays_listed_and_saveable_after_scrollback_trims_it() {
    let mut app = app();
    let old = file_message(1, PEER_ID, "old.pdf", b"old");
    let hash = old.attachment.as_ref().unwrap().hash;
    let mut history = vec![old];
    for id in 2..=MAX_SCROLLBACK as u64 + 1 {
        history.push(compose_message(SELF_ID, &format!("message {id}"), None));
    }
    app.load_history("general", history);

    assert_eq!(app.active().messages.len(), MAX_SCROLLBACK);
    open_files(&mut app);
    assert_eq!(listed(&app), vec!["old.pdf"]);
    app.handle_key(key(KeyCode::Esc));

    let (channel, got, filename, providers, _) =
        save_fields(submit(&mut app, &format!("/save {}", &hash.to_hex()[..8])));
    assert_eq!(
        (channel, got, filename),
        ("general".into(), hash, "old.pdf".into())
    );
    assert_eq!(providers, vec![PEER_ID]);
}

fn dated_message(ts_unix_ms: u64, id: u64, sender: [u8; 32], text: &str) -> ChatMessage {
    ChatMessage {
        v: 1,
        id,
        sender,
        ts_unix_ms,
        text: text.to_string(),
        attachment: None,
        reply_to: None,
    }
}

fn reply_message(
    ts_unix_ms: u64,
    id: u64,
    sender: [u8; 32],
    text: &str,
    reply_to: u64,
) -> ChatMessage {
    ChatMessage {
        reply_to: Some(reply_to),
        ..dated_message(ts_unix_ms, id, sender, text)
    }
}

fn file_message(id: u64, sender: [u8; 32], filename: &str, content: &[u8]) -> ChatMessage {
    ChatMessage {
        v: 2,
        id,
        sender,
        ts_unix_ms: id,
        text: String::new(),
        attachment: Some(crate::message::FileAttachment {
            filename: filename.to_string(),
            size: content.len() as u64,
            hash: iroh_blobs::Hash::new(content),
        }),
        reply_to: None,
    }
}

#[test]
fn merge_history_dedupes_against_already_known_messages() {
    let mut app = app();
    let known = dated_message(100, 1, PEER_ID, "known");
    app.load_history("general", vec![known.clone()]);

    let new_message = dated_message(200, 2, PEER_ID, "new");
    let accepted = app.merge_history("general", vec![known, new_message.clone()]);

    assert_eq!(accepted, vec![new_message]);
    assert_eq!(app.active().messages.len(), 2);
}

#[test]
fn merge_history_sorts_backfilled_messages_by_timestamp_not_append_order() {
    let mut app = app();
    let recent = dated_message(200, 1, PEER_ID, "recent");
    app.load_history("general", vec![recent.clone()]);

    let older = dated_message(100, 2, PEER_ID, "older");
    app.merge_history("general", vec![older.clone()]);

    let messages: Vec<&ChatMessage> = app.active().messages.iter().map(as_chat).collect();
    assert_eq!(
        messages,
        vec![&older, &recent],
        "an older backfilled message must sort before what was already there"
    );
}

#[test]
fn merge_history_returns_nothing_when_everything_is_already_known() {
    let mut app = app();
    let message = dated_message(100, 1, PEER_ID, "hi");
    app.load_history("general", vec![message.clone()]);

    let accepted = app.merge_history("general", vec![message]);
    assert!(accepted.is_empty());
}

#[test]
fn merge_history_on_an_unjoined_channel_is_a_no_op() {
    let mut app = app();
    let accepted = app.merge_history("nonexistent", vec![dated_message(100, 1, PEER_ID, "hi")]);
    assert!(accepted.is_empty());
    assert_eq!(app.active().messages.len(), 0);
}

#[test]
fn merge_history_keeps_system_lines_after_the_resorted_chat_messages() {
    let mut app = app();
    app.push_system("earlier notice");
    let older = dated_message(100, 1, PEER_ID, "older");
    app.merge_history("general", vec![older.clone()]);

    let lines: Vec<&TranscriptLine> = app.active().messages.iter().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(as_chat(lines[0]), &older);
    assert_eq!(as_system(lines[1]), "earlier notice");
}

#[test]
fn merge_history_enforces_the_scrollback_cap_evicting_the_oldest_first() {
    let mut app = app();
    let batch: Vec<ChatMessage> = (0..(MAX_SCROLLBACK as u64 + 10))
        .map(|i| dated_message(i, i, PEER_ID, "msg"))
        .collect();

    let accepted = app.merge_history("general", batch);

    assert_eq!(accepted.len(), MAX_SCROLLBACK + 10);
    assert_eq!(app.active().messages.len(), MAX_SCROLLBACK);
    let oldest_remaining = as_chat(app.active().messages.front().unwrap());
    assert_eq!(
        oldest_remaining.id, 10,
        "the 10 oldest messages must have been evicted to stay within the cap"
    );
}

#[test]
fn slash_search_without_arg_and_no_active_filter_shows_usage() {
    let mut app = app();
    let action = submit(&mut app, "/search");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"), "got: {last}");
}

#[test]
fn slash_search_finds_matches_in_loaded_history_and_returns_an_action() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hello world", None),
    ));
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "goodbye", None),
    ));

    let action = submit(&mut app, "/search world");

    match action.expect("a successful /search returns an action") {
        InputAction::Search { channel, term } => {
            assert_eq!(channel, "general");
            assert_eq!(term, "world");
        }
        _ => panic!("expected InputAction::Search"),
    }
    let search = app
        .active()
        .search
        .as_ref()
        .expect("search should be active");
    assert_eq!(search.messages.len(), 1);
    assert_eq!(search.messages[0].text, "hello world");
    assert!(search.pending, "the on-disk scan hasn't reported back yet");
}

#[test]
fn slash_search_is_case_insensitive() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "Hello World", None),
    ));

    submit(&mut app, "/search WORLD");

    let search = app
        .active()
        .search
        .as_ref()
        .expect("search should be active");
    assert_eq!(search.messages.len(), 1);
}

#[test]
fn slash_search_with_no_matches_still_activates_an_empty_search() {
    let mut app = app();
    let action = submit(&mut app, "/search nonexistent");
    assert!(action.is_some());
    let search = app
        .active()
        .search
        .as_ref()
        .expect("search should be active");
    assert!(search.messages.is_empty());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("0 match"), "got: {last}");
}

#[test]
fn slash_search_with_no_arg_clears_an_active_filter() {
    let mut app = app();
    submit(&mut app, "/search hello");
    assert!(app.active().search.is_some());

    let action = submit(&mut app, "/search");

    assert!(action.is_none());
    assert!(app.active().search.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "search cleared");
}

#[test]
fn apply_search_outcome_merges_on_disk_matches_into_existing_results() {
    let mut app = app();
    submit(&mut app, "/search hello");
    let older = dated_message(50, 99, PEER_ID, "hello from the past");

    app.apply_search_outcome(SearchOutcome {
        channel: "general".to_string(),
        term: "hello".to_string(),
        messages: vec![older.clone()],
    });

    let search = app.active().search.as_ref().unwrap();
    assert_eq!(search.messages, vec![older]);
    assert!(!search.pending);
}

#[test]
fn apply_search_outcome_deduplicates_messages_already_in_the_snapshot() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hello there", None),
    ));
    let action = submit(&mut app, "/search hello");
    let term = match action.expect("search returns an action") {
        InputAction::Search { term, .. } => term,
        _ => panic!("expected InputAction::Search"),
    };
    let already_shown = app.active().search.as_ref().unwrap().messages[0].clone();

    app.apply_search_outcome(SearchOutcome {
        channel: "general".to_string(),
        term,
        messages: vec![already_shown.clone()],
    });

    let search = app.active().search.as_ref().unwrap();
    assert_eq!(search.messages, vec![already_shown]);
}

#[test]
fn apply_search_outcome_ignores_a_stale_result_from_a_superseded_search() {
    let mut app = app();
    submit(&mut app, "/search first");
    submit(&mut app, "/search second");

    app.apply_search_outcome(SearchOutcome {
        channel: "general".to_string(),
        term: "first".to_string(),
        messages: vec![dated_message(1, 1, PEER_ID, "first match")],
    });

    let search = app.active().search.as_ref().unwrap();
    assert_eq!(search.term, "second");
    assert!(search.messages.is_empty());
}

#[test]
fn apply_search_outcome_is_a_no_op_for_an_unknown_channel() {
    let mut app = app();
    app.apply_search_outcome(SearchOutcome {
        channel: "nonexistent".to_string(),
        term: "hello".to_string(),
        messages: vec![dated_message(1, 1, PEER_ID, "hello")],
    });
    assert!(app.active().search.is_none());
}

#[test]
fn slash_s_is_a_shorthand_for_search() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hello world", None),
    ));

    let action = submit(&mut app, "/s world");

    match action.expect("a successful /s returns an action") {
        InputAction::Search { channel, term } => {
            assert_eq!(channel, "general");
            assert_eq!(term, "world");
        }
        _ => panic!("expected InputAction::Search"),
    }
    let search = app
        .active()
        .search
        .as_ref()
        .expect("search should be active");
    assert_eq!(search.messages.len(), 1);
}

#[test]
fn slash_s_with_no_arg_clears_an_active_filter() {
    let mut app = app();
    submit(&mut app, "/search hello");
    assert!(app.active().search.is_some());

    let action = submit(&mut app, "/s");

    assert!(action.is_none());
    assert!(app.active().search.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "search cleared");
}

#[test]
fn slash_send_with_arg_returns_send_file_action_and_pushes_a_breadcrumb() {
    let mut app = app();
    let action = submit(&mut app, "/send /tmp/report.pdf");
    match action.expect("non-empty /send returns an action") {
        InputAction::SendFile { channel, path } => {
            assert_eq!(channel, "general");
            assert_eq!(path, "/tmp/report.pdf");
        }
        _ => panic!("expected InputAction::SendFile"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "sending report.pdf...");
}

#[test]
fn slash_send_strips_surrounding_double_quotes() {
    let mut app = app();
    let action = submit(&mut app, "/send \"~/Downloads/tilemap.png\"");
    match action.expect("non-empty /send returns an action") {
        InputAction::SendFile { path, .. } => {
            assert_eq!(path, "~/Downloads/tilemap.png");
        }
        _ => panic!("expected InputAction::SendFile"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "sending tilemap.png...");
}

#[test]
fn strip_quotes_strips_matching_double_or_single_quotes() {
    assert_eq!(strip_quotes("\"foo\""), "foo");
    assert_eq!(strip_quotes("'foo'"), "foo");
}

#[test]
fn strip_quotes_leaves_unquoted_or_mismatched_input_alone() {
    assert_eq!(strip_quotes("foo"), "foo");
    assert_eq!(strip_quotes("\"foo"), "\"foo");
    assert_eq!(strip_quotes("'foo\""), "'foo\"");
    assert_eq!(strip_quotes("\""), "\"");
}

#[test]
fn slash_save_resolves_a_known_attachment_by_hash_prefix() {
    let mut app = app();
    let message = file_message(1, PEER_ID, "report.pdf", b"report contents");
    let hash = message.attachment.as_ref().unwrap().hash;
    app.handle_net_event(NetEvent::Received("general".to_string(), message));

    let full_hash = hash.to_hex();
    let action = submit(&mut app, &format!("/save {}", &full_hash[..8]));

    match action.expect("a successful /save returns an action") {
        InputAction::SaveFile(request) => {
            assert_eq!(request.hash, hash);
            assert_eq!(request.filename, "report.pdf");
            assert_eq!(request.providers, vec![PEER_ID]);
            assert_eq!(
                request.mode,
                SaveMode::Manual,
                "someone asked for it and can see it: no limit on its size"
            );
        }
        _ => panic!("expected InputAction::SaveFile"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "saving report.pdf...");
}

#[test]
fn slash_save_reports_an_error_for_an_unknown_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/save ffffffff");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no known file"), "got: {last}");
}

#[test]
fn slash_save_rejects_a_non_hex_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/save not-hex");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("invalid hash prefix"), "got: {last}");
}

#[test]
fn resolve_attachment_ignores_duplicate_sends_of_the_same_file() {
    let mut app = app();
    let message = file_message(1, PEER_ID, "report.pdf", b"same bytes");
    let hash = message.attachment.as_ref().unwrap().hash;
    // The same file, resent (e.g. to two different channels) -- must
    // not turn into an "ambiguous prefix" error just because it shows
    // up twice.
    app.handle_net_event(NetEvent::Received("general".to_string(), message));
    let resent = file_message(2, PEER_ID_2, "report.pdf", b"same bytes");
    app.handle_net_event(NetEvent::Received("general".to_string(), resent));

    let resolved = app.resolve_attachment(&hash.to_hex()[..8]).unwrap();

    assert_eq!(resolved.0, hash);
}

/// Delivers `message` to `channel` as if a peer had just sent it.
fn receive(app: &mut AppState, channel: &str, message: ChatMessage) {
    app.handle_net_event(NetEvent::Received(channel.to_string(), message));
}

/// What a `/save` asked to fetch, in full -- the hash, filename, and
/// providers of its `InputAction::SaveFile` -- panicking on anything
/// else.
fn save_request(action: Option<InputAction>) -> (Hash, String, Vec<[u8; 32]>) {
    match action.expect("a /save with a file to save returns an action") {
        InputAction::SaveFile(request) => (request.hash, request.filename, request.providers),
        _ => panic!("expected InputAction::SaveFile"),
    }
}

/// The folder a `/save` asked to save into -- panicking on anything
/// else.
fn save_dir(action: Option<InputAction>) -> PathBuf {
    match action.expect("a /save with a file to save returns an action") {
        InputAction::SaveFile(request) => request.dir,
        _ => panic!("expected InputAction::SaveFile"),
    }
}

/// Which file a `/save` chose: its hash, its filename, and the sender --
/// the first provider, the one asked before anyone else.
fn saved_file(action: Option<InputAction>) -> (Hash, String, [u8; 32]) {
    let (hash, filename, providers) = save_request(action);
    (hash, filename, providers[0])
}

/// Delivers a gossip neighbor coming up on `channel` for each of `peers`,
/// in order.
fn peers_join(app: &mut AppState, channel: &str, peers: &[[u8; 32]]) {
    for peer in peers {
        app.handle_net_event(NetEvent::PeerJoined(channel.to_string(), *peer));
    }
}

#[test]
fn bare_slash_save_saves_the_newest_file_from_another_peer() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "old.pdf", b"old contents"),
    );
    let newest = file_message(2, PEER_ID_2, "new.pdf", b"new contents");
    let newest_hash = newest.attachment.as_ref().unwrap().hash;
    receive(&mut app, "general", newest);

    let saved = saved_file(submit(&mut app, "/save"));

    assert_eq!(saved, (newest_hash, "new.pdf".to_string(), PEER_ID_2));
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "saving new.pdf...");
}

#[test]
fn bare_slash_save_skips_our_own_files_even_when_they_are_newer() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "theirs.pdf", b"theirs"),
    );
    app.record_sent_message("general", file_message(2, SELF_ID, "mine.pdf", b"mine"));

    let (_, filename, sender) = saved_file(submit(&mut app, "/save"));

    assert_eq!((filename.as_str(), sender), ("theirs.pdf", PEER_ID));
}

#[test]
fn bare_slash_save_looks_past_newer_lines_that_carry_no_file() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID_2, "got it, thanks", None),
    );
    app.push_system("some notice");

    let (_, filename, _) = saved_file(submit(&mut app, "/save"));

    assert_eq!(filename, "report.pdf");
}

#[test]
fn bare_slash_save_is_not_narrowed_by_an_open_search() {
    // A search narrows what `/reply` can target to what is in view; the
    // file just shared belongs to the channel, not to the view.
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID_2, "hello there", None),
    );
    submit(&mut app, "/search hello");
    assert_eq!(
        app.active().visible_chat_ids().len(),
        1,
        "the search leaves only the greeting in view"
    );

    let (_, filename, _) = saved_file(submit(&mut app, "/save"));

    assert_eq!(filename, "report.pdf");
}

#[test]
fn bare_slash_save_only_reads_the_active_channel() {
    let mut app = multi_channel_app(); // "random" is active
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "here.pdf", b"here"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "elsewhere.pdf", b"elsewhere"),
    );

    let (_, filename, sender) = saved_file(submit(&mut app, "/save"));

    assert_eq!((filename.as_str(), sender), ("here.pdf", PEER_ID));
}

#[test]
fn bare_slash_save_saves_a_file_from_one_of_our_own_other_devices() {
    // Like `ui::attachment_line`, a bare `/save` counts anyone but this
    // device as another peer: a paired sibling's file is not on this one.
    let mut app = app();
    let user_key = SecretKey::generate();
    app.load_self_user_id(*user_key.public().as_bytes());
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID),
    ));
    assert_eq!(app.display_name(&PEER_ID), "you", "PEER_ID is our sibling");
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "from-the-other-laptop.png", b"png"),
    );

    let (_, filename, sender) = saved_file(submit(&mut app, "/save"));

    assert_eq!(
        (filename.as_str(), sender),
        ("from-the-other-laptop.png", PEER_ID)
    );
}

#[test]
fn bare_slash_save_says_so_when_the_channel_has_no_file() {
    let mut app = app();
    receive(&mut app, "general", compose_message(PEER_ID, "hello", None));

    let action = submit(&mut app, "/save");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no file from another peer"), "got: {last}");
}

#[test]
fn bare_slash_save_says_so_when_only_our_own_files_are_there() {
    let mut app = app();
    app.record_sent_message("general", file_message(1, SELF_ID, "mine.pdf", b"mine"));

    let action = submit(&mut app, "/save");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no file from another peer"), "got: {last}");
}

#[test]
fn bare_slash_save_says_so_when_files_were_only_shared_in_other_channels() {
    let mut app = multi_channel_app(); // "random" is active
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "elsewhere.pdf", b"elsewhere"),
    );

    let action = submit(&mut app, "/save");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no file from another peer"), "got: {last}");
}

#[test]
fn slash_save_with_a_prefix_still_reaches_an_older_file() {
    let mut app = app();
    let older = file_message(1, PEER_ID, "old.pdf", b"old contents");
    let older_hash = older.attachment.as_ref().unwrap().hash;
    receive(&mut app, "general", older);
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "new.pdf", b"new contents"),
    );

    let saved = saved_file(submit(
        &mut app,
        &format!("/save {}", &older_hash.to_hex()[..8]),
    ));

    assert_eq!(saved, (older_hash, "old.pdf".to_string(), PEER_ID));
}

#[test]
fn save_asks_the_sender_first_then_online_peers_then_those_who_only_posted() {
    let mut app = app();
    // A poster who spoke before the file was shared still comes after a
    // peer who is online now: someone reachable beats someone who may
    // be long gone.
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID_3, "morning", None),
    );
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    peers_join(&mut app, "general", &[PEER_ID_2]);

    let (_, _, providers) = save_request(submit(&mut app, "/save"));

    assert_eq!(providers, vec![PEER_ID, PEER_ID_2, PEER_ID_3]);
}

#[test]
fn save_asks_each_peer_once() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID_2, "thanks", None),
    );
    // Both are online *and* posted, and the sender is among them.
    peers_join(&mut app, "general", &[PEER_ID_2, PEER_ID]);

    let (_, _, providers) = save_request(submit(&mut app, "/save"));

    assert_eq!(providers, vec![PEER_ID, PEER_ID_2]);
}

#[test]
fn save_never_asks_this_device_but_does_ask_a_paired_sibling() {
    let mut app = app();
    let user_key = SecretKey::generate();
    app.load_self_user_id(*user_key.public().as_bytes());
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(&user_key, PEER_ID_2),
    ));
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    app.record_sent_message("general", compose_message(SELF_ID, "got it", None));
    peers_join(&mut app, "general", &[PEER_ID_2]);

    let (_, _, providers) = save_request(submit(&mut app, "/save"));

    assert_eq!(
        providers,
        vec![PEER_ID, PEER_ID_2],
        "our other device may hold the file; this one is never asked"
    );
}

#[test]
fn save_of_our_own_file_still_lists_this_device_first_since_it_holds_the_file() {
    // Left out of everyone else's list, but as the sender it stays: the
    // file is already in our own blob store, so it needs no one.
    let mut app = app();
    let mine = file_message(1, SELF_ID, "mine.pdf", b"mine");
    let hash = mine.attachment.as_ref().unwrap().hash;
    app.record_sent_message("general", mine);
    peers_join(&mut app, "general", &[PEER_ID]);

    let (_, _, providers) =
        save_request(submit(&mut app, &format!("/save {}", &hash.to_hex()[..8])));

    assert_eq!(providers, vec![SELF_ID, PEER_ID]);
}

#[test]
fn save_only_asks_peers_of_the_channels_that_carried_the_file() {
    let mut app = multi_channel_app(); // "random" is active
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "here.pdf", b"here"),
    );
    peers_join(&mut app, "random", &[PEER_ID_3]);
    // Someone who is only in #general, where the file was never shared.
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID_2, "hello", None),
    );
    peers_join(&mut app, "general", &[PEER_ID_2]);

    let (_, _, providers) = save_request(submit(&mut app, "/save"));

    assert_eq!(providers, vec![PEER_ID, PEER_ID_3]);
}

#[test]
fn save_asks_every_channel_the_file_was_shared_in() {
    let mut app = multi_channel_app(); // "random" is active
    let content = b"same bytes";
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", content),
    );
    receive(
        &mut app,
        "random",
        file_message(2, PEER_ID_2, "report.pdf", content),
    );
    peers_join(&mut app, "general", &[PEER_ID_3]);
    let prefix = &iroh_blobs::Hash::new(content).to_hex()[..8];

    // Resolved by prefix, the same answer whichever channel is active:
    // the first sender seen, then the peer online in #general, then the
    // one who only posted in #random.
    let (_, _, providers) = save_request(submit(&mut app, &format!("/save {prefix}")));

    assert_eq!(providers, vec![PEER_ID, PEER_ID_3, PEER_ID_2]);
}

#[test]
fn save_names_the_channel_it_saves_for_so_the_copy_can_be_recorded_for_it() {
    let mut app = multi_channel_app(); // "random" is active
    let elsewhere = file_message(1, PEER_ID, "elsewhere.pdf", b"elsewhere");
    let prefix = elsewhere.attachment.as_ref().unwrap().hash.to_hex()[..8].to_string();
    receive(&mut app, "general", elsewhere);
    receive(
        &mut app,
        "random",
        file_message(2, PEER_ID, "here.pdf", b"here"),
    );

    let bare = submit(&mut app, "/save");
    let by_prefix = submit(&mut app, &format!("/save {prefix}"));

    let channel_of = |action: Option<InputAction>| match action.expect("saves a file") {
        InputAction::SaveFile(request) => request.channel,
        _ => panic!("expected InputAction::SaveFile"),
    };
    assert_eq!(channel_of(bare), "random");
    assert_eq!(
        channel_of(by_prefix),
        "general",
        "the channel the file was shared in, as its folder is"
    );
}

#[test]
fn save_goes_into_the_folder_of_the_channel_the_file_was_shared_in() {
    let mut app = multi_channel_app(); // "random" is active
    app.load_files_root(PathBuf::from("/files"));
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "here.pdf", b"here"),
    );

    let dir = save_dir(submit(&mut app, "/save"));

    assert_eq!(dir, PathBuf::from("/files/random"));
}

#[test]
fn save_by_prefix_goes_into_the_folder_of_the_channel_that_carried_the_file() {
    let mut app = multi_channel_app(); // "random" is active
    app.load_files_root(PathBuf::from("/files"));
    let message = file_message(1, PEER_ID, "elsewhere.pdf", b"elsewhere");
    let prefix = message.attachment.as_ref().unwrap().hash.to_hex()[..8].to_string();
    receive(&mut app, "general", message);

    let dir = save_dir(submit(&mut app, &format!("/save {prefix}")));

    assert_eq!(
        dir,
        PathBuf::from("/files/general"),
        "not the active channel's, which never had the file"
    );
}

#[test]
fn a_file_shared_in_two_channels_is_saved_where_you_are_looking_when_it_can_be() {
    let mut app = multi_channel_app(); // "random" is active, "general" joined first
    app.load_files_root(PathBuf::from("/files"));
    let content = b"same bytes";
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", content),
    );
    receive(
        &mut app,
        "random",
        file_message(2, PEER_ID_2, "report.pdf", content),
    );
    let prefix = &iroh_blobs::Hash::new(content).to_hex()[..8];

    let from_random = save_dir(submit(&mut app, &format!("/save {prefix}")));
    app.active = 0; // general
    let from_general = save_dir(submit(&mut app, &format!("/save {prefix}")));

    assert_eq!(from_random, PathBuf::from("/files/random"));
    assert_eq!(from_general, PathBuf::from("/files/general"));
}

#[test]
fn a_file_shared_in_two_other_channels_goes_to_the_first_one_joined() {
    let mut app = AppState::new(
        SELF_ID,
        vec![
            "general".to_string(),
            "random".to_string(),
            "other".to_string(),
        ],
        "other",
    );
    app.load_files_root(PathBuf::from("/files"));
    let content = b"same bytes";
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "report.pdf", content),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "report.pdf", content),
    );
    let prefix = &iroh_blobs::Hash::new(content).to_hex()[..8];

    let dir = save_dir(submit(&mut app, &format!("/save {prefix}")));

    assert_eq!(dir, PathBuf::from("/files/general"));
}

#[test]
fn renaming_a_channel_does_not_move_its_folder() {
    let mut app = app_for_the_switcher();
    app.load_files_root(PathBuf::from("/files"));
    app.active = 2; // project-x
    receive(
        &mut app,
        "project-x",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    let before = save_dir(submit(&mut app, "/save"));

    submit(&mut app, "/rename proj");
    let after = save_dir(submit(&mut app, "/save"));

    assert_eq!(app.channel_label("project-x"), "proj");
    assert_eq!(before, PathBuf::from("/files/project-x"));
    assert_eq!(after, before, "the label changed, the stored name did not");
}

#[test]
fn aliasing_a_peer_does_not_move_the_folder_of_a_conversation_with_them() {
    let mut app = app_for_the_switcher(); // `Alice` labels the DM `dm-Alice`
    app.load_files_root(PathBuf::from("/files"));
    app.active = 1;
    receive(
        &mut app,
        "dm-Alice",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    let before = save_dir(submit(&mut app, "/save"));

    submit(
        &mut app,
        &format!("/alias {} Carol", &hex_id(&PEER_ID)[..8]),
    );
    let after = save_dir(submit(&mut app, "/save"));

    assert_eq!(app.channel_label("dm-Alice"), "Carol");
    assert_eq!(before, channel_dir(Path::new("/files"), "dm-Alice"));
    assert_eq!(after, before, "the label changed, the stored name did not");
}

#[test]
fn a_dm_and_a_dm_chat_save_into_folders_named_from_their_stored_names() {
    let mut app = app_for_the_switcher(); // `dm-Alice` and `p2p-08080808`
    app.load_files_root(PathBuf::from("/files"));
    receive(
        &mut app,
        "dm-Alice",
        file_message(1, PEER_ID, "a.pdf", b"from alice"),
    );
    receive(
        &mut app,
        "p2p-08080808",
        file_message(2, PEER_ID_2, "b.pdf", b"from bob"),
    );

    app.active = 1;
    let dm = save_dir(submit(&mut app, "/save"));
    app.active = 3;
    let chat = save_dir(submit(&mut app, "/save"));

    assert_eq!(dm, channel_dir(Path::new("/files"), "dm-Alice"));
    assert_eq!(chat, PathBuf::from("/files/p2p-08080808"));
    assert_ne!(dm, chat);
}

#[test]
fn a_direct_folder_mapping_controls_saves_without_following_later_aliases() {
    let mut app = app_for_the_switcher();
    app.load_files_root(PathBuf::from("/files"));
    app.load_direct_folder("p2p-08080808".into(), "dm-bob-08080808".into());
    app.active = 3;
    receive(
        &mut app,
        "p2p-08080808",
        file_message(1, PEER_ID_2, "report.pdf", b"report"),
    );

    let before = save_dir(submit(&mut app, "/save"));
    submit(
        &mut app,
        &format!("/alias {} Carol", &hex_id(&PEER_ID_2)[..8]),
    );
    let after = save_dir(submit(&mut app, "/save"));

    assert_eq!(before, PathBuf::from("/files/dm-bob-08080808"));
    assert_eq!(after, before);
}

#[test]
fn sync_status_reports_a_mapped_direct_folder() {
    let mut app = app_for_the_switcher();
    app.load_files_root(PathBuf::from("/files"));
    app.load_direct_folder("p2p-08080808".into(), "dm-bob-08080808".into());
    app.active = 3;
    submit(&mut app, "/sync on");

    submit(&mut app, "/sync");

    assert!(as_system(app.active().messages.back().unwrap()).contains("/files/dm-bob-08080808"));
}

#[test]
fn a_bare_folder_says_where_received_files_go() {
    let mut app = app_for_the_switcher();
    app.load_files_root(PathBuf::from("/files"));
    app.active = 2; // project-x
    submit(&mut app, "/rename proj");

    let action = submit(&mut app, "/folder");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("#proj"), "names the channel, got: {last}");
    assert!(
        last.contains("/files/project-x"),
        "the channel's folder is the stored name's, got: {last}"
    );
    assert!(last.contains("under /files,"), "and the root, got: {last}");
}

#[test]
fn folder_with_a_path_asks_to_move_the_files_root_and_strips_one_layer_of_quotes() {
    let mut app = app();

    for (typed, expected) in [
        ("~/Documents/leyline", "~/Documents/leyline"),
        ("\"/my files\"", "/my files"),
        ("  relative/dir  ", "relative/dir"),
    ] {
        match submit(&mut app, &format!("/folder {typed}")) {
            Some(InputAction::SetFilesRoot(path)) => assert_eq!(path, expected, "{typed}"),
            _ => panic!("expected InputAction::SetFilesRoot for {typed}"),
        }
    }
}

#[test]
fn a_files_root_that_took_effect_decides_where_the_next_save_goes() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    app.load_files_root(PathBuf::from("/old"));
    assert_eq!(
        save_dir(submit(&mut app, "/save")),
        PathBuf::from("/old/general")
    );

    app.load_files_root(PathBuf::from("/new"));

    assert_eq!(
        save_dir(submit(&mut app, "/save")),
        PathBuf::from("/new/general")
    );
}

#[test]
fn a_bare_maxsize_says_what_the_limit_is_and_how_to_change_it() {
    let mut app = app();

    let action = submit(&mut app, "/maxsize");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.contains("files over 500.0 MB are refused (the default)"),
        "got: {last}"
    );
    assert!(
        last.contains("/maxsize <size>") && last.contains("/maxsize off"),
        "got: {last}"
    );
}

#[test]
fn autosave_limit_changes_the_automatic_request_and_refuses_off() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());

    match submit(&mut app, "/autosave limit 1KB") {
        Some(InputAction::SetChannelAutoSaveLimit {
            channel,
            limit: Some(AutoSaveLimit::Bytes(1024)),
        }) if channel == "general" => {}
        _ => panic!("expected an auto-save limit action"),
    }
    assert!(
        arrive(
            &mut app,
            "general",
            file_message_of_size(1, PEER_ID, "over.bin", 1025)
        )
        .is_none()
    );

    assert!(submit(&mut app, "/autosave limit off").is_none());
    assert!(newest_notice(&app, "general").contains("auto-save limit"));
}

#[test]
fn a_bare_maxsize_reports_a_chosen_limit_and_no_limit_too_without_changing_either() {
    let mut app = app();

    app.load_size_limit(SizeLimit::Bytes(1 << 20));
    assert!(submit(&mut app, "/maxsize").is_none());
    let chosen = as_system(app.active().messages.back().unwrap()).to_string();
    assert_eq!(app.size_limit(), SizeLimit::Bytes(1 << 20));

    app.load_size_limit(SizeLimit::Off);
    assert!(submit(&mut app, "/maxsize").is_none());
    let off = as_system(app.active().messages.back().unwrap()).to_string();
    assert_eq!(app.size_limit(), SizeLimit::Off);

    assert!(
        chosen.contains("files over 1.0 MB are refused (set with /maxsize)"),
        "got: {chosen}"
    );
    assert!(
        chosen.contains("/maxsize default restores the 500.0 MB default"),
        "got: {chosen}"
    );
    assert!(
        off.contains("there is no limit on the size of a file you share"),
        "got: {off}"
    );
}

#[test]
fn maxsize_with_a_size_applies_it_at_once_and_asks_for_it_to_be_remembered() {
    let mut app = app();

    match submit(&mut app, "/maxsize 2GB").expect("a limit that took effect is remembered") {
        InputAction::SetSizeLimit(limit) => assert_eq!(limit, SizeLimit::Bytes(2 << 30)),
        _ => panic!("expected InputAction::SetSizeLimit"),
    }

    assert_eq!(app.size_limit(), SizeLimit::Bytes(2 << 30));
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.contains("files over 2.0 GB are refused (set with /maxsize)"),
        "got: {last}"
    );
}

#[test]
fn maxsize_off_lifts_the_limit_and_default_restores_it() {
    let mut app = app();

    match submit(&mut app, "/maxsize off") {
        Some(InputAction::SetSizeLimit(limit)) => assert_eq!(limit, SizeLimit::Off),
        _ => panic!("expected InputAction::SetSizeLimit"),
    }
    assert_eq!(app.size_limit(), SizeLimit::Off);
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("there is no limit"), "got: {last}");

    match submit(&mut app, "/maxsize default") {
        Some(InputAction::SetSizeLimit(limit)) => assert_eq!(limit, SizeLimit::Default),
        _ => panic!("expected InputAction::SetSizeLimit"),
    }
    assert_eq!(app.size_limit(), SizeLimit::Default);
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("(the default)"), "got: {last}");
}

#[test]
fn a_maxsize_that_is_not_a_size_is_refused_and_changes_nothing() {
    let mut app = app();
    app.load_size_limit(SizeLimit::Bytes(1 << 20));

    for typed in ["banana", "5", "0MB", "2XB"] {
        let action = submit(&mut app, &format!("/maxsize {typed}"));

        assert!(action.is_none(), "{typed}: nothing to remember");
        assert_eq!(
            app.size_limit(),
            SizeLimit::Bytes(1 << 20),
            "{typed}: unchanged"
        );
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains(&format!("'{typed}'")), "{typed}: got {last}");
    }
}

#[test]
fn the_size_limit_is_the_default_until_one_is_loaded() {
    let mut app = app();
    assert_eq!(app.size_limit(), SizeLimit::Default);

    app.load_size_limit(SizeLimit::Off);

    assert_eq!(app.size_limit(), SizeLimit::Off);
}

#[test]
fn help_lists_maxsize() {
    let mut app = app();

    submit(&mut app, "/help");

    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("/maxsize"), "got: {last}");
}

#[test]
fn record_sent_message_places_the_message_without_marking_unread() {
    let mut app = multi_channel_app();
    app.active = 1; // "random" is active; "general" is not
    let message = file_message(1, SELF_ID, "report.pdf", b"contents");

    app.record_sent_message("general", message.clone());

    assert_eq!(as_chat(app.channels[0].messages.back().unwrap()), &message);
    assert!(
        app.channels[0].unread_count == 0,
        "a self-sent file shouldn't flag its channel unread"
    );
}

#[test]
fn record_sent_message_dedupes_by_id() {
    let mut app = app();
    let message = file_message(1, SELF_ID, "report.pdf", b"contents");
    app.record_sent_message("general", message.clone());

    let before = app.active().messages.len();
    app.record_sent_message("general", message);

    assert_eq!(app.active().messages.len(), before);
}

#[test]
fn file_send_failed_and_save_failed_are_a_no_op_here() {
    // Both are handled in main.rs instead (which also logs them via
    // tracing::warn!, unlike a plain push_system) -- see
    // handle_net_event's doc comment.
    let mut app = app();
    let before = app.active().messages.len();

    app.handle_net_event(NetEvent::FileSendFailed {
        path: "/tmp/missing.pdf".to_string(),
        error: "no such file".to_string(),
    });
    app.handle_net_event(NetEvent::FileSaveFailed {
        channel: "general".to_string(),
        hash: iroh_blobs::Hash::new(b"report"),
        filename: "report.pdf".to_string(),
        error: "boom".to_string(),
        mode: SaveMode::Manual,
    });

    assert_eq!(app.active().messages.len(), before);
}

#[test]
fn file_saved_pushes_a_system_notice_with_the_final_path() {
    let mut app = app();
    app.handle_net_event(NetEvent::FileSaved {
        channel: "general".to_string(),
        hash: iroh_blobs::Hash::new(b"report"),
        filename: "report.pdf".to_string(),
        path: std::path::PathBuf::from("/home/alice/Downloads/leyline/report.pdf"),
        mode: SaveMode::Manual,
    });
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(
        last,
        "saved report.pdf to /home/alice/Downloads/leyline/report.pdf"
    );
}

#[test]
fn search_matches_an_attachments_filename() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        file_message(1, PEER_ID, "vacation-photo.png", b"bytes"),
    ));

    submit(&mut app, "/search vacation");

    let search = app
        .active()
        .search
        .as_ref()
        .expect("search should be active");
    assert_eq!(search.messages.len(), 1);
}

fn ctrl_r(app: &mut AppState) -> Option<InputAction> {
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
}

#[test]
fn find_message_resolves_a_message_still_in_the_transcript() {
    let mut app = app();
    let parent = compose_message(PEER_ID, "original", None);
    let parent_id = parent.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), parent));
    let reply = reply_message(1, 999, PEER_ID_2, "sounds good", parent_id);
    app.handle_net_event(NetEvent::Received("general".to_string(), reply));

    let found = app
        .active()
        .find_message(parent_id)
        .expect("parent must resolve");
    assert_eq!(found.text, "original");
}

#[test]
fn find_message_returns_none_for_an_unknown_id() {
    let app = app();
    assert!(app.active().find_message(12345).is_none());
}

#[test]
fn ctrl_r_with_no_messages_pushes_a_notice_and_does_not_enter_picking() {
    let mut app = app();
    ctrl_r(&mut app);
    assert!(app.picking_reply.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "no messages to reply to");
}

#[test]
fn ctrl_r_selects_the_newest_message() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "first", None),
    ));
    let newest = compose_message(PEER_ID, "second", None);
    let newest_id = newest.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), newest));

    ctrl_r(&mut app);

    assert_eq!(app.picking_reply, Some(newest_id));
}

#[test]
fn ctrl_r_picks_from_active_search_results_when_a_search_is_running() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "apple", None),
    ));
    let matching = compose_message(PEER_ID, "banana split", None);
    let matching_id = matching.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), matching));
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "cherry", None),
    ));

    submit(&mut app, "/search banana");
    ctrl_r(&mut app);

    assert_eq!(
        app.picking_reply,
        Some(matching_id),
        "picking must be scoped to the active search's results"
    );
}

#[test]
fn up_and_down_move_the_pick_older_and_newer_and_clamp_at_both_ends() {
    let mut app = app();
    let first = compose_message(PEER_ID, "first", None);
    let second = compose_message(PEER_ID, "second", None);
    let (first_id, second_id) = (first.id, second.id);
    app.handle_net_event(NetEvent::Received("general".to_string(), first));
    app.handle_net_event(NetEvent::Received("general".to_string(), second));

    ctrl_r(&mut app);
    assert_eq!(app.picking_reply, Some(second_id));

    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.picking_reply, Some(first_id));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(
        app.picking_reply,
        Some(first_id),
        "must clamp at the oldest message"
    );

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.picking_reply, Some(second_id));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(
        app.picking_reply,
        Some(second_id),
        "must clamp at the newest message"
    );
}

#[test]
fn enter_confirms_the_pick_and_exits_picking() {
    let mut app = app();
    let message = compose_message(PEER_ID, "hi", None);
    let id = message.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), message));

    ctrl_r(&mut app);
    app.handle_key(key(KeyCode::Enter));

    assert!(app.picking_reply.is_none());
    assert_eq!(app.replying_to, Some(id));
}

#[test]
fn esc_while_picking_cancels_without_quitting() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hi", None),
    ));
    ctrl_r(&mut app);

    app.handle_key(key(KeyCode::Esc));

    assert!(app.picking_reply.is_none());
    assert!(app.replying_to.is_none());
    assert!(!app.should_quit);
}

#[test]
fn ctrl_c_while_picking_still_quits() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hi", None),
    ));
    ctrl_r(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit);
}

#[test]
fn esc_with_an_armed_reply_clears_it_without_quitting() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hi", None),
    ));
    ctrl_r(&mut app);
    app.handle_key(key(KeyCode::Enter));
    assert!(app.replying_to.is_some());

    app.handle_key(key(KeyCode::Esc));

    assert!(app.replying_to.is_none());
    assert!(!app.should_quit);
}

#[test]
fn sending_while_armed_attaches_reply_to_and_clears_armed_state() {
    let mut app = app();
    let parent = compose_message(PEER_ID, "original", None);
    let parent_id = parent.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), parent));
    app.replying_to = Some(parent_id);

    let action = submit(&mut app, "sounds good");

    match action.expect("a non-empty send returns an action") {
        InputAction::Send(_, message) => assert_eq!(message.reply_to, Some(parent_id)),
        _ => panic!("expected InputAction::Send"),
    }
    assert!(app.replying_to.is_none());
}

#[test]
fn running_a_command_while_armed_does_not_clear_it() {
    let mut app = app();
    app.handle_net_event(NetEvent::Received(
        "general".to_string(),
        compose_message(PEER_ID, "hi", None),
    ));
    ctrl_r(&mut app);
    app.handle_key(key(KeyCode::Enter));
    assert!(app.replying_to.is_some());

    submit(&mut app, "/hints");

    assert!(
        app.replying_to.is_some(),
        "an unrelated command must not clear an armed reply"
    );
}

#[test]
fn switch_channel_clears_an_armed_reply() {
    let mut app = multi_channel_app();
    app.replying_to = Some(2);

    app.handle_key(key(KeyCode::Tab));

    assert!(app.replying_to.is_none());
}

#[test]
fn switch_channel_directly_clears_picking_and_armed_reply() {
    // `switch_channel` itself must clear all three, even though `Tab`
    // can't actually reach it while picking in practice -- `handle_key`
    // routes every key through `handle_picking_key`/
    // `handle_thread_picking_key` first in that state (see
    // `esc_while_picking_cancels_without_quitting` and friends),
    // neither of which has a `Tab` arm of its own. Calling the method
    // directly still documents/guards the invariant.
    let mut app = multi_channel_app();
    app.picking_reply = Some(1);
    app.picking_thread = Some(3);
    app.replying_to = Some(2);

    app.switch_channel(1);

    assert!(app.picking_reply.is_none());
    assert!(app.picking_thread.is_none());
    assert!(app.replying_to.is_none());
}

#[test]
fn remove_channel_of_the_active_one_clears_picking_and_armed_reply() {
    let mut app = multi_channel_app();
    assert_eq!(app.active().name, "random");
    app.picking_reply = Some(1);
    app.picking_thread = Some(3);
    app.replying_to = Some(2);

    app.remove_channel("random");

    assert!(app.picking_reply.is_none());
    assert!(app.picking_thread.is_none());
    assert!(app.replying_to.is_none());
}

#[test]
fn remove_channel_of_a_different_tab_preserves_picking_and_armed_reply() {
    let mut app = multi_channel_app();
    assert_eq!(app.active().name, "random");
    app.picking_reply = Some(1);
    app.picking_thread = Some(3);
    app.replying_to = Some(2);

    app.remove_channel("general");

    assert_eq!(app.picking_reply, Some(1));
    assert_eq!(app.picking_thread, Some(3));
    assert_eq!(app.replying_to, Some(2));
}

#[test]
fn joined_event_clears_picking_and_armed_reply() {
    let mut app = app();
    app.picking_reply = Some(1);
    app.picking_thread = Some(3);
    app.replying_to = Some(2);

    app.handle_net_event(NetEvent::Joined("project-x".to_string()));

    assert!(app.picking_reply.is_none());
    assert!(app.picking_thread.is_none());
    assert!(app.replying_to.is_none());
}

#[test]
fn slash_reply_with_no_messages_reports_an_error() {
    let mut app = app();
    let action = submit(&mut app, "/reply");
    assert!(action.is_none());
    assert!(app.replying_to.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "no messages to reply to");
}

#[test]
fn slash_reply_with_no_text_arms_the_most_recent_message_and_pushes_a_notice() {
    let mut app = app();
    let message = compose_message(PEER_ID, "original", None);
    let id = message.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), message));

    let action = submit(&mut app, "/reply");

    assert!(action.is_none());
    assert_eq!(app.replying_to, Some(id));
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("replying to"), "got: {last}");
}

#[test]
fn slash_reply_with_text_arms_and_sends_in_one_step() {
    let mut app = app();
    let message = compose_message(PEER_ID, "original", None);
    let id = message.id;
    app.handle_net_event(NetEvent::Received("general".to_string(), message));

    let action = submit(&mut app, "/reply sounds good");

    match action.expect("/reply <text> returns a send action") {
        InputAction::Send(channel, sent) => {
            assert_eq!(channel, "general");
            assert_eq!(sent.text, "sounds good");
            assert_eq!(sent.reply_to, Some(id));
        }
        _ => panic!("expected InputAction::Send"),
    }
    assert!(app.replying_to.is_none());
}

#[test]
fn thread_command_opens_the_newest_messages_chain() {
    let mut app = app();
    app.load_history(
        "general",
        vec![
            dated_message(100, 1, PEER_ID, "root"),
            reply_message(200, 2, SELF_ID, "reply", 1),
        ],
    );

    let action = submit(&mut app, "/thread");

    assert!(action.is_none());
    let thread = app
        .active()
        .thread
        .as_ref()
        .expect("thread view must be active");
    assert_eq!(
        thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        thread.origin.id, 2,
        "a bare /thread targets the newest visible message"
    );
}

#[test]
fn thread_command_reports_when_there_are_no_messages() {
    let mut app = app();
    let action = submit(&mut app, "/thread");
    assert!(action.is_none());
    assert!(app.active().thread.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "no messages to view a thread for");
}

#[test]
fn thread_command_closes_an_active_thread_view() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
    submit(&mut app, "/thread");
    assert!(app.active().thread.is_some());

    let action = submit(&mut app, "/thread");

    assert!(action.is_none());
    assert!(app.active().thread.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "thread view closed");
}

#[test]
fn esc_closes_an_active_thread_view_without_quitting() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
    submit(&mut app, "/thread");

    app.handle_key(key(KeyCode::Esc));

    assert!(app.active().thread.is_none());
    assert!(!app.should_quit);
}

#[test]
fn ctrl_t_pick_then_enter_opens_the_picked_messages_thread() {
    let mut app = app();
    app.load_history(
        "general",
        vec![
            dated_message(100, 1, PEER_ID, "first"),
            dated_message(200, 2, PEER_ID, "second"),
        ],
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(
        app.picking_thread,
        Some(2),
        "Ctrl+T starts on the newest message"
    );
    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Enter));

    assert!(app.picking_thread.is_none());
    let thread = app
        .active()
        .thread
        .as_ref()
        .expect("thread view must be active");
    assert_eq!(thread.origin.id, 1);
}

#[test]
fn esc_while_thread_picking_cancels_without_opening_a_thread() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "hi")]);

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    app.handle_key(key(KeyCode::Esc));

    assert!(app.picking_thread.is_none());
    assert!(app.active().thread.is_none());
    assert!(!app.should_quit);
}

#[test]
fn opening_a_thread_clears_an_active_search() {
    let mut app = app();
    app.load_history(
        "general",
        vec![
            dated_message(100, 1, PEER_ID, "hello world"),
            dated_message(200, 2, PEER_ID, "unrelated"),
        ],
    );
    submit(&mut app, "/search hello");
    assert!(app.active().search.is_some());

    submit(&mut app, "/thread");

    assert!(app.active().search.is_none());
    assert!(app.active().thread.is_some());
}

#[test]
fn starting_a_search_clears_an_active_thread() {
    let mut app = app();
    app.load_history(
        "general",
        vec![dated_message(100, 1, PEER_ID, "hello world")],
    );
    submit(&mut app, "/thread");
    assert!(app.active().thread.is_some());

    submit(&mut app, "/search hello");

    assert!(app.active().thread.is_none());
    assert!(app.active().search.is_some());
}

#[test]
fn thread_excludes_a_sibling_reply_to_the_same_parent() {
    let mut app = app();
    app.load_history(
        "general",
        vec![
            dated_message(100, 1, PEER_ID, "parent"),
            reply_message(200, 2, PEER_ID, "origin", 1),
            reply_message(300, 3, PEER_ID, "sibling", 1),
        ],
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    app.handle_key(key(KeyCode::Up)); // move highlight from id 3 to id 2
    app.handle_key(key(KeyCode::Enter));

    let thread = app
        .active()
        .thread
        .as_ref()
        .expect("thread view must be active");
    assert_eq!(
        thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![1, 2],
        "a sibling reply to the same parent must not be pulled in"
    );
}

#[test]
fn sending_a_plain_message_while_viewing_a_thread_replies_within_it() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
    submit(&mut app, "/thread");

    let action = submit(&mut app, "joining the thread");

    match action.expect("a non-empty send returns an action") {
        InputAction::Send(_, message) => {
            assert_eq!(
                message.reply_to,
                Some(1),
                "a plain message sent while viewing a thread must join it"
            );
        }
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn explicit_reply_target_wins_over_the_automatic_thread_target() {
    let mut app = app();
    app.load_history(
        "general",
        vec![
            dated_message(100, 1, PEER_ID, "root"),
            dated_message(200, 2, PEER_ID, "another message"),
        ],
    );
    submit(&mut app, "/thread"); // opens around the newest message (id 2)
    app.replying_to = Some(1); // explicitly armed, e.g. via Ctrl+R or /reply

    let action = submit(&mut app, "explicit reply");

    match action.expect("a non-empty send returns an action") {
        InputAction::Send(_, message) => {
            assert_eq!(
                message.reply_to,
                Some(1),
                "an explicitly armed reply target must win over the thread's implicit one"
            );
        }
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn sending_a_plain_message_without_a_thread_view_has_no_reply_to() {
    let mut app = app();

    let action = submit(&mut app, "just chatting");

    match action.expect("a non-empty send returns an action") {
        InputAction::Send(_, message) => assert_eq!(message.reply_to, None),
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn sending_a_plain_message_while_viewing_a_thread_shows_up_immediately() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
    submit(&mut app, "/thread");

    submit(&mut app, "joining the thread");

    let thread = app
        .active()
        .thread
        .as_ref()
        .expect("thread view must still be active");
    assert_eq!(
        thread.messages.len(),
        2,
        "the just-sent reply must show up without re-running /thread"
    );
    assert!(
        thread
            .messages
            .iter()
            .any(|m| m.text == "joining the thread"),
        "got: {:?}",
        thread.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
    );
}

#[test]
fn a_received_reply_to_a_message_in_the_open_thread_appears_immediately() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
    submit(&mut app, "/thread");

    let reply = reply_message(200, 2, PEER_ID_2, "from a peer", 1);
    app.handle_net_event(NetEvent::Received("general".to_string(), reply));

    let thread = app
        .active()
        .thread
        .as_ref()
        .expect("thread view must still be active");
    assert_eq!(
        thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn an_unrelated_received_message_does_not_join_the_open_thread() {
    let mut app = app();
    app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
    submit(&mut app, "/thread");

    let unrelated = dated_message(200, 2, PEER_ID_2, "different topic");
    app.handle_net_event(NetEvent::Received("general".to_string(), unrelated));

    let thread = app
        .active()
        .thread
        .as_ref()
        .expect("thread view must still be active");
    assert_eq!(
        thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![1],
        "a message with no reply_to into the thread must not join it"
    );
}

#[test]
fn msg_usage_error_when_target_is_missing() {
    let mut app = app();
    let action = submit(&mut app, "/msg");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"));
}

#[test]
fn msg_reports_invalid_hex_for_a_non_hex_unaliased_target() {
    let mut app = app();
    let action = submit(&mut app, "/msg carol");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("invalid hex prefix"), "got: {last}");
}

#[test]
fn msg_reports_no_known_peer_for_an_unknown_hex_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/msg ffffffff");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no known peer"), "got: {last}");
}

#[test]
fn msg_reports_an_error_for_an_ambiguous_alias() {
    let mut app = app();
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_2, "Alice".to_string()),
    ]));

    let action = submit(&mut app, "/msg alice");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("multiple aliased peers"), "got: {last}");
}

#[test]
fn msg_by_alias_starts_a_new_dm_when_none_exists() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/msg alice");

    match action.expect("/msg with no existing DM returns an action") {
        InputAction::StartDm {
            peer,
            channel,
            text,
        } => {
            assert_eq!(peer, PEER_ID);
            assert_eq!(channel, "dm-Alice");
            assert!(text.is_none());
        }
        _ => panic!("expected InputAction::StartDm"),
    }
}

#[test]
fn msg_target_matching_is_case_insensitive() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/msg ALICE");

    match action.expect("a case-insensitive alias match should still work") {
        InputAction::StartDm { peer, .. } => assert_eq!(peer, PEER_ID),
        _ => panic!("expected InputAction::StartDm"),
    }
}

#[test]
fn msg_by_hex_prefix_starts_a_new_dm_named_from_the_hex_prefix() {
    let mut app = app();
    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

    let full_id = hex_id(&PEER_ID);
    let action = submit(&mut app, &format!("/msg {}", &full_id[..8]));

    match action.expect("/msg with a hex prefix returns an action") {
        InputAction::StartDm { peer, channel, .. } => {
            assert_eq!(peer, PEER_ID);
            assert_eq!(channel, format!("dm-{}", hex_prefix(&PEER_ID)));
        }
        _ => panic!("expected InputAction::StartDm"),
    }
}

#[test]
fn msg_with_inline_text_queues_it_on_the_new_dm_action() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/msg alice hey there");

    match action.expect("a successful /msg returns an action") {
        InputAction::StartDm { text, .. } => {
            assert_eq!(text, Some("hey there".to_string()));
        }
        _ => panic!("expected InputAction::StartDm"),
    }
}

#[test]
fn msg_disambiguates_the_channel_name_when_already_taken() {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), "dm-Alice".to_string()],
        "general",
    );
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/msg alice");

    match action.expect("a successful /msg returns an action") {
        InputAction::StartDm { channel, .. } => assert_eq!(channel, "dm-Alice-2"),
        _ => panic!("expected InputAction::StartDm"),
    }
}

#[test]
fn msg_switches_to_an_already_joined_dm_channel_instead_of_creating_another() {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), "dm-Alice".to_string()],
        "general",
    );
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.load_dm_registry(HashMap::from([(PEER_ID, "dm-Alice".to_string())]));

    let action = submit(&mut app, "/msg alice");

    assert!(action.is_none(), "no inline text means nothing to send");
    assert_eq!(app.active().name, "dm-Alice");
}

#[test]
fn msg_switches_to_an_existing_dm_and_sends_inline_text() {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), "dm-Alice".to_string()],
        "general",
    );
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.load_dm_registry(HashMap::from([(PEER_ID, "dm-Alice".to_string())]));

    let action = submit(&mut app, "/msg alice hello again");

    assert_eq!(app.active().name, "dm-Alice");
    match action.expect("inline text should send immediately") {
        InputAction::Send(channel, message) => {
            assert_eq!(channel, "dm-Alice");
            assert_eq!(message.text, "hello again");
        }
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn msg_creates_a_fresh_dm_when_the_recorded_channel_is_no_longer_joined() {
    let mut app = app(); // only "general" is joined
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.load_dm_registry(HashMap::from([(PEER_ID, "dm-Alice".to_string())]));

    let action = submit(&mut app, "/msg alice");

    match action.expect("should fall back to creating a fresh DM") {
        InputAction::StartDm { channel, .. } => assert_eq!(channel, "dm-Alice"),
        _ => panic!("expected InputAction::StartDm"),
    }
}

#[test]
fn dm_joined_activates_the_channel_and_a_later_msg_reuses_it() {
    let mut app = app();

    let result = app.handle_net_event(NetEvent::DmJoined {
        channel: "dm-Alice".to_string(),
        peer: PEER_ID,
    });

    assert!(result.is_none());
    assert_eq!(app.active().name, "dm-Alice");
    assert_eq!(app.channels.len(), 2);

    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    let action = submit(&mut app, "/msg alice");
    assert!(action.is_none(), "should just switch, nothing to send");
    assert_eq!(app.active().name, "dm-Alice");
    assert_eq!(app.channels.len(), 2, "must not create a second DM channel");
}

#[test]
fn record_dm_channel_lets_a_later_msg_reuse_it() {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), "dm-Alice".to_string()],
        "dm-Alice",
    );
    app.record_dm_channel(PEER_ID, "dm-Alice".to_string());
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/msg alice");

    assert!(action.is_none());
    assert_eq!(app.active().name, "dm-Alice");
}

#[test]
fn dm_reports_usage_with_no_argument() {
    let mut app = app();
    let action = submit(&mut app, "/dm");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"), "got: {last}");
}

#[test]
fn dm_reports_no_known_peer_for_an_unknown_short_hex_prefix() {
    let mut app = app();
    let action = submit(&mut app, "/dm ffffffff");
    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("no known peer"), "got: {last}");
}

#[test]
fn dm_accepts_a_full_endpoint_id_even_if_never_seen() {
    // The whole point of /dm: a peer never observed in any channel
    // and never /alias-ed can still be reached directly, as long as
    // their full endpoint id is known -- unlike /msg, which has
    // nothing but `known_ids()` to resolve a hex prefix against.
    let mut app = app();
    let full_id = hex_id(&PEER_ID);

    let action = submit(&mut app, &format!("/dm {full_id}"));

    match action.expect("/dm with a full endpoint id returns an action") {
        InputAction::OpenDirect { peer, text } => {
            assert_eq!(peer, PEER_ID);
            assert!(text.is_none());
        }
        _ => panic!("expected InputAction::OpenDirect"),
    }
}

#[test]
fn dm_by_alias_starts_a_new_conversation_when_none_exists() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/dm alice");

    match action.expect("/dm with no existing conversation returns an action") {
        InputAction::OpenDirect { peer, text } => {
            assert_eq!(peer, PEER_ID);
            assert!(text.is_none());
        }
        _ => panic!("expected InputAction::OpenDirect"),
    }
}

#[test]
fn dm_with_inline_text_queues_it_on_the_open_direct_action() {
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/dm alice hey there");

    match action.expect("a successful /dm returns an action") {
        InputAction::OpenDirect { text, .. } => {
            assert_eq!(text, Some("hey there".to_string()));
        }
        _ => panic!("expected InputAction::OpenDirect"),
    }
}

#[test]
fn dm_refuses_to_message_yourself() {
    let mut app = app();
    let full_id = hex_id(&SELF_ID);

    let action = submit(&mut app, &format!("/dm {full_id}"));

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("yourself"), "got: {last}");
}

#[test]
fn dm_switches_to_an_already_open_direct_chat_instead_of_starting_another() {
    let mut app = app();
    app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.active = 0; // switch away, so /dm has to switch back

    let action = submit(&mut app, "/dm alice");

    assert!(action.is_none(), "no inline text means nothing to send");
    assert_eq!(app.active().name, "p2p-alice");
}

#[test]
fn dm_switches_to_an_existing_direct_chat_and_sends_inline_text() {
    let mut app = app();
    app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.active = 0;

    let action = submit(&mut app, "/dm alice hello again");

    assert_eq!(app.active().name, "p2p-alice");
    match action.expect("inline text should send immediately") {
        InputAction::Send(channel, message) => {
            assert_eq!(channel, "p2p-alice");
            assert_eq!(message.text, "hello again");
        }
        _ => panic!("expected InputAction::Send"),
    }
}

#[test]
fn activate_direct_channel_reports_whether_the_tab_is_new() {
    let mut app = app();
    assert!(app.activate_direct_channel("p2p-alice".to_string(), PEER_ID));
    assert!(
        !app.activate_direct_channel("p2p-alice".to_string(), PEER_ID),
        "an already-open point-to-point tab is not new"
    );
}

#[test]
fn activate_direct_channel_switches_focus_and_records_the_peer() {
    let mut app = app();
    app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);
    assert_eq!(app.active().name, "p2p-alice");
    assert_eq!(app.direct_peer_for("p2p-alice"), Some(PEER_ID));
}

#[test]
fn background_direct_channel_does_not_switch_focus() {
    let mut app = app();
    assert_eq!(app.active().name, "general");

    let is_new = app.background_direct_channel("p2p-alice".to_string(), PEER_ID);

    assert!(is_new);
    assert_eq!(
        app.active().name,
        "general",
        "a background direct channel must not steal focus"
    );
    assert!(app.channels.iter().any(|c| c.name == "p2p-alice"));
    assert_eq!(app.direct_peer_for("p2p-alice"), Some(PEER_ID));
}

#[test]
fn background_direct_channel_on_an_already_known_peer_does_not_duplicate() {
    let mut app = app();
    app.background_direct_channel("p2p-alice".to_string(), PEER_ID);
    let is_new = app.background_direct_channel("p2p-alice".to_string(), PEER_ID);
    assert!(!is_new);
    assert_eq!(app.channels.len(), 2, "must not duplicate an existing tab");
}

#[test]
fn remove_channel_forgets_a_point_to_point_association() {
    let mut app = app();
    app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);

    app.remove_channel("p2p-07070707");

    assert_eq!(app.direct_peer_for("p2p-07070707"), None);
}

#[test]
fn remove_channel_keeps_other_channels_associations() {
    let mut app = app();
    app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);
    app.background_direct_channel("p2p-08080808".to_string(), PEER_ID_2);

    app.remove_channel("p2p-07070707");

    assert_eq!(app.direct_peer_for("p2p-08080808"), Some(PEER_ID_2));
}

#[test]
fn remove_channel_forgets_a_dms_association_so_a_reused_name_is_a_group() {
    let mut app = app_with_dm("dm-Alice", PEER_ID);
    app.remove_channel("dm-Alice");

    // The same name later reused for an ordinary channel must not
    // inherit the old DM's classification.
    app.handle_net_event(NetEvent::Joined("dm-Alice".to_string()));

    assert_eq!(app.channel_kind("dm-Alice"), ChannelKind::Group);
}

#[test]
fn the_left_notice_names_a_dm_by_its_heading() {
    let mut app = app_with_dm("dm-07070707", PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    app.remove_channel("dm-07070707");

    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "left @Alice");
}

#[test]
fn the_left_notice_names_a_point_to_point_chat_by_its_heading() {
    let mut app = app();
    app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    app.remove_channel("p2p-07070707");

    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "left ⚡Alice");
}

#[test]
fn invite_is_refused_on_a_point_to_point_channel() {
    let mut app = app();
    app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);

    let action = submit(&mut app, "/invite");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("point-to-point"), "got: {last}");
}

/// An app with `general` (active) plus a `/msg` DM tab named `name`
/// associated with `peer` -- the shape the channel-label tests below
/// all start from.
fn app_with_dm(name: &str, peer: [u8; 32]) -> AppState {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), name.to_string()],
        "general",
    );
    app.record_dm_channel(peer, name.to_string());
    app
}

#[test]
fn an_ordinary_channel_is_a_group() {
    assert_eq!(app().channel_kind("general"), ChannelKind::Group);
}

#[test]
fn a_msg_dm_channel_is_classified_with_its_peer() {
    let app = app_with_dm("dm-Alice", PEER_ID);
    assert_eq!(
        app.channel_kind("dm-Alice"),
        ChannelKind::GossipDm { peer: PEER_ID }
    );
}

#[test]
fn a_point_to_point_channel_is_classified_with_its_peer() {
    let mut app = app();
    app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);
    assert_eq!(
        app.channel_kind("p2p-07070707"),
        ChannelKind::Direct { peer: PEER_ID }
    );
}

#[test]
fn a_channel_that_only_looks_like_a_dm_is_still_a_group() {
    // Classification comes from the recorded association, never from
    // the shape of the name.
    let mut app = app();
    app.handle_net_event(NetEvent::Joined("dm-Alice".to_string()));

    assert_eq!(app.channel_kind("dm-Alice"), ChannelKind::Group);
    assert_eq!(app.channel_label("dm-Alice"), "dm-Alice");
    assert_eq!(app.channel_heading("dm-Alice"), "#dm-Alice");
}

#[test]
fn a_group_channels_label_is_its_stored_name() {
    let mut app = multi_channel_app();
    // A peer aliased the same as a group channel must not leak into
    // that channel's label.
    app.load_contacts(HashMap::from([(PEER_ID, "random".to_string())]));

    assert_eq!(app.channel_label("general"), "general");
    assert_eq!(app.channel_label("random"), "random");
}

#[test]
fn a_dm_label_follows_a_later_alias() {
    let mut app = app_with_dm("dm-07070707", PEER_ID);
    assert_eq!(app.channel_label("dm-07070707"), "07070707");

    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    submit(
        &mut app,
        &format!("/alias {} Alice", &hex_id(&PEER_ID)[..8]),
    );

    assert_eq!(app.channel_label("dm-07070707"), "Alice");
}

#[test]
fn a_point_to_point_label_follows_a_later_alias() {
    let raw = crate::direct::channel_name(&PEER_ID);
    let mut app = app();
    app.background_direct_channel(raw.clone(), PEER_ID);
    assert_eq!(app.channel_label(&raw), "07070707");

    app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
    submit(
        &mut app,
        &format!("/alias {} Alice", &hex_id(&PEER_ID)[..8]),
    );

    assert_eq!(app.channel_label(&raw), "Alice");
}

#[test]
fn a_label_falls_back_to_the_peers_nickname_when_there_is_no_alias() {
    let mut app = app_with_dm("dm-whoever", PEER_ID);
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: PEER_ID,
            nickname: "alice".to_string(),
        },
    ));

    assert_eq!(app.channel_label("dm-whoever"), "alice (07070707)");
}

#[test]
fn a_dm_invite_recipients_tab_is_labeled_by_the_inviter() {
    // The recipient of a /msg invite gets the channel under the name
    // the *initiator* chose for it -- their own label for the
    // recipient. So "dm-Bob" arriving from Alice is named for Bob
    // himself; the label must say who the chat is actually with.
    let mut app = app();
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    app.handle_net_event(NetEvent::DmJoined {
        channel: "dm-Bob".to_string(),
        peer: PEER_ID,
    });

    assert_eq!(app.active().name, "dm-Bob");
    assert_eq!(app.channel_label("dm-Bob"), "Alice");
}

#[test]
fn a_msg_dm_and_a_dm_chat_with_the_same_peer_share_a_label_but_not_a_heading() {
    let raw = crate::direct::channel_name(&PEER_ID);
    let mut app = app_with_dm("dm-Alice", PEER_ID);
    app.background_direct_channel(raw.clone(), PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    assert_eq!(app.channel_label("dm-Alice"), "Alice");
    assert_eq!(app.channel_label(&raw), "Alice");
    assert_eq!(app.channel_heading("dm-Alice"), "@Alice");
    assert_eq!(app.channel_heading(&raw), "⚡Alice");
}

#[test]
fn leave_accepts_a_dm_by_the_name_shown_on_screen() {
    let mut app = app_with_dm("dm-07070707", PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/leave Alice");

    match action.expect("a label that resolves should leave") {
        InputAction::Leave(channel) => assert_eq!(channel, "dm-07070707"),
        _ => panic!("expected InputAction::Leave"),
    }
}

#[test]
fn leave_still_accepts_a_channels_raw_name() {
    let mut app = app_with_dm("dm-07070707", PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/leave dm-07070707");

    match action.expect("a raw name should still leave") {
        InputAction::Leave(channel) => assert_eq!(channel, "dm-07070707"),
        _ => panic!("expected InputAction::Leave"),
    }
}

#[test]
fn leave_prefers_an_exact_channel_name_over_a_matching_label() {
    // "Alice" is both a group channel's own name and the DM's label.
    let mut app = AppState::new(
        SELF_ID,
        vec![
            "general".to_string(),
            "Alice".to_string(),
            "dm-07070707".to_string(),
        ],
        "general",
    );
    app.record_dm_channel(PEER_ID, "dm-07070707".to_string());
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    let action = submit(&mut app, "/leave Alice");

    match action.expect("an exact name should win") {
        InputAction::Leave(channel) => assert_eq!(channel, "Alice"),
        _ => panic!("expected InputAction::Leave"),
    }
}

#[test]
fn leave_refuses_a_label_shared_by_several_channels() {
    let mut app = AppState::new(
        SELF_ID,
        vec![
            "general".to_string(),
            "dm-07070707".to_string(),
            "dm-08080808".to_string(),
        ],
        "general",
    );
    app.record_dm_channel(PEER_ID, "dm-07070707".to_string());
    app.record_dm_channel(PEER_ID_2, "dm-08080808".to_string());
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_2, "Alice".to_string()),
    ]));

    let action = submit(&mut app, "/leave Alice");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("more than one channel"), "got: {last}");
}

#[test]
fn who_names_a_dm_by_its_heading() {
    let mut app = app_with_dm("dm-07070707", PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    app.active = 1; // the DM tab

    let action = submit(&mut app, "/who");

    assert!(action.is_none());
    assert_eq!(app.viewing_peers.as_ref().unwrap().heading, "@Alice");
}

/// `general`, a `/msg` DM (`dm-Alice`), `project-x`, and a `/dm` chat
/// (`p2p-08080808`) -- joined in that order, so the two kinds are
/// interleaved -- with `general` active. The shape the navigation tests
/// below all start from.
fn app_with_interleaved_kinds() -> AppState {
    let mut app = AppState::new(
        SELF_ID,
        vec![
            "general".to_string(),
            "dm-Alice".to_string(),
            "project-x".to_string(),
        ],
        "general",
    );
    app.record_dm_channel(PEER_ID, "dm-Alice".to_string());
    app.background_direct_channel(crate::direct::channel_name(&PEER_ID_2), PEER_ID_2);
    app
}

#[test]
fn each_channel_kind_has_its_own_glyph() {
    let glyphs = [
        ChannelKind::Group.glyph(),
        ChannelKind::GossipDm { peer: PEER_ID }.glyph(),
        ChannelKind::Direct { peer: PEER_ID }.glyph(),
    ];

    assert_eq!(glyphs, ["#", "@", "⚡"]);
}

#[test]
fn a_channels_heading_is_its_label_behind_its_kind_glyph() {
    let mut app = app_with_interleaved_kinds();
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_2, "Bob".to_string()),
    ]));

    assert_eq!(app.channel_heading("general"), "#general");
    assert_eq!(app.channel_heading("dm-Alice"), "@Alice");
    assert_eq!(app.channel_heading("p2p-08080808"), "⚡Bob");
}

#[test]
fn nav_sections_split_group_channels_from_conversations_in_join_order() {
    let app = app_with_interleaved_kinds();

    let sections = app.nav_sections();

    assert_eq!(sections.channels, vec![0, 2], "general and project-x");
    assert_eq!(sections.people, vec![1, 3], "the DM and the /dm chat");
}

#[test]
fn nav_sections_of_only_group_channels_has_no_people() {
    let sections = multi_channel_app().nav_sections();

    assert_eq!(sections.channels, vec![0, 1]);
    assert!(sections.people.is_empty());
}

#[test]
fn tab_walks_the_panel_top_to_bottom_and_wraps() {
    let mut app = app_with_interleaved_kinds();
    let mut visited = Vec::new();

    for _ in 0..4 {
        app.handle_key(key(KeyCode::Tab));
        visited.push(app.active().name.clone());
    }

    // Group channels first (general -> project-x), then conversations
    // (dm-Alice -> the /dm chat): the order the panel draws them in,
    // not the order they were joined in. Then back to the top.
    assert_eq!(
        visited,
        ["project-x", "dm-Alice", "p2p-08080808", "general"]
    );
}

#[test]
fn backtab_walks_the_panel_bottom_to_top_and_wraps() {
    let mut app = app_with_interleaved_kinds();
    let mut visited = Vec::new();

    for _ in 0..4 {
        app.handle_key(key(KeyCode::BackTab));
        visited.push(app.active().name.clone());
    }

    assert_eq!(
        visited,
        ["p2p-08080808", "dm-Alice", "project-x", "general"]
    );
}

#[test]
fn leave_accepts_a_channel_by_its_heading() {
    let mut app = app_with_interleaved_kinds();
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_2, "Bob".to_string()),
    ]));

    for (typed, expected) in [
        ("#project-x", "project-x"),
        ("@Alice", "dm-Alice"),
        ("⚡Bob", "p2p-08080808"),
    ] {
        match submit(&mut app, &format!("/leave {typed}")) {
            Some(InputAction::Leave(channel)) => assert_eq!(channel, expected, "{typed}"),
            _ => panic!("expected InputAction::Leave for {typed}"),
        }
    }
}

#[test]
fn a_heading_tells_apart_a_dm_and_a_dm_chat_with_the_same_person() {
    let raw = crate::direct::channel_name(&PEER_ID);
    let mut app = app_with_dm("dm-Alice", PEER_ID);
    app.background_direct_channel(raw.clone(), PEER_ID);
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    // The bare label is shared, so on its own it can't say which one
    // is meant...
    assert!(submit(&mut app, "/leave Alice").is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("more than one channel"), "got: {last}");

    // ...but each one's heading can.
    match submit(&mut app, "/leave @Alice") {
        Some(InputAction::Leave(channel)) => assert_eq!(channel, "dm-Alice"),
        _ => panic!("expected InputAction::Leave for the DM"),
    }
    match submit(&mut app, "/leave ⚡Alice") {
        Some(InputAction::Leave(channel)) => assert_eq!(channel, raw),
        _ => panic!("expected InputAction::Leave for the /dm chat"),
    }
}

/// `app_with_interleaved_kinds` with its two peers named, so the labels
/// are `general`, `Alice` (a `/msg` DM), `project-x` and `Bob` (a `/dm`
/// chat): channel indices 0 to 3, but 0, 2, 1, 3 in the order the panel
/// draws them.
fn app_for_the_switcher() -> AppState {
    let mut app = app_with_interleaved_kinds();
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_2, "Bob".to_string()),
    ]));
    app
}

fn ctrl_p(app: &mut AppState) {
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
}

/// Types `text` one key at a time without pressing `Enter` -- into
/// whatever has the keyboard, the switcher's query once it's open.
fn type_text(app: &mut AppState, text: &str) {
    for c in text.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

fn switcher_query(app: &AppState) -> &str {
    &app.switching.as_ref().expect("the switcher is open").query
}

#[test]
fn ctrl_p_opens_the_switcher_listing_every_channel_in_panel_order() {
    let mut app = app_for_the_switcher();
    assert!(app.switching.is_none());
    assert!(
        app.switcher_matches().is_empty(),
        "nothing is listed while it is closed"
    );

    ctrl_p(&mut app);

    assert!(app.switching.is_some());
    assert_eq!(app.switcher_matches(), vec![0, 2, 1, 3]);
}

#[test]
fn typing_narrows_the_switcher_to_labels_containing_the_text_ignoring_case() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);

    type_text(&mut app, "ECT"); // from the middle of "project-x"
    assert_eq!(app.switcher_matches(), vec![2]);

    type_text(&mut app, "z");
    assert!(app.switcher_matches().is_empty());
}

#[test]
fn the_switcher_matches_the_label_not_the_stored_channel_name() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);

    // `dm-Alice` and `p2p-08080808` are the stored names; what is shown
    // for them is `Alice` and `Bob`.
    type_text(&mut app, "dm-");
    assert!(app.switcher_matches().is_empty());
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "p2p");
    assert!(app.switcher_matches().is_empty());
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "ali");
    assert_eq!(app.switcher_matches(), vec![1]);
}

#[test]
fn backspace_widens_the_switcher_again() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);
    type_text(&mut app, "zz");
    assert!(app.switcher_matches().is_empty());

    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.switcher_matches(), vec![0, 2, 1, 3]);

    app.handle_key(key(KeyCode::Backspace)); // nothing left to delete
    assert_eq!(switcher_query(&app), "");
}

#[test]
fn up_and_down_move_the_switcher_highlight_and_clamp_at_both_ends() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);
    assert_eq!(app.switcher_highlight(), Some(0));

    // Down walks the panel order: 0, 2, 1, 3 -- and stops at the end.
    let mut seen = Vec::new();
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Down));
        seen.push(app.switcher_highlight());
    }
    assert_eq!(seen, [Some(2), Some(1), Some(3), Some(3)]);

    let mut seen = Vec::new();
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Up));
        seen.push(app.switcher_highlight());
    }
    assert_eq!(seen, [Some(1), Some(2), Some(0), Some(0)]);
}

#[test]
fn editing_the_query_puts_the_switcher_highlight_back_on_the_first_match() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.switcher_highlight(), Some(1));

    type_text(&mut app, "o"); // project-x and Bob
    assert_eq!(app.switcher_matches(), vec![2, 3]);
    assert_eq!(app.switcher_highlight(), Some(2));

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.switcher_highlight(), Some(3));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.switcher_highlight(), Some(0));
}

#[test]
fn enter_in_the_switcher_switches_to_the_highlighted_channel_and_closes_it() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);
    type_text(&mut app, "bob");

    let action = app.handle_key(key(KeyCode::Enter));

    assert!(action.is_none());
    assert_eq!(app.active().name, "p2p-08080808");
    assert!(app.switching.is_none());
}

#[test]
fn the_arrow_keys_pick_which_match_enter_switches_to() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);

    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.active().name, "dm-Alice");
}

#[test]
fn switching_through_the_switcher_clears_unread_and_an_armed_reply() {
    let mut app = app_for_the_switcher();
    app.channels[3].unread_count = 1;
    app.channels[3].first_unread = Some(7);
    app.replying_to = Some(7);
    ctrl_p(&mut app);
    type_text(&mut app, "bob");

    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.active().unread_count, 0);
    assert_eq!(app.active().unread_anchor, Some(7));
    assert!(app.replying_to.is_none());
}

#[test]
fn choosing_the_channel_you_are_already_on_just_closes_the_switcher() {
    let mut app = app_for_the_switcher();
    app.replying_to = Some(7);
    ctrl_p(&mut app);
    type_text(&mut app, "general");

    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.active, 0);
    assert!(app.switching.is_none());
    assert_eq!(
        app.replying_to,
        Some(7),
        "staying put must not discard an armed reply"
    );
}

#[test]
fn enter_with_nothing_matching_keeps_the_switcher_open() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);
    type_text(&mut app, "zzz");

    app.handle_key(key(KeyCode::Enter));

    assert!(app.switching.is_some());
    assert_eq!(app.active, 0);
}

#[test]
fn esc_closes_the_switcher_without_switching_or_quitting() {
    let mut app = app_for_the_switcher();
    app.replying_to = Some(7);
    ctrl_p(&mut app);
    app.handle_key(key(KeyCode::Down)); // highlight another channel

    app.handle_key(key(KeyCode::Esc));

    assert!(app.switching.is_none());
    assert_eq!(app.active, 0, "cancelling leaves the active channel alone");
    assert!(!app.should_quit);
    assert_eq!(app.replying_to, Some(7));
}

#[test]
fn ctrl_c_in_the_switcher_still_quits() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit);
}

#[test]
fn keys_typed_into_the_switcher_never_reach_the_input_draft() {
    let mut app = app_for_the_switcher();
    type_text(&mut app, "draft");
    ctrl_p(&mut app);

    type_text(&mut app, "bob");
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Esc));

    assert_eq!(app.active().input, "draft");
    assert_eq!(app.active().cursor, 5);
}

#[test]
fn control_chords_do_not_type_into_the_switcher_query() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    ctrl_p(&mut app); // a second Ctrl+P is just another chord to ignore

    assert_eq!(switcher_query(&app), "");
    assert!(app.switching.is_some());
}

#[test]
fn a_paste_filters_the_switcher_instead_of_the_draft() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);

    app.paste_text("bo\nb\r\n");

    assert_eq!(switcher_query(&app), "bob");
    assert_eq!(app.active().input, "");
    assert_eq!(app.switcher_matches(), vec![3]);
}

#[test]
fn the_switcher_highlight_stays_in_range_when_the_matches_shrink_underneath() {
    let mut app = app_for_the_switcher();
    ctrl_p(&mut app);
    for _ in 0..3 {
        app.handle_key(key(KeyCode::Down));
    }
    assert_eq!(app.switcher_highlight(), Some(3)); // Bob, the last match

    app.remove_channel("p2p-08080808"); // gone while the overlay is open

    assert_eq!(app.switcher_matches(), vec![0, 2, 1]);
    assert_eq!(
        app.switcher_highlight(),
        Some(1),
        "clamped to the new last match"
    );
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.active().name, "dm-Alice");
}

#[test]
fn the_switcher_follows_labels_that_change_while_it_is_open() {
    let mut app = app_with_interleaved_kinds(); // peers unnamed: hex labels
    ctrl_p(&mut app);
    type_text(&mut app, "ali");
    assert!(app.switcher_matches().is_empty());

    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    assert_eq!(app.switcher_matches(), vec![1]);
}

#[test]
fn ctrl_p_is_ignored_while_another_mode_has_the_keyboard() {
    let mut app = app_for_the_switcher();
    app.picking_reply = Some(1);
    ctrl_p(&mut app);
    assert!(app.switching.is_none());
    assert_eq!(app.picking_reply, Some(1));

    app.picking_reply = None;
    app.pending_redeem = Some("ticket".to_string());
    ctrl_p(&mut app);
    assert!(app.switching.is_none());
}

// -- the files overlay ----------------------------------------------------
// These reuse the switcher fixture (`app_for_the_switcher`) where they
// need every kind of conversation: `general` (active), a `/msg` DM shown
// as `Alice`, `project-x`, and a `/dm` chat shown as `Bob`.

/// Opens the files overlay with `/files`, returning what that asked of
/// the caller.
fn open_files(app: &mut AppState) -> Option<InputAction> {
    submit(app, "/files")
}

fn ctrl_o(app: &mut AppState) -> Option<InputAction> {
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL))
}

fn files_query(app: &AppState) -> &str {
    &app.viewing_files
        .as_ref()
        .expect("the files overlay is open")
        .query
}

fn files_notice(app: &AppState) -> Option<FilesNotice> {
    app.viewing_files
        .as_ref()
        .expect("the files overlay is open")
        .notice
}

/// The overlay open on `general`, whose one file was announced as
/// `announced` and -- if the disk has said so -- is saved at `saved_at`.
fn app_viewing_file(announced: &str, saved_at: Option<&str>) -> AppState {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, announced, b"content"),
    );
    open_files(&mut app);
    if let Some(path) = saved_at {
        mark_saved(&mut app, "general", hash_of(b"content"), path);
    }
    app
}

/// `app_viewing_file` on `report.pdf`, `saved` or not.
fn app_viewing_report(saved: bool) -> AppState {
    app_viewing_file("report.pdf", saved.then_some("/files/general/report.pdf"))
}

/// The names of the files the open overlay lists, top to bottom.
fn listed(app: &AppState) -> Vec<String> {
    app.files_matches()
        .iter()
        .map(|entry| entry.filename().to_string())
        .collect()
}

/// The hash `file_message` gives an attachment with `content`.
fn hash_of(content: &[u8]) -> Hash {
    Hash::new(content)
}

/// Every field of a `SaveFile` action -- channel, hash, filename,
/// providers, folder -- panicking on anything else, to compare what the
/// overlay asks for with what `/save` asks for.
fn save_fields(action: Option<InputAction>) -> (String, Hash, String, Vec<[u8; 32]>, PathBuf) {
    match action.expect("a save returns an action") {
        InputAction::SaveFile(request) => (
            request.channel,
            request.hash,
            request.filename,
            request.providers,
            request.dir,
        ),
        _ => panic!("expected InputAction::SaveFile"),
    }
}

/// Tells the open overlay that `channel`'s copy of `hash` is at `path`,
/// as the caller does when it answers `InputAction::CheckSavedFiles`.
fn mark_saved(app: &mut AppState, channel: &str, hash: Hash, path: &str) {
    app.apply_saved_files(channel, HashMap::from([(hash, PathBuf::from(path))]));
}

fn apply_local_files(app: &mut AppState, files: Vec<LocalFile>) {
    let overlay = app.viewing_files.as_ref().expect("files overlay is open");
    app.apply_files_listing(FilesListing {
        request: overlay.request,
        channel: overlay.channel.clone(),
        files: Ok(files),
    });
}

fn local_file(relative: &str, modified_unix_ms: u64) -> LocalFile {
    LocalFile {
        path: PathBuf::from("/files/general").join(relative),
        relative: relative.to_string(),
        size: 7,
        modified_unix_ms,
    }
}

/// `general` with four files -- two from Alice, one from Bob, one of our
/// own -- listed newest first as `mine.zip`, `notes.txt`, `holiday.png`,
/// `Quarterly-Report.PDF`.
fn app_with_a_few_files() -> AppState {
    let mut app = app();
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_2, "Bob".to_string()),
    ]));
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "Quarterly-Report.PDF", b"q"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "holiday.png", b"h"),
    );
    receive(
        &mut app,
        "general",
        file_message(3, PEER_ID, "notes.txt", b"n"),
    );
    app.record_sent_message("general", file_message(4, SELF_ID, "mine.zip", b"m"));
    app
}

#[test]
fn slash_files_opens_the_overlay_on_the_active_channel_and_reads_its_folder() {
    let mut app = multi_channel_app(); // "random" is active

    let action = open_files(&mut app);

    match action.expect("/files returns an action") {
        InputAction::ReadFiles { channel, .. } => assert_eq!(channel, "random"),
        _ => panic!("expected InputAction::ReadFiles"),
    }
    let overlay = app.viewing_files.as_ref().expect("the overlay is open");
    assert_eq!(overlay.channel, "random");
    assert_eq!(overlay.query, "");
}

#[test]
fn synced_files_join_shared_files_and_match_nested_relative_paths() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    apply_local_files(
        &mut app,
        vec![local_file("docs/project-plan.md", now_unix_ms())],
    );

    assert_eq!(listed(&app), ["docs/project-plan.md", "report.pdf"]);
    type_text(&mut app, "project");
    assert_eq!(listed(&app), ["docs/project-plan.md"]);
}

#[test]
fn a_local_scan_deduplicates_a_saved_shared_copy() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    mark_saved(
        &mut app,
        "general",
        hash_of(b"report"),
        "/files/general/report.pdf",
    );
    apply_local_files(&mut app, vec![local_file("report.pdf", now_unix_ms())]);

    assert_eq!(app.overlay_files().len(), 1);
    assert!(matches!(
        app.overlay_files().as_slice(),
        [FilesEntry::Shared(_)]
    ));
}

#[test]
fn a_late_files_listing_does_not_update_a_newer_overlay() {
    let mut app = app();
    open_files(&mut app);
    let request = app.viewing_files.as_ref().unwrap().request;
    let channel = app.viewing_files.as_ref().unwrap().channel.clone();
    app.handle_key(key(KeyCode::Esc));
    open_files(&mut app);

    app.apply_files_listing(FilesListing {
        request,
        channel,
        files: Ok(vec![local_file("stale.md", now_unix_ms())]),
    });

    assert!(app.overlay_files().is_empty());
}

#[test]
fn local_files_offer_open_reveal_and_edit_actions() {
    let mut app = app();
    open_files(&mut app);
    apply_local_files(&mut app, vec![local_file("notes.txt", now_unix_ms())]);

    assert!(matches!(
        ctrl_o(&mut app),
        Some(InputAction::OpenFile(path)) if path == PathBuf::from("/files/general/notes.txt")
    ));

    open_files(&mut app);
    apply_local_files(&mut app, vec![local_file("notes.txt", now_unix_ms())]);
    assert!(matches!(
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        Some(InputAction::RevealFile(path)) if path == PathBuf::from("/files/general/notes.txt")
    ));

    open_files(&mut app);
    apply_local_files(&mut app, vec![local_file("notes.txt", now_unix_ms())]);
    assert!(matches!(
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
        Some(InputAction::EditFile(path)) if path == PathBuf::from("/files/general/notes.txt")
    ));
}

#[test]
fn the_overlay_lists_the_files_of_the_channel_newest_first_and_nothing_else() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "old.pdf", b"old"),
    );
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID, "chatter", None),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "mid.pdf", b"mid"),
    );
    app.push_system("a notice");
    receive(
        &mut app,
        "general",
        file_message(3, PEER_ID, "new.pdf", b"new"),
    );

    open_files(&mut app);

    assert_eq!(listed(&app), ["new.pdf", "mid.pdf", "old.pdf"]);
}

#[test]
fn a_row_carries_the_name_size_sender_and_date_of_its_share() {
    let mut app = app();
    let sent = ChatMessage {
        ts_unix_ms: 1_760_000_000_000,
        ..file_message(1, PEER_ID, "report.pdf", b"six by")
    };
    receive(&mut app, "general", sent);
    open_files(&mut app);

    let files = app.files_matches();

    assert_eq!(files.len(), 1);
    let FilesEntry::Shared(shared) = &files[0] else {
        panic!("expected shared attachment");
    };
    assert_eq!(shared.file.filename, "report.pdf");
    assert_eq!(shared.file.size, 6);
    assert_eq!(shared.file.hash, hash_of(b"six by"));
    assert_eq!(shared.sender, PEER_ID);
    assert_eq!(shared.ts_unix_ms, 1_760_000_000_000);
}

#[test]
fn loaded_backfilled_and_live_files_are_all_listed() {
    let mut app = app();
    app.load_history(
        "general",
        vec![file_message(10, PEER_ID, "loaded.pdf", b"loaded")],
    );
    app.merge_history(
        "general",
        vec![file_message(5, PEER_ID_2, "backfilled.pdf", b"backfilled")],
    );
    receive(
        &mut app,
        "general",
        file_message(20, PEER_ID, "live.pdf", b"live"),
    );

    open_files(&mut app);

    assert_eq!(listed(&app), ["live.pdf", "loaded.pdf", "backfilled.pdf"]);
}

#[test]
fn a_file_shared_twice_is_listed_once_per_share_with_its_own_sender() {
    let mut app = app();
    let content = b"same bytes";
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", content),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "report-final.pdf", content),
    );
    open_files(&mut app);

    let files = app.files_matches();

    let rows: Vec<(&str, [u8; 32])> = files
        .iter()
        .map(|entry| match entry {
            FilesEntry::Shared(shared) => (shared.file.filename.as_str(), shared.sender),
            FilesEntry::Local(_) => panic!("fixture has no local files"),
        })
        .collect();
    assert_eq!(
        rows,
        [("report-final.pdf", PEER_ID_2), ("report.pdf", PEER_ID)]
    );
}

#[test]
fn our_own_files_are_listed_too() {
    let mut app = app();
    app.record_sent_message("general", file_message(1, SELF_ID, "mine.pdf", b"mine"));

    open_files(&mut app);

    assert_eq!(listed(&app), ["mine.pdf"]);
    assert!(matches!(
        app.files_matches()[0],
        FilesEntry::Shared(SharedFile {
            sender: SELF_ID,
            ..
        })
    ));
}

#[test]
fn only_the_files_of_the_channel_it_was_opened_on_are_listed() {
    let mut app = multi_channel_app(); // "random" is active
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "here.pdf", b"here"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID, "elsewhere.pdf", b"elsewhere"),
    );

    open_files(&mut app);

    assert_eq!(listed(&app), ["here.pdf"]);
}

#[test]
fn the_overlay_works_in_group_channels_dms_and_dm_chats() {
    let mut app = app_for_the_switcher();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "group.pdf", b"group"),
    );
    receive(
        &mut app,
        "dm-Alice",
        file_message(2, PEER_ID, "dm.pdf", b"dm"),
    );
    receive(
        &mut app,
        "p2p-08080808",
        file_message(3, PEER_ID_2, "chat.pdf", b"chat"),
    );

    for (index, name, expected) in [
        (0, "general", "group.pdf"),
        (1, "dm-Alice", "dm.pdf"),
        (3, "p2p-08080808", "chat.pdf"),
    ] {
        app.active = index;
        match open_files(&mut app).expect("/files returns an action") {
            InputAction::ReadFiles { channel, .. } => assert_eq!(channel, name),
            _ => panic!("expected InputAction::ReadFiles"),
        }
        assert_eq!(listed(&app), [expected], "{name}");
        app.handle_key(key(KeyCode::Esc));
    }
}

#[test]
fn an_empty_channel_opens_an_overlay_with_nothing_in_it() {
    let mut app = app();

    open_files(&mut app);

    assert!(app.viewing_files.is_some());
    assert!(app.files_matches().is_empty());
    assert_eq!(app.files_highlight(), None);
    // Nothing to save or open, so those keys leave it as it is.
    assert!(app.handle_key(key(KeyCode::Enter)).is_none());
    assert!(ctrl_o(&mut app).is_none());
    assert!(app.viewing_files.is_some());
    assert_eq!(files_notice(&app), None, "no file, nothing to say about it");
}

#[test]
fn typing_narrows_the_list_to_names_containing_the_text_ignoring_case() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);
    assert_eq!(
        listed(&app),
        [
            "mine.zip",
            "notes.txt",
            "holiday.png",
            "Quarterly-Report.PDF"
        ]
    );

    type_text(&mut app, "REPORT");
    assert_eq!(listed(&app), ["Quarterly-Report.PDF"]);

    type_text(&mut app, "x");
    assert!(listed(&app).is_empty());
}

#[test]
fn typing_also_narrows_by_the_sender_as_shown() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);

    type_text(&mut app, "ALI");
    assert_eq!(
        listed(&app),
        ["notes.txt", "Quarterly-Report.PDF"],
        "Alice's files"
    );

    for _ in 0..3 {
        app.handle_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "bob");
    assert_eq!(listed(&app), ["holiday.png"]);

    for _ in 0..3 {
        app.handle_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "you");
    assert_eq!(listed(&app), ["mine.zip"], "our own are from you");
}

#[test]
fn a_sender_is_matched_by_what_is_shown_not_by_the_raw_id() {
    let mut app = app();
    receive(&mut app, "general", file_message(1, PEER_ID, "a.pdf", b"a"));
    open_files(&mut app);

    // Unnamed, a peer is shown by the first bytes of its id.
    type_text(&mut app, "0707");
    assert_eq!(listed(&app), ["a.pdf"]);

    // Named, it is shown by its name -- which the hex no longer matches.
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
    assert!(listed(&app).is_empty());
}

#[test]
fn up_and_down_move_the_highlight_and_clamp_at_both_ends() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);
    assert_eq!(app.files_highlight(), Some(0));

    let mut seen = Vec::new();
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Down));
        seen.push(app.files_highlight());
    }
    assert_eq!(seen, [Some(1), Some(2), Some(3), Some(3), Some(3)]);

    let mut seen = Vec::new();
    for _ in 0..5 {
        app.handle_key(key(KeyCode::Up));
        seen.push(app.files_highlight());
    }
    assert_eq!(seen, [Some(2), Some(1), Some(0), Some(0), Some(0)]);
}

#[test]
fn editing_the_query_puts_the_highlight_back_on_the_first_match() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.files_highlight(), Some(2));

    // From the name `holiday.png` and `Quarterly-Report.PDF`, and from
    // the sender of `mine.zip`, `you`.
    type_text(&mut app, "y");
    assert_eq!(
        listed(&app),
        ["mine.zip", "holiday.png", "Quarterly-Report.PDF"]
    );
    assert_eq!(app.files_highlight(), Some(0));

    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.files_highlight(), Some(2));
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.files_highlight(), Some(0));
}

#[test]
fn the_highlight_stays_in_range_when_the_matches_shrink_underneath() {
    let mut app = app();
    receive(&mut app, "general", file_message(1, PEER_ID, "a.pdf", b"a"));
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "b.pdf", b"b"),
    );
    receive(
        &mut app,
        "general",
        file_message(3, PEER_ID_2, "c.pdf", b"c"),
    );
    open_files(&mut app);
    type_text(&mut app, "0"); // in the hex of both unnamed peers
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(listed(&app), ["c.pdf", "b.pdf", "a.pdf"]);
    assert_eq!(app.files_highlight(), Some(2));

    // Alice is no longer shown as hex, so `a.pdf` stops matching.
    app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

    assert_eq!(listed(&app), ["c.pdf", "b.pdf"]);
    assert_eq!(
        app.files_highlight(),
        Some(1),
        "clamped to the new last match"
    );
}

#[test]
fn esc_closes_the_overlay_without_quitting_or_acting() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);
    let before = app.active().messages.len();

    let action = app.handle_key(key(KeyCode::Esc));

    assert!(action.is_none());
    assert!(app.viewing_files.is_none());
    assert!(!app.should_quit);
    assert_eq!(app.active().messages.len(), before);
}

#[test]
fn ctrl_c_in_the_overlay_still_quits() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

    assert!(app.should_quit);
}

#[test]
fn keys_typed_into_the_overlay_never_reach_the_input_draft() {
    let mut app = multi_channel_app();
    open_files(&mut app);

    type_text(&mut app, "report");
    app.handle_key(key(KeyCode::Backspace));
    app.handle_key(key(KeyCode::Left));
    app.handle_key(key(KeyCode::Tab));

    assert_eq!(app.active().input, "");
    assert_eq!(app.active().cursor, 0);
    assert_eq!(app.active, 1, "Tab did not switch channels either");
    assert_eq!(files_query(&app), "repor");
}

#[test]
fn control_chords_do_not_type_into_the_query_or_open_other_overlays() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    ctrl_p(&mut app);

    assert_eq!(files_query(&app), "");
    assert!(app.viewing_files.is_some());
    assert!(app.switching.is_none());
}

#[test]
fn a_paste_filters_the_overlay_instead_of_the_draft() {
    let mut app = app_with_a_few_files();
    open_files(&mut app);

    app.paste_text("rep\nort\r\n");

    assert_eq!(files_query(&app), "report");
    assert_eq!(app.active().input, "");
    assert_eq!(listed(&app), ["Quarterly-Report.PDF"]);
}

#[test]
fn nothing_is_composing_while_the_overlay_is_open() {
    let mut app = app();
    assert!(app.is_composing());

    open_files(&mut app);
    assert!(!app.is_composing());

    app.handle_key(key(KeyCode::Esc));
    assert!(app.is_composing());
}

#[test]
fn enter_saves_the_highlighted_file_exactly_as_slash_save_would() {
    let mut app = app();
    app.load_files_root(PathBuf::from("/files"));
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "old.pdf", b"old"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID_2, "new.pdf", b"new"),
    );
    peers_join(&mut app, "general", &[PEER_ID_3]);
    // A bare `/save` asks for the newest file, which is the one the
    // overlay starts on.
    let by_command = save_fields(submit(&mut app, "/save"));
    open_files(&mut app);

    let by_overlay = save_fields(app.handle_key(key(KeyCode::Enter)));

    assert_eq!(by_overlay, by_command);
    assert_eq!(by_overlay.0, "general");
    assert_eq!(by_overlay.2, "new.pdf");
    assert_eq!(by_overlay.3, vec![PEER_ID_2, PEER_ID_3, PEER_ID]);
    assert_eq!(by_overlay.4, PathBuf::from("/files/general"));
}

#[test]
fn enter_closes_the_overlay_and_leaves_the_saving_notice_in_the_transcript() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);

    app.handle_key(key(KeyCode::Enter));

    assert!(app.viewing_files.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "saving report.pdf...");
}

#[test]
fn enter_saves_the_file_that_is_highlighted_not_just_the_newest() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "old.pdf", b"old"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID, "new.pdf", b"new"),
    );
    open_files(&mut app);
    app.handle_key(key(KeyCode::Down));

    let (_, hash, filename, ..) = save_fields(app.handle_key(key(KeyCode::Enter)));

    assert_eq!((hash, filename.as_str()), (hash_of(b"old"), "old.pdf"));
}

#[test]
fn enter_saves_one_of_our_own_files_from_our_own_store() {
    let mut app = app();
    app.record_sent_message("general", file_message(1, SELF_ID, "mine.pdf", b"mine"));
    peers_join(&mut app, "general", &[PEER_ID]);
    open_files(&mut app);

    let (_, _, filename, providers, _) = save_fields(app.handle_key(key(KeyCode::Enter)));

    assert_eq!(filename, "mine.pdf");
    assert_eq!(
        providers,
        vec![SELF_ID, PEER_ID],
        "this device is the sender, and holds the file"
    );
}

#[test]
fn enter_saves_into_the_folder_of_each_kind_of_conversation() {
    let mut app = app_for_the_switcher();
    app.load_files_root(PathBuf::from("/files"));
    receive(
        &mut app,
        "dm-Alice",
        file_message(1, PEER_ID, "dm.pdf", b"dm"),
    );
    receive(
        &mut app,
        "p2p-08080808",
        file_message(2, PEER_ID_2, "chat.pdf", b"chat"),
    );

    let mut dirs = Vec::new();
    for index in [1, 3] {
        app.active = index;
        open_files(&mut app);
        dirs.push(save_fields(app.handle_key(key(KeyCode::Enter))).4);
    }

    assert_eq!(
        dirs,
        [
            channel_dir(Path::new("/files"), "dm-Alice"),
            PathBuf::from("/files/p2p-08080808")
        ]
    );
}

#[test]
fn the_overlay_stays_on_the_channel_it_was_opened_on_when_focus_moves() {
    let mut app = multi_channel_app(); // "random" is active
    app.load_files_root(PathBuf::from("/files"));
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "here.pdf", b"here"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID, "there.pdf", b"there"),
    );
    open_files(&mut app);

    // A join finishing elsewhere takes focus.
    app.handle_net_event(NetEvent::Joined("general".to_string()));
    assert_eq!(app.active().name, "general");

    assert_eq!(listed(&app), ["here.pdf"], "still #random's list");
    let (channel, _, filename, _, dir) = save_fields(app.handle_key(key(KeyCode::Enter)));
    assert_eq!(
        (channel.as_str(), filename.as_str()),
        ("random", "here.pdf")
    );
    assert_eq!(dir, PathBuf::from("/files/random"));
}

#[test]
fn enter_on_a_file_already_saved_does_nothing_so_it_is_never_saved_twice() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    mark_saved(
        &mut app,
        "general",
        hash_of(b"report"),
        "/files/general/report.pdf",
    );
    let before = app.active().messages.len();

    let action = app.handle_key(key(KeyCode::Enter));

    assert!(action.is_none());
    assert!(app.viewing_files.is_some(), "still open");
    assert_eq!(
        app.active().messages.len(),
        before,
        "no `saving` notice either"
    );
}

#[test]
fn enter_on_a_file_already_saved_says_so_and_what_to_press_instead_works() {
    let mut app = app_viewing_report(true);
    assert_eq!(files_notice(&app), None, "nothing to say before a key");

    app.handle_key(key(KeyCode::Enter));

    assert_eq!(files_notice(&app), Some(FilesNotice::AlreadySaved));
    match ctrl_o(&mut app).expect("the notice points at Ctrl+O, which opens it") {
        InputAction::OpenFile(path) => {
            assert_eq!(path, PathBuf::from("/files/general/report.pdf"));
        }
        _ => panic!("expected InputAction::OpenFile"),
    }
}

#[test]
fn ctrl_o_on_a_file_that_is_not_saved_says_so_and_what_to_press_instead_works() {
    let mut app = app_viewing_report(false);

    assert!(ctrl_o(&mut app).is_none());

    assert_eq!(files_notice(&app), Some(FilesNotice::NotLocal));
    let (_, _, filename, ..) = save_fields(app.handle_key(key(KeyCode::Enter)));
    assert_eq!(filename, "report.pdf", "the notice points at Enter");
}

#[test]
fn a_notice_is_gone_at_the_next_key_whatever_it_is() {
    let keys = [
        key(KeyCode::Up),
        key(KeyCode::Down),
        key(KeyCode::Char('x')),
        key(KeyCode::Backspace),
        key(KeyCode::Left), // not a key the overlay uses
        key(KeyCode::Tab),  // nor this
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL), // nor a chord
    ];
    for pressed in keys {
        let mut app = app_viewing_report(true);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(files_notice(&app), Some(FilesNotice::AlreadySaved));

        app.handle_key(pressed);

        assert_eq!(files_notice(&app), None, "after {pressed:?}");
    }
}

#[test]
fn pressing_the_same_key_again_says_it_again() {
    let mut app = app_viewing_report(true);

    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(files_notice(&app), Some(FilesNotice::AlreadySaved));
}

#[test]
fn a_paste_drops_the_notice_too() {
    let mut app = app_viewing_report(true);
    app.handle_key(key(KeyCode::Enter));

    app.paste_text("rep");

    assert_eq!(files_notice(&app), None);
}

#[test]
fn a_notice_does_not_outlive_its_overlay() {
    let mut app = app_viewing_report(true);
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Esc));

    open_files(&mut app);

    assert_eq!(files_notice(&app), None);
}

#[test]
fn what_a_file_is_offered_follows_whether_it_is_saved_and_whether_opening_it_would_run_it() {
    let unsaved_program = app_viewing_file("setup.exe", None);
    assert_eq!(
        unsaved_program.file_offer(&unsaved_program.files_matches()[0]),
        FileOffer::Save,
        "saving a program is as safe as saving anything"
    );

    let saved_document = app_viewing_file("report.pdf", Some("/files/general/report.pdf"));
    assert_eq!(
        saved_document.file_offer(&saved_document.files_matches()[0]),
        FileOffer::Open(PathBuf::from("/files/general/report.pdf"))
    );

    let saved_program = app_viewing_file("setup.exe", Some("/files/general/setup.exe"));
    assert_eq!(
        saved_program.file_offer(&saved_program.files_matches()[0]),
        FileOffer::SavedOnly
    );
}

#[test]
fn ctrl_o_on_a_saved_program_is_refused_and_nothing_is_opened() {
    let mut app = app_viewing_file("setup.exe", Some("/files/general/setup.exe"));
    let before = app.active().messages.len();

    let action = ctrl_o(&mut app);

    assert!(action.is_none(), "nothing for the caller to open");
    assert!(app.viewing_files.is_some(), "still open");
    assert_eq!(files_notice(&app), Some(FilesNotice::SavedNotOpened));
    assert_eq!(
        app.active().messages.len(),
        before,
        "no `opening` line either"
    );
}

#[test]
fn enter_on_a_saved_program_says_it_is_saved_and_is_not_opened_from_here() {
    let mut app = app_viewing_file("setup.exe", Some("/files/general/setup.exe"));
    let before = app.active().messages.len();

    let action = app.handle_key(key(KeyCode::Enter));

    assert!(action.is_none(), "not saved a second time");
    assert_eq!(files_notice(&app), Some(FilesNotice::SavedNotOpened));
    assert_eq!(app.active().messages.len(), before);
}

#[test]
fn a_program_that_is_not_saved_yet_is_saved_like_any_file() {
    let mut app = app_viewing_file("setup.exe", None);

    let (_, _, filename, ..) = save_fields(app.handle_key(key(KeyCode::Enter)));

    assert_eq!(filename, "setup.exe");
}

#[test]
fn the_saved_copys_name_decides_not_the_one_the_file_was_announced_under() {
    // `files::sanitize_filename` keeps only the last component of a name,
    // so a peer's "setup.exe/" is saved as `setup.exe` -- which is what
    // the OS would run.
    let mut app = app_viewing_file("setup.exe/", Some("/files/general/setup.exe"));

    assert!(ctrl_o(&mut app).is_none());

    assert_eq!(files_notice(&app), Some(FilesNotice::SavedNotOpened));
}

#[test]
fn ctrl_o_opens_the_saved_copy_of_the_highlighted_file_and_closes_the_overlay() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    mark_saved(
        &mut app,
        "general",
        hash_of(b"report"),
        "/files/general/report (1).pdf",
    );

    let action = ctrl_o(&mut app);

    match action.expect("Ctrl+O on a saved file returns an action") {
        InputAction::OpenFile(path) => {
            assert_eq!(path, PathBuf::from("/files/general/report (1).pdf"));
        }
        _ => panic!("expected InputAction::OpenFile"),
    }
    assert!(app.viewing_files.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "opening report.pdf...");
}

#[test]
fn ctrl_o_on_a_file_that_is_not_saved_does_nothing() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    let before = app.active().messages.len();

    let action = ctrl_o(&mut app);

    assert!(action.is_none());
    assert!(app.viewing_files.is_some(), "still open");
    assert_eq!(app.active().messages.len(), before);
}

#[test]
fn ctrl_o_opens_whichever_file_is_highlighted() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "old.pdf", b"old"),
    );
    receive(
        &mut app,
        "general",
        file_message(2, PEER_ID, "new.pdf", b"new"),
    );
    open_files(&mut app);
    // Only the older one is saved.
    mark_saved(
        &mut app,
        "general",
        hash_of(b"old"),
        "/files/general/old.pdf",
    );

    assert!(
        ctrl_o(&mut app).is_none(),
        "new.pdf is highlighted, unsaved"
    );
    app.handle_key(key(KeyCode::Down));

    match ctrl_o(&mut app).expect("old.pdf is saved") {
        InputAction::OpenFile(path) => {
            assert_eq!(path, PathBuf::from("/files/general/old.pdf"));
        }
        _ => panic!("expected InputAction::OpenFile"),
    }
}

#[test]
fn what_the_disk_says_only_reaches_the_overlay_it_is_about() {
    let mut app = multi_channel_app(); // "random" is active
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "here.pdf", b"here"),
    );
    open_files(&mut app);

    // An answer about #general, which the overlay isn't on.
    mark_saved(
        &mut app,
        "general",
        hash_of(b"here"),
        "/files/general/here.pdf",
    );
    assert_eq!(app.saved_copy(hash_of(b"here")), None);

    mark_saved(
        &mut app,
        "random",
        hash_of(b"here"),
        "/files/random/here.pdf",
    );
    assert_eq!(
        app.saved_copy(hash_of(b"here")),
        Some(Path::new("/files/random/here.pdf"))
    );
}

#[test]
fn an_answer_after_the_overlay_closed_is_ignored_and_each_opening_starts_afresh() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    mark_saved(
        &mut app,
        "general",
        hash_of(b"report"),
        "/files/general/report.pdf",
    );
    app.handle_key(key(KeyCode::Esc));

    // A late answer, with no overlay to give it to.
    mark_saved(
        &mut app,
        "general",
        hash_of(b"report"),
        "/files/general/report.pdf",
    );
    assert!(app.viewing_files.is_none());

    // Nothing is known about the disk until the caller has looked again.
    open_files(&mut app);
    assert_eq!(app.saved_copy(hash_of(b"report")), None);
}

#[test]
fn a_save_that_finishes_while_the_overlay_is_open_marks_the_file_saved_at_once() {
    let mut app = app();
    receive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);
    assert_eq!(app.saved_copy(hash_of(b"report")), None);

    app.handle_net_event(NetEvent::FileSaved {
        channel: "general".to_string(),
        hash: hash_of(b"report"),
        filename: "report.pdf".to_string(),
        path: PathBuf::from("/files/general/report.pdf"),
        mode: SaveMode::Manual,
    });

    assert_eq!(
        app.saved_copy(hash_of(b"report")),
        Some(Path::new("/files/general/report.pdf"))
    );
}

#[test]
fn a_save_for_another_channel_does_not_mark_this_overlays_files() {
    let mut app = multi_channel_app(); // "random" is active
    receive(
        &mut app,
        "random",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );
    open_files(&mut app);

    app.handle_net_event(NetEvent::FileSaved {
        channel: "general".to_string(),
        hash: hash_of(b"report"),
        filename: "report.pdf".to_string(),
        path: PathBuf::from("/files/general/report.pdf"),
        mode: SaveMode::Manual,
    });

    assert_eq!(
        app.saved_copy(hash_of(b"report")),
        None,
        "a copy in #general's folder is not one in #random's"
    );
}

// The `/rename` tests below reuse the switcher fixture
// (`app_for_the_switcher`): `general` (active), a `/msg` DM shown as
// `Alice`, `project-x`, and a `/dm` chat shown as `Bob` -- channel
// indices 0 to 3.

#[test]
fn rename_gives_the_active_group_channel_a_local_name() {
    let mut app = app_for_the_switcher();
    app.active = 2; // project-x

    let action = submit(&mut app, "/rename proj");

    match action.expect("a successful /rename returns an action") {
        InputAction::Rename { channel, name } => {
            assert_eq!(channel, "project-x");
            assert_eq!(name, Some("proj".to_string()));
        }
        _ => panic!("expected InputAction::Rename"),
    }
    assert_eq!(app.channel_label("project-x"), "proj");
    assert_eq!(app.channel_heading("project-x"), "#proj");
    assert_eq!(
        app.channels[2].name, "project-x",
        "the stored name is the channel's identity and never changes"
    );
    assert_eq!(
        app.channel_label("general"),
        "general",
        "other channels are untouched"
    );
}

#[test]
fn a_renamed_channel_is_called_by_its_local_name_in_overlays_and_notices() {
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename proj");

    submit(&mut app, "/who");
    assert_eq!(app.viewing_peers.as_ref().unwrap().heading, "#proj");
    app.handle_key(key(KeyCode::Esc));

    app.remove_channel("project-x");
    let last = as_system(app.active().messages.back().unwrap());
    assert_eq!(last, "left #proj");
}

#[test]
fn renaming_again_replaces_the_previous_name() {
    let mut app = app_for_the_switcher();
    app.active = 2;

    submit(&mut app, "/rename proj");
    submit(&mut app, "/rename px");

    assert_eq!(app.channel_label("project-x"), "px");
}

#[test]
fn a_local_name_may_have_spaces() {
    let mut app = app_for_the_switcher();
    app.active = 2;

    submit(&mut app, "/rename  my project ");

    assert_eq!(app.channel_label("project-x"), "my project");
}

#[test]
fn a_bare_rename_restores_the_stored_name() {
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename proj");

    let action = submit(&mut app, "/rename");

    match action.expect("restoring a name returns an action to persist that") {
        InputAction::Rename { channel, name } => {
            assert_eq!(channel, "project-x");
            assert_eq!(name, None);
        }
        _ => panic!("expected InputAction::Rename"),
    }
    assert_eq!(app.channel_label("project-x"), "project-x");
}

#[test]
fn a_bare_rename_with_nothing_to_restore_shows_usage() {
    let mut app = app_for_the_switcher();
    app.active = 2;

    let action = submit(&mut app, "/rename");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("usage:"), "got: {last}");
}

#[test]
fn rename_is_refused_on_a_dm_with_a_pointer_to_alias() {
    let mut app = app_for_the_switcher();
    app.active = 1; // the /msg DM

    let action = submit(&mut app, "/rename buddy");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("/alias"), "got: {last}");
    assert_eq!(app.channel_label("dm-Alice"), "Alice", "label untouched");
    // Restoring is refused the same way: there is nothing here to restore.
    assert!(submit(&mut app, "/rename").is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("/alias"), "got: {last}");
}

#[test]
fn rename_is_refused_on_a_point_to_point_chat_too() {
    let mut app = app_for_the_switcher();
    app.active = 3; // the /dm chat

    let action = submit(&mut app, "/rename buddy");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("/alias"), "got: {last}");
    assert_eq!(app.channel_label("p2p-08080808"), "Bob", "label untouched");
}

#[test]
fn rename_refuses_a_name_another_group_channel_already_shows() {
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename work"); // project-x is now shown as `work`
    app.active = 0; // general

    let action = submit(&mut app, "/rename work");

    assert!(action.is_none());
    assert_eq!(app.channel_label("general"), "general");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.contains("another channel"), "got: {last}");
}

#[test]
fn rename_refuses_another_channels_stored_name_even_when_it_is_hidden() {
    // `/leave project-x` still matches that stored name first, so naming
    // another channel that would leave the two impossible to tell apart.
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename work"); // `project-x` no longer on screen
    app.active = 0; // general

    let action = submit(&mut app, "/rename project-x");

    assert!(action.is_none());
    assert_eq!(app.channel_label("general"), "general");
}

#[test]
fn a_channel_may_be_renamed_to_its_own_stored_name() {
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename proj");

    let action = submit(&mut app, "/rename project-x");

    assert!(action.is_some());
    assert_eq!(app.channel_label("project-x"), "project-x");
}

#[test]
fn a_group_channel_may_share_a_name_with_a_conversation() {
    let mut app = app_for_the_switcher(); // `Alice` is the DM's label

    let action = submit(&mut app, "/rename Alice"); // general is active

    assert!(action.is_some());
    assert_eq!(app.channel_heading("general"), "#Alice");
    assert_eq!(
        app.channel_heading("dm-Alice"),
        "@Alice",
        "the glyph tells the two apart"
    );
}

#[test]
fn leaving_a_renamed_channel_forgets_its_name_so_a_rejoin_starts_clean() {
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename proj");

    app.remove_channel("project-x");
    // Rejoined later under the very same stored name.
    app.handle_net_event(NetEvent::Joined("project-x".to_string()));

    assert_eq!(app.active().name, "project-x");
    assert_eq!(app.channel_label("project-x"), "project-x");
}

#[test]
fn load_renames_seeds_local_names() {
    let mut app = app_for_the_switcher();

    app.load_renames(HashMap::from([(
        "project-x".to_string(),
        "proj".to_string(),
    )]));

    assert_eq!(app.channel_label("project-x"), "proj");
    assert_eq!(app.channel_label("general"), "general");
}

#[test]
fn a_stored_rename_never_applies_to_a_conversation() {
    let mut app = app_for_the_switcher();

    app.load_renames(HashMap::from([
        ("dm-Alice".to_string(), "nope".to_string()),
        ("p2p-08080808".to_string(), "nope".to_string()),
    ]));

    assert_eq!(app.channel_label("dm-Alice"), "Alice");
    assert_eq!(app.channel_label("p2p-08080808"), "Bob");
}

#[test]
fn leave_and_the_switcher_find_a_renamed_channel_by_its_new_name() {
    let mut app = app_for_the_switcher();
    app.active = 2;
    submit(&mut app, "/rename proj");
    app.active = 0;

    // `/leave` takes the new name, its heading, and still the stored name.
    for typed in ["proj", "#proj", "project-x"] {
        match submit(&mut app, &format!("/leave {typed}")) {
            Some(InputAction::Leave(channel)) => {
                assert_eq!(channel, "project-x", "{typed}");
            }
            _ => panic!("expected InputAction::Leave for {typed}"),
        }
    }

    // The switcher matches what is shown, not the stored name.
    ctrl_p(&mut app);
    type_text(&mut app, "proj");
    assert_eq!(app.switcher_matches(), vec![2]);
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "-x");
    assert!(
        app.switcher_matches().is_empty(),
        "the stored name is not searched"
    );
}

// -- auto-save ------------------------------------------------------------
// `/autosave` and `auto_save`. Most of these start from
// `app_with_autosave`: `general` (active) with auto-save on and the files
// root at `/files`.

fn app_with_autosave() -> AppState {
    let mut app = app();
    app.load_files_root(PathBuf::from("/files"));
    app.load_autosave(HashSet::from(["general".to_string()]));
    app
}

/// `SaveMode` of every auto-save.
fn automatic() -> SaveMode {
    SaveMode::Automatic {
        max_bytes: DEFAULT_AUTOSAVE_LIMIT,
    }
}

/// What main.rs does with a message off the network: hands it to the
/// app and -- only if that made it a new line in the transcript -- asks
/// what auto-save makes of it, with nothing yet saved on disk.
fn arrive(app: &mut AppState, channel: &str, message: ChatMessage) -> Option<SaveRequest> {
    let (channel, message) =
        app.handle_net_event(NetEvent::Received(channel.to_string(), message))?;
    app.auto_save(&channel, &message, |_| false)
}

/// Gives `app` a user key of its own and returns it, so that devices can
/// be certified under it.
fn own_user_key(app: &mut AppState) -> SecretKey {
    let key = SecretKey::generate();
    app.load_self_user_id(*key.public().as_bytes());
    key
}

/// Certifies `device` under `key`, as its own announcement would.
fn certify(app: &mut AppState, key: &SecretKey, device: [u8; 32]) {
    app.handle_net_event(NetEvent::Device(
        "general".to_string(),
        device_cert(key, device),
    ));
}

/// The newest line in `channel`'s transcript, which must be a notice.
fn newest_notice<'a>(app: &'a AppState, channel: &str) -> &'a str {
    let channel = app
        .channels
        .iter()
        .find(|c| c.name == channel)
        .expect("a joined channel");
    as_system(channel.messages.back().expect("a line in the transcript"))
}

/// How many notices `channel`'s transcript holds.
fn notices_in(app: &AppState, channel: &str) -> usize {
    app.channels
        .iter()
        .find(|c| c.name == channel)
        .expect("a joined channel")
        .messages
        .iter()
        .filter(|line| matches!(line, TranscriptLine::System(_)))
        .count()
}

/// A file message from `sender` that announces `size` bytes, whatever
/// the file really holds.
fn file_message_of_size(id: u64, sender: [u8; 32], filename: &str, size: u64) -> ChatMessage {
    let mut message = file_message(id, sender, filename, filename.as_bytes());
    message.attachment.as_mut().unwrap().size = size;
    message
}

/// The first eight hex digits of the hash `file_message` gives `content`,
/// which is what a `/save` hint names.
fn hint_prefix(content: &[u8]) -> String {
    hash_of(content).to_hex()[..8].to_string()
}

fn alice() -> HashMap<[u8; 32], String> {
    HashMap::from([(PEER_ID, "Alice".to_string())])
}

#[test]
fn auto_save_is_off_until_it_is_turned_on_and_a_bare_command_says_so() {
    let mut app = app();
    app.load_files_root(PathBuf::from("/files"));

    let action = submit(&mut app, "/autosave");

    assert!(action.is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is off for #general"),
        "got: {last}"
    );
    assert!(last.contains("25.0 MB"), "the limit, got: {last}");
    assert!(last.contains("/files/general"), "the folder, got: {last}");
    assert!(
        last.contains("/autosave on"),
        "how to turn it on, got: {last}"
    );
}

#[test]
fn autosave_on_switches_it_on_for_the_active_channel_and_asks_to_remember_it() {
    let mut app = app();

    match submit(&mut app, "/autosave on") {
        Some(InputAction::SetAutoSave { channel, on }) => {
            assert_eq!(channel, "general");
            assert!(on);
        }
        _ => panic!("expected InputAction::SetAutoSave"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is on for #general"),
        "got: {last}"
    );
    assert!(last.contains("/autosave off"), "got: {last}");

    // A bare command agrees.
    assert!(submit(&mut app, "/autosave").is_none());
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is on for #general"),
        "got: {last}"
    );
}

#[test]
fn outbox_on_requests_runtime_setup_without_claiming_success_early() {
    let mut app = app();
    app.load_files_root(PathBuf::from("/files"));

    match submit(&mut app, "/outbox on") {
        Some(InputAction::SetOutbox { channel, on }) => {
            assert_eq!(channel, "general");
            assert!(on);
        }
        _ => panic!("expected InputAction::SetOutbox"),
    }
    assert!(
        !app.outbox.contains("general"),
        "the runtime must create and persist the folder before state says it is on"
    );
}

#[test]
fn outbox_status_is_per_channel_and_uses_the_send_subfolder() {
    let mut app = multi_channel_app();
    app.load_files_root(PathBuf::from("/files"));
    app.load_outbox(HashSet::from(["general".to_string()]));

    submit(&mut app, "/outbox"); // random is active
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("outbox is off for #random"), "got: {last}");
    assert!(last.contains("/files/random/send"), "got: {last}");

    app.active = 0;
    submit(&mut app, "/outbox");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("outbox is on for #general"), "got: {last}");
}

#[test]
fn a_bad_outbox_argument_is_usage_and_changes_nothing() {
    let mut app = app();

    assert!(submit(&mut app, "/outbox later").is_none());

    assert_eq!(
        as_system(app.active().messages.back().unwrap()),
        "usage: /outbox [on|off]"
    );
    assert!(app.outbox.is_empty());
}

#[test]
fn autosave_off_switches_it_off_again_and_asks_to_remember_that_too() {
    let mut app = app_with_autosave();

    match submit(&mut app, "/autosave off") {
        Some(InputAction::SetAutoSave { channel, on }) => {
            assert_eq!(channel, "general");
            assert!(!on);
        }
        _ => panic!("expected InputAction::SetAutoSave"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is off for #general"),
        "got: {last}"
    );
}

#[test]
fn asking_for_what_already_holds_remembers_nothing_but_still_says_it() {
    let mut app = app();

    assert!(submit(&mut app, "/autosave off").is_none(), "already off");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("auto-save is off"), "got: {last}");

    assert!(submit(&mut app, "/autosave on").is_some());
    assert!(submit(&mut app, "/autosave on").is_none(), "already on");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(last.starts_with("auto-save is on"), "got: {last}");
}

#[test]
fn auto_save_is_per_channel() {
    let mut app = multi_channel_app(); // "random" is active

    submit(&mut app, "/autosave on");
    app.active = 0; // general
    submit(&mut app, "/autosave");

    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is off for #general"),
        "got: {last}"
    );
}

#[test]
fn auto_save_can_be_switched_on_in_a_conversation_and_names_it_by_its_heading() {
    let mut app = app_for_the_switcher();
    app.active = 1; // the /msg DM, shown as Alice

    match submit(&mut app, "/autosave on") {
        Some(InputAction::SetAutoSave { channel, on }) => {
            assert_eq!(channel, "dm-Alice", "by stored name, like its folder");
            assert!(on);
        }
        _ => panic!("expected InputAction::SetAutoSave"),
    }
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is on for @Alice"),
        "got: {last}"
    );
}

#[test]
fn a_bad_autosave_argument_is_usage_and_changes_nothing() {
    let mut app = app();

    for typed in ["maybe", "on off", "ON"] {
        let action = submit(&mut app, &format!("/autosave {typed}"));

        assert!(action.is_none(), "{typed}");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(
            last, "usage: /autosave [on|off|limit <size|default>]",
            "{typed}"
        );
    }
    assert!(app.autosave.is_empty());
}

#[test]
fn load_autosave_seeds_the_channels_that_have_it_on() {
    let mut app = multi_channel_app();

    app.load_autosave(HashSet::from(["general".to_string()]));

    submit(&mut app, "/autosave"); // random is active
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is off for #random"),
        "got: {last}"
    );
    app.active = 0;
    submit(&mut app, "/autosave");
    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.starts_with("auto-save is on for #general"),
        "got: {last}"
    );
}

#[test]
fn help_lists_autosave_and_its_limit() {
    let mut app = app();

    submit(&mut app, "/help");

    let last = as_system(app.active().messages.back().unwrap());
    assert!(
        last.contains("/autosave [on|off|limit <size|default>]"),
        "got: {last}"
    );
    assert!(last.contains("device-wide"), "got: {last}");
    assert!(last.contains("channel overrides it"), "got: {last}");
}

#[test]
fn a_file_from_one_of_our_own_devices_is_saved() {
    let mut app = app_with_autosave();
    let key = own_user_key(&mut app);
    certify(&mut app, &key, PEER_ID);

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    )
    .expect("our own device is trusted");

    assert_eq!(
        request,
        SaveRequest {
            channel: "general".to_string(),
            hash: hash_of(b"report"),
            filename: "report.pdf".to_string(),
            providers: vec![PEER_ID],
            dir: PathBuf::from("/files/general"),
            mode: automatic(),
        }
    );
    assert_eq!(
        notices_in(&app, "general"),
        0,
        "nothing to say until it has landed"
    );
}

#[test]
fn a_file_from_a_peer_we_have_aliased_is_saved() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );

    assert_eq!(request.expect("Alice is aliased").providers, vec![PEER_ID]);
}

#[test]
fn an_alias_covers_every_device_of_that_person() {
    let mut app = app_with_autosave();
    let their_key = SecretKey::generate();
    for device in [PEER_ID, PEER_ID_2] {
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&their_key, device),
        ));
    }
    // Given to one of their devices; kept under who they are.
    app.load_contacts(HashMap::from([(
        app.canonical_id(&PEER_ID),
        "Jurgen".to_string(),
    )]));

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID_2, "report.pdf", b"report"),
    );

    assert!(request.is_some());
}

#[test]
fn a_file_from_a_stranger_is_skipped_and_says_why() {
    let mut app = app_with_autosave();

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );

    assert!(request.is_none());
    let notice = newest_notice(&app, "general");
    assert!(
        notice.starts_with("not auto-saving report.pdf from "),
        "got: {notice}"
    );
    assert!(notice.contains(&hex_prefix(&PEER_ID)), "who, got: {notice}");
    assert!(
        notice.contains("your own devices and the peers you have aliased"),
        "why, got: {notice}"
    );
    assert!(
        notice.contains(&format!(
            "/save {} to download it yourself",
            hint_prefix(b"report")
        )),
        "the way out, got: {notice}"
    );
}

#[test]
fn a_device_we_forgot_is_no_longer_one_of_our_own() {
    let mut app = app_with_autosave();
    let key = own_user_key(&mut app);
    certify(&mut app, &key, PEER_ID);
    app.load_forgotten_devices(HashSet::from([PEER_ID]));

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );

    assert!(request.is_none());
    assert!(newest_notice(&app, "general").starts_with("not auto-saving report.pdf"));
}

#[test]
fn a_file_from_ourselves_is_never_auto_saved_and_nothing_is_said() {
    let mut app = app_with_autosave();

    let request = arrive(
        &mut app,
        "general",
        file_message(1, SELF_ID, "report.pdf", b"report"),
    );

    assert!(request.is_none());
    assert_eq!(notices_in(&app, "general"), 0);
}

#[test]
fn a_message_without_a_file_is_none_of_auto_saves_business() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());

    let request = arrive(&mut app, "general", compose_message(PEER_ID, "hello", None));

    assert!(request.is_none());
    assert_eq!(notices_in(&app, "general"), 0);
}

#[test]
fn nothing_is_saved_or_said_while_auto_save_is_off() {
    let mut app = app();
    app.load_contacts(alice());

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );

    assert!(request.is_none());
    assert_eq!(notices_in(&app, "general"), 0);
}

#[test]
fn a_file_over_the_limit_is_skipped_with_its_size_and_the_way_to_fetch_it() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());
    let message = file_message_of_size(1, PEER_ID, "video.mov", 80 * 1024 * 1024);
    let prefix = message.attachment.as_ref().unwrap().hash.to_hex()[..8].to_string();

    let request = arrive(&mut app, "general", message);

    assert!(request.is_none());
    let notice = newest_notice(&app, "general");
    assert!(notice.contains("video.mov (80.0 MB)"), "got: {notice}");
    assert!(
        notice.contains("over the 25.0 MB auto-save limit"),
        "got: {notice}"
    );
    assert!(
        notice.contains(&format!("/save {prefix} to download it yourself")),
        "got: {notice}"
    );
}

#[test]
fn a_file_exactly_at_the_limit_is_saved_and_one_byte_more_is_not() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());

    let at = file_message_of_size(1, PEER_ID, "at.bin", DEFAULT_AUTOSAVE_LIMIT);
    let over = file_message_of_size(2, PEER_ID, "over.bin", DEFAULT_AUTOSAVE_LIMIT + 1);

    assert!(arrive(&mut app, "general", at).is_some());
    assert!(arrive(&mut app, "general", over).is_none());
}

#[test]
fn a_file_the_channel_already_holds_is_skipped_and_says_so() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());
    let (channel, message) = app
        .handle_net_event(NetEvent::Received(
            "general".to_string(),
            file_message(1, PEER_ID, "report.pdf", b"report"),
        ))
        .unwrap();

    let request = app.auto_save(&channel, &message, |hash| hash == hash_of(b"report"));

    assert!(request.is_none());
    let notice = newest_notice(&app, "general");
    assert!(
        notice.contains("not auto-saving report.pdf")
            && notice.contains("already has a saved copy"),
        "got: {notice}"
    );
}

#[test]
fn a_repeat_of_a_save_in_flight_is_dropped_until_it_finishes() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());
    assert!(
        arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report")
        )
        .is_some()
    );

    // The same file again -- sent twice, or a share replayed -- while the
    // first is still downloading: one download is enough.
    let again = arrive(
        &mut app,
        "general",
        file_message(2, PEER_ID, "report.pdf", b"report"),
    );
    assert!(again.is_none());
    assert_eq!(
        notices_in(&app, "general"),
        0,
        "dropped quietly: the first one's notice is on its way"
    );

    // Once it has landed it is a new matter (main.rs asks the disk first).
    app.handle_net_event(NetEvent::FileSaved {
        channel: "general".to_string(),
        hash: hash_of(b"report"),
        filename: "report.pdf".to_string(),
        path: PathBuf::from("/files/general/report.pdf"),
        mode: automatic(),
    });
    let later = arrive(
        &mut app,
        "general",
        file_message(3, PEER_ID, "report.pdf", b"report"),
    );
    assert!(later.is_some());
}

#[test]
fn a_save_that_failed_can_be_tried_again_by_the_next_share() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());
    assert!(
        arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report")
        )
        .is_some()
    );

    app.save_failed(
        "general",
        hash_of(b"report"),
        "report.pdf",
        "boom",
        automatic(),
    );

    let next = arrive(
        &mut app,
        "general",
        file_message(2, PEER_ID, "report.pdf", b"report"),
    );
    assert!(next.is_some());
}

#[test]
fn a_redelivered_message_is_not_auto_saved_a_second_time() {
    let mut app = app_with_autosave();
    app.load_contacts(alice());
    let message = file_message(1, PEER_ID, "report.pdf", b"report");
    assert!(arrive(&mut app, "general", message.clone()).is_some());
    app.handle_net_event(NetEvent::FileSaved {
        channel: "general".to_string(),
        hash: hash_of(b"report"),
        filename: "report.pdf".to_string(),
        path: PathBuf::from("/files/general/report.pdf"),
        mode: automatic(),
    });

    // The very same message again: not a new line, so never asked about.
    assert!(arrive(&mut app, "general", message).is_none());
}

#[test]
fn a_file_is_only_ever_asked_of_peers_we_trust() {
    let mut app = app_with_autosave();
    app.load_contacts(HashMap::from([
        (PEER_ID, "Alice".to_string()),
        (PEER_ID_3, "Carol".to_string()),
    ]));
    // A stranger who is online and has posted, listed ahead of Carol.
    receive(
        &mut app,
        "general",
        compose_message(PEER_ID_2, "hello", None),
    );
    peers_join(&mut app, "general", &[PEER_ID_2, PEER_ID_3]);

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    )
    .unwrap();

    assert_eq!(request.providers, vec![PEER_ID, PEER_ID_3]);
}

#[test]
fn our_other_devices_that_are_online_are_asked_too() {
    let mut app = app_with_autosave();
    let key = own_user_key(&mut app);
    certify(&mut app, &key, PEER_ID);
    certify(&mut app, &key, PEER_ID_2);
    peers_join(&mut app, "general", &[PEER_ID_2]);

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    )
    .unwrap();

    assert_eq!(request.providers, vec![PEER_ID, PEER_ID_2]);
}

#[test]
fn a_share_forged_as_a_trusted_peer_still_cannot_reach_a_stranger() {
    // Nothing signs a chat message, so a stranger in the channel can claim
    // to be Alice. What it can't do is be asked for the file: the only
    // peers asked are the ones we trust, known by their connection.
    let mut app = app_with_autosave();
    app.load_contacts(alice());
    peers_join(&mut app, "general", &[PEER_ID_2]); // the stranger, online
    let forged = file_message(1, PEER_ID, "invoice.pdf", b"the stranger's own bytes");

    let request = arrive(&mut app, "general", forged).expect("claims to be Alice: tried");

    assert_eq!(
        request.providers,
        vec![PEER_ID],
        "only the real Alice is asked, and she doesn't have it"
    );
}

#[test]
fn a_file_goes_into_the_folder_of_the_channel_it_arrived_in_whichever_is_active() {
    let mut app = multi_channel_app(); // "random" is active
    app.load_files_root(PathBuf::from("/files"));
    app.load_autosave(HashSet::from(["general".to_string()]));
    app.load_contacts(alice());

    let request = arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    )
    .unwrap();

    assert_eq!(request.channel, "general");
    assert_eq!(request.dir, PathBuf::from("/files/general"));
}

#[test]
fn what_auto_save_says_lands_in_the_channel_the_file_arrived_in() {
    let mut app = multi_channel_app(); // "random" is active
    app.load_autosave(HashSet::from(["general".to_string()]));

    arrive(
        &mut app,
        "general",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );

    assert!(newest_notice(&app, "general").starts_with("not auto-saving report.pdf"));
    assert_eq!(
        notices_in(&app, "random"),
        0,
        "the channel being looked at hears nothing"
    );
}

#[test]
fn an_automatic_save_is_reported_in_its_own_channel_and_a_manual_one_where_you_are() {
    let mut app = multi_channel_app(); // "random" is active
    let saved = |mode| NetEvent::FileSaved {
        channel: "general".to_string(),
        hash: hash_of(b"report"),
        filename: "report.pdf".to_string(),
        path: PathBuf::from("/files/general/report.pdf"),
        mode,
    };

    app.handle_net_event(saved(automatic()));
    assert_eq!(
        newest_notice(&app, "general"),
        "auto-saved report.pdf to /files/general/report.pdf"
    );
    assert_eq!(notices_in(&app, "random"), 0);

    app.handle_net_event(saved(SaveMode::Manual));
    assert_eq!(
        newest_notice(&app, "random"),
        "saved report.pdf to /files/general/report.pdf"
    );
}

#[test]
fn a_failed_save_says_so_in_the_words_of_its_mode() {
    let mut app = multi_channel_app(); // "random" is active
    let hash = hash_of(b"report");

    app.save_failed("general", hash, "report.pdf", "boom", SaveMode::Manual);
    assert_eq!(
        newest_notice(&app, "random"),
        "failed to save report.pdf: boom"
    );

    app.save_failed("general", hash, "report.pdf", "boom", automatic());
    assert_eq!(
        newest_notice(&app, "general"),
        format!(
            "couldn't auto-save report.pdf: boom -- /save {} to download it yourself",
            hint_prefix(b"report")
        )
    );
    assert_eq!(notices_in(&app, "random"), 1, "only the manual one");
}

#[test]
fn leaving_a_channel_forgets_its_auto_save_so_a_later_room_of_that_name_starts_off() {
    let mut app = app_for_the_switcher(); // Alice (PEER_ID) is aliased
    app.load_autosave(HashSet::from(["project-x".to_string()]));

    app.remove_channel("project-x");
    // A different room, rejoined under the very same stored name.
    app.handle_net_event(NetEvent::Joined("project-x".to_string()));
    assert_eq!(app.active().name, "project-x");

    let request = arrive(
        &mut app,
        "project-x",
        file_message(1, PEER_ID, "report.pdf", b"report"),
    );

    assert!(
        request.is_none(),
        "it must not inherit the old room's setting"
    );
    assert_eq!(notices_in(&app, "project-x"), 0);
    submit(&mut app, "/autosave");
    assert!(newest_notice(&app, "project-x").starts_with("auto-save is off"));
}

#[test]
fn leaving_a_channel_drops_its_saves_in_flight() {
    let mut app = app_for_the_switcher(); // Alice (PEER_ID) is aliased
    app.load_autosave(HashSet::from(["project-x".to_string()]));
    let file = || file_message(1, PEER_ID, "report.pdf", b"report");
    assert!(arrive(&mut app, "project-x", file()).is_some());

    app.remove_channel("project-x");
    app.handle_net_event(NetEvent::Joined("project-x".to_string()));
    submit(&mut app, "/autosave on");

    let request = arrive(
        &mut app,
        "project-x",
        file_message(2, PEER_ID, "report.pdf", b"report"),
    );
    assert!(request.is_some(), "no stale entry may block it");
}

#[test]
fn channel_auto_save_limit_overrides_device_limit_and_is_forgotten_on_leave() {
    let mut app = app_for_the_switcher();
    app.load_autosave_limit(AutoSaveLimit::Bytes(1024));
    app.load_autosave_limits(HashMap::from([(
        "project-x".into(),
        AutoSaveLimit::Bytes(2048),
    )]));
    app.load_autosave(HashSet::from(["project-x".to_string()]));
    app.load_contacts(alice());

    assert!(
        arrive(
            &mut app,
            "project-x",
            file_message_of_size(1, PEER_ID, "large.bin", 1500)
        )
        .is_some()
    );
    app.remove_channel("project-x");
    assert!(!app.autosave_limits.contains_key("project-x"));
}

#[test]
fn sync_works_on_a_msg_dm_channel() {
    let mut app = app_with_dm("dm-Alice", PEER_ID);
    app.active = 1;

    let action = submit(&mut app, "/sync publish-and-receive").unwrap();

    let InputAction::SetSync { channel, config } = action else {
        panic!("expected sync persistence action");
    };
    assert_eq!(channel, "dm-Alice");
    assert_eq!(config.policy, SyncPolicy::PublishAndReceive);
}

#[test]
fn sync_works_on_a_point_to_point_channel() {
    let mut app = app();
    app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);

    let action = submit(&mut app, "/sync on").unwrap();

    let InputAction::SetSync { channel, config } = action else {
        panic!("expected sync persistence action");
    };
    assert_eq!(channel, "p2p-alice");
    assert_eq!(config.policy, SyncPolicy::PublishAndReceive);
}

#[test]
fn a_bare_sync_explains_setup_before_anything_is_configured() {
    let mut app = app();

    submit(&mut app, "/sync");

    let notice = as_system(app.active().messages.back().unwrap());
    assert!(notice.contains("/invite"), "got: {notice}");
    assert!(notice.contains("/join"), "got: {notice}");
    assert!(
        !notice.contains("pending"),
        "an unconfigured channel should not look like an active status report: {notice}"
    );
}

#[test]
fn a_bare_sync_explains_setup_differently_for_a_point_to_point_chat() {
    // Already has its one peer, so there is no invited group to join.
    let mut app = app();
    app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);

    submit(&mut app, "/sync");

    let notice = as_system(app.active().messages.back().unwrap());
    assert!(
        !notice.contains("/invite"),
        "a point-to-point chat has no invite step: {notice}"
    );
}

#[test]
fn a_bare_sync_reports_status_once_sync_has_been_configured() {
    let mut app = app();
    submit(&mut app, "/sync receive-only");

    submit(&mut app, "/sync");

    let notice = as_system(app.active().messages.back().unwrap());
    assert!(notice.contains("pending"), "got: {notice}");
    assert!(!notice.contains("/invite"), "got: {notice}");
}

#[test]
fn a_sync_report_with_conflicts_points_to_how_to_resolve_them() {
    let mut app = app();
    submit(&mut app, "/sync on");
    app.sync_conflict(
        "general",
        SyncConflict {
            local_path: "report.txt".into(),
            remote_path: "report.txt".into(),
            remote_copy_path: Some("report (remote conflict 1).txt".into()),
            local_modified_unix_ms: None,
            remote_modified_unix_ms: None,
            author: None,
        },
    );

    submit(&mut app, "/sync");

    let notice = as_system(app.active().messages.back().unwrap());
    assert!(
        notice.contains("/conflicts local|remote|both"),
        "got: {notice}"
    );
}

#[test]
fn a_sync_report_with_pending_files_points_to_who() {
    let mut app = app();
    submit(&mut app, "/sync on");
    app.sync_pending("general", "report.txt".to_string());

    submit(&mut app, "/sync");

    let notice = as_system(app.active().messages.back().unwrap());
    assert!(notice.contains("/who"), "got: {notice}");
}

#[test]
fn sync_policies_are_independent_per_channel() {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), "project".to_string()],
        "general",
    );

    let action = submit(&mut app, "/sync receive-only").unwrap();
    let InputAction::SetSync { channel, config } = action else {
        panic!("expected sync persistence action");
    };
    assert_eq!(channel, "general");
    assert_eq!(config.policy, SyncPolicy::ReceiveOnly);
    assert!(app.sync_receives("general"));

    app.active = 1;
    let action = submit(&mut app, "/sync publish-and-receive").unwrap();
    let InputAction::SetSync { channel, config } = action else {
        panic!("expected sync persistence action");
    };
    assert_eq!(channel, "project");
    assert_eq!(config.policy, SyncPolicy::PublishAndReceive);
    assert_eq!(app.sync_config("general").policy, SyncPolicy::ReceiveOnly);
}

#[test]
fn sync_rules_are_persisted_actions_and_visible_in_status() {
    let mut app = app();

    let action = submit(&mut app, "/sync include docs/**").unwrap();
    let InputAction::SetSync { config, .. } = action else {
        panic!("expected sync persistence action");
    };
    assert_eq!(config.rules, vec![SyncRule::include("docs/**").unwrap()]);
    assert!(app.sync_allows_path("general", "docs/plan.md"));
    assert!(!app.sync_allows_path("general", "images/logo.png"));

    submit(&mut app, "/sync exclude docs/private/**");
    submit(&mut app, "/sync");
    let status = as_system(app.active().messages.back().unwrap());
    assert!(status.contains("include docs/**"), "{status}");
    assert!(status.contains("exclude docs/private/**"), "{status}");
    assert!(!app.sync_allows_path("general", "docs/private/plan.md"));
}

#[test]
fn sync_pause_resumes_the_selected_policy() {
    let mut app = app();
    submit(&mut app, "/sync receive-only");
    submit(&mut app, "/sync pause");
    assert!(app.sync_paused("general"));
    assert!(!app.sync_receives("general"));

    let action = submit(&mut app, "/sync resume").unwrap();
    let InputAction::SetSync { config, .. } = action else {
        panic!("expected sync persistence action");
    };
    assert_eq!(config.policy, SyncPolicy::ReceiveOnly);
    assert!(!config.paused);
}

#[test]
fn sync_rejects_an_invalid_rule_without_changing_configuration() {
    let mut app = app();

    let action = submit(&mut app, "/sync include ../outside");

    assert!(action.is_none());
    assert!(app.sync_config("general").rules.is_empty());
    assert!(as_system(app.active().messages.back().unwrap()).contains("relative glob"));
}

#[test]
fn trash_commands_target_only_the_active_channel() {
    let mut app = AppState::new(
        SELF_ID,
        vec!["general".to_string(), "project".to_string()],
        "general",
    );
    app.active = 1;

    assert!(matches!(
        submit(&mut app, "/trash"),
        Some(InputAction::ListSyncTrash(channel)) if channel == "project"
    ));
    assert!(matches!(
        submit(&mut app, "/trash restore old/report.txt"),
        Some(InputAction::RestoreSyncTrash { channel, path })
            if channel == "project" && path == "old/report.txt"
    ));
    assert!(matches!(
        submit(&mut app, "/trash purge"),
        Some(InputAction::PurgeSyncTrash(channel)) if channel == "project"
    ));
}

#[test]
fn trash_rejects_missing_restore_path_and_unknown_arguments() {
    let mut app = app();

    assert!(submit(&mut app, "/trash restore").is_none());
    assert!(as_system(app.active().messages.back().unwrap()).starts_with("usage:"));
    assert!(submit(&mut app, "/trash tomorrow").is_none());
    assert!(as_system(app.active().messages.back().unwrap()).starts_with("usage:"));
}

#[test]
fn conflicts_list_provenance_and_clear_only_after_resolution() {
    let mut app = app();
    submit(&mut app, "/sync on");
    let conflict = SyncConflict {
        local_path: "report.txt".into(),
        remote_path: "report.txt".into(),
        remote_copy_path: Some("report (remote conflict 1).txt".into()),
        local_modified_unix_ms: Some(10),
        remote_modified_unix_ms: Some(20),
        author: Some(PEER_ID),
    };
    app.sync_conflict("general", conflict.clone());
    assert_eq!(app.sync_marker("general"), Some('C'));

    submit(&mut app, "/conflicts");
    let report = as_system(app.active().messages.back().unwrap());
    assert!(report.contains("/conflicts local|remote|both"));
    assert!(
        app.sync_status("general")
            .is_some_and(|status| status.conflict_count() == 1)
    );

    assert!(matches!(
        submit(&mut app, "/conflicts remote report.txt"),
        Some(InputAction::ResolveSyncConflict {
            resolution: ConflictResolution::Remote,
            ..
        })
    ));
    assert_eq!(app.sync_marker("general"), Some('C'));
    app.sync_conflict_resolved("general", &conflict);
    assert_eq!(app.sync_marker("general"), Some('S'));
}
