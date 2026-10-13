//! The fork's client-side `[keys]` actions come from the local config under
//! a host's key profile, and win a key that profile also binds.

use super::*;

#[test]
fn client_only_actions_come_from_local_config_under_an_endpoint_profile() {
    let local: Config = toml::from_str(
        r#"
[keys]
prefix = "ctrl+a"
agent_picker = "prefix+a"
history_back = "alt+left"
"#,
    )
    .unwrap();
    let endpoint: Config = toml::from_str(
        r#"
[keys]
prefix = "ctrl+x"
new_tab = "prefix+n"
"#,
    )
    .unwrap();
    let mut state = ClientShellState::new(
        ClientShellConfig::from_config(&local)
            .with_keybinding_source(ClientShellKeybindingSource::Endpoint),
    );
    let mut projection = snapshot();
    projection.server_keybindings_toml = endpoint.local_keybindings_profile_toml().ok();
    state.set_snapshot(Box::new(projection));

    let keybinds = &state.config.keybinds.keybinds;
    assert_eq!(state.config.keybinds.prefix[0].0, KeyCode::Char('x'));
    assert_eq!(keybinds.new_tab.label().as_deref(), Some("prefix+n"));
    assert_eq!(keybinds.agent_picker.label().as_deref(), Some("prefix+a"));
    assert_eq!(keybinds.history_back.label().as_deref(), Some("alt+left"));
    assert!(keybinds.mark_unread.label().is_none());
}

#[test]
fn a_local_client_action_wins_a_key_the_endpoint_profile_also_binds() {
    use crate::input::{KeybindAction, KeybindMatch, TerminalKey};
    use crossterm::event::KeyModifiers;

    let local: Config = toml::from_str(
        r#"
[keys]
agent_picker = "prefix+a"
history_back = "alt+left"
"#,
    )
    .unwrap();
    let endpoint: Config = toml::from_str(
        r#"
[keys]
prefix = "ctrl+x"
new_tab = "prefix+a"
previous_tab = "alt+left"
"#,
    )
    .unwrap();
    let mut state = ClientShellState::new(
        ClientShellConfig::from_config(&local)
            .with_keybinding_source(ClientShellKeybindingSource::Endpoint),
    );
    let mut projection = snapshot();
    projection.server_keybindings_toml = endpoint.local_keybindings_profile_toml().ok();
    state.set_snapshot(Box::new(projection));

    let keybinds = &state.config.keybinds.keybinds;
    assert_eq!(keybinds.new_tab.label().as_deref(), Some("prefix+a"));
    let prefixed = crate::input::resolve_prefix_binding(
        keybinds,
        &TerminalKey::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert!(
        matches!(
            prefixed,
            Some(KeybindMatch::Action(KeybindAction::AgentPicker))
        ),
        "{prefixed:?}"
    );
    let direct = crate::input::resolve_direct_binding(
        keybinds,
        &TerminalKey::new(KeyCode::Left, KeyModifiers::ALT),
    );
    assert!(
        matches!(
            direct,
            Some(KeybindMatch::Action(KeybindAction::HistoryBack))
        ),
        "{direct:?}"
    );
}
